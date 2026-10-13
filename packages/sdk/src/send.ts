/**
 * `sendSolanaTransactions`: sign every transaction of one action with ONE wallet prompt, then send them in order.
 *
 * An action is one or more transactions ("steps"), e.g. `planCreatePool`'s `[initializePool, addLiquidity]`. They are
 * signed together — one approval for the whole action — then sent one at a time, each confirmed before the next is
 * sent, because a later step reads what an earlier one wrote. The first failure stops the run and names the step;
 * nothing after it is sent.
 *
 * How the wallet is asked, in order of preference:
 * - `standard`: the Wallet Standard `solana:signTransaction` feature, called once with every transaction (one prompt);
 * - `signAllTransactions`: the legacy batch call (one prompt);
 * - `signTransaction`: one call, and one prompt, PER STEP — the fallback for a wallet that cannot batch.
 *
 * Every transaction the wallet returns is checked before anything is sent: its message bytes must equal the planned
 * message byte for byte, and every required signature — the expected signer's included — must be present and valid
 * over those bytes. A wallet that alters a transaction (adds an instruction, changes the fee payer) fails the run with
 * reason `sign`.
 *
 * A step is never sent twice. Each step is tracked by its fee-payer signature:
 * - a `sendTransaction` error that is not a preflight refusal (a timeout, a dropped connection) may still have
 *   delivered the transaction, so the step keeps waiting on its signature instead of failing;
 * - a preflight refusal (the node simulated the transaction and did not forward it) is re-checked by signature once,
 *   then reported as `send`;
 * - "Blockhash not found" alone never means expired. A step is expired only when its signature is absent, the
 *   cluster's block height has passed the blockhash's `lastValidBlockHeight`, at least two more blocks have been
 *   produced, a re-check with `searchTransactionHistory` still finds nothing, and the node reports the blockhash
 *   invalid. Only then is `opts.replan` asked for fresh transactions for the steps that have not landed (one more
 *   prompt). A step whose signature was ever seen (processed or confirmed) is never re-planned.
 * `lastValidBlockHeight` comes from `opts` (from `getLatestBlockhash`); without it the sender asks the node: the
 * latest blockhash's own figure when it is the step's blockhash, else an upper bound.
 *
 * `opts.beforeSend` runs before each step is sent (after the previous one confirmed); throwing from it stops the run
 * with reason `guard` and sends nothing more (`planCreatePool` uses it to check the new pool's price).
 *
 * A wallet that simulates each transaction before it signs cannot simulate a step that depends on an account an
 * earlier step creates (a lookup table, a pool): it shows that step as failing even though it lands once the earlier
 * step has.
 */

import {
  getBase58Decoder,
  getBase64Decoder,
  getBase64Encoder,
  getCompiledTransactionMessageDecoder,
  getPublicKeyFromAddress,
  getTransactionDecoder,
  verifySignature,
  type Address,
  type Base64EncodedWireTransaction,
  type Blockhash,
  type GetBlockHeightApi,
  type GetLatestBlockhashApi,
  type GetSignatureStatusesApi,
  type IsBlockhashValidApi,
  type Rpc,
  type SendTransactionApi,
  type Signature,
  type SignatureBytes,
} from '@solana/kit';

/** One account as the Wallet Standard passes it; only `address` is read here, the wallet reads the rest. */
export interface WalletStandardAccount {
  readonly address: string;
  readonly [key: string]: unknown;
}

/** The Wallet Standard `solana:signTransaction` feature, bound to the account that signs. */
export interface WalletStandardSigner {
  readonly account: WalletStandardAccount;
  /** The Wallet Standard chain id, e.g. `solana:devnet`. */
  readonly chain?: string;
  signTransaction(
    ...inputs: readonly { account: WalletStandardAccount; transaction: Uint8Array; chain?: string }[]
  ): Promise<readonly { signedTransaction: Uint8Array }[]>;
}

/** A wallet that signs serialized (wire-format) transactions. Give whichever calls the wallet has. */
export interface SolanaWallet {
  /** The Wallet Standard feature: every step in one call (one prompt). */
  readonly standard?: WalletStandardSigner;
  /** A legacy adapter's batch call: every step in one call (one prompt). */
  signAllTransactions?(transactions: readonly Uint8Array[]): Promise<readonly Uint8Array[]>;
  /** One transaction per call (one prompt each). */
  signTransaction?(transaction: Uint8Array): Promise<Uint8Array>;
}

/** Which call signed the action: one prompt for `standard` and `signAll`, one per step for `oneByOne`. */
export type SignPath = 'standard' | 'signAll' | 'oneByOne';

export type SendRpc = Rpc<SendTransactionApi & GetSignatureStatusesApi & GetBlockHeightApi & GetLatestBlockhashApi & IsBlockhashValidApi>;

/** The steps of one action: base64 wire transactions in send order, or any object carrying them (an `UnsignedTx`). */
export type SendSteps = readonly string[] | { readonly transactions: readonly string[] };

export interface ReplanRequest {
  /** Index (into the ORIGINAL steps) of the first step that has not landed. */
  readonly fromIndex: number;
  /** Signatures of the steps that landed, in order. */
  readonly landed: readonly Signature[];
}

export interface BeforeSendRequest {
  /** Index (into the ORIGINAL steps) of the step about to be sent. */
  readonly index: number;
  /** Its name (`SendOptions.steps`) or `#<index>`. */
  readonly step: string;
  /** Signatures of the steps that landed before it, in order. */
  readonly landed: readonly Signature[];
}

export interface SendOptions {
  /** A name per step, in order (e.g. `['initializePool', 'addLiquidity']`), used in errors. */
  readonly steps?: readonly string[];
  /** The confirmation each step waits for before the next is sent. Default `confirmed`. */
  readonly commitment?: 'confirmed' | 'finalized';
  /** Give up on one step's confirmation after this many milliseconds (reason `timeout`, which may still land). Default 90,000. */
  readonly timeoutMs?: number;
  /** Between status polls. Default 500 ms. */
  readonly pollMs?: number;
  /** The blockhash's last valid block height (from `getLatestBlockhash`). Omitted, the sender asks the node. */
  readonly lastValidBlockHeight?: bigint;
  /**
   * The address that must have signed every step. Default: the Wallet Standard account's address when given, else
   * each transaction's fee payer. Every other required signature is checked too.
   */
  readonly signer?: Address;
  /**
   * Fresh transactions for steps `fromIndex..` after the blockhash expired: one per remaining step, in order, with a new
   * blockhash (and its `lastValidBlockHeight`). They are signed with one more prompt.
   */
  readonly replan?: (req: ReplanRequest) => Promise<{ transactions: readonly string[]; lastValidBlockHeight?: bigint }>;
  /** How many times `replan` may be called. Default 1. */
  readonly maxReplans?: number;
  /** A check run before each step is sent; throwing stops the run with reason `guard`. */
  readonly beforeSend?: (req: BeforeSendRequest) => Promise<void> | void;
}

export interface SendResult {
  /** Every step's signature, in order. */
  readonly signatures: Signature[];
  /** How the wallet was asked (the first signing call; a replan uses the same one). */
  readonly path: SignPath;
  /** Signing calls that prompted the user: 1 for a batch path with no replan. */
  readonly prompts: number;
  /** How many times `replan` was called. */
  readonly replans: number;
}

/**
 * Why a run stopped:
 * - `sign`: the wallet refused, returned the wrong count, or returned a transaction that is not the planned one
 *   (different message bytes, a missing or invalid signature);
 * - `send`: the node refused the transaction at preflight and it was not found by signature;
 * - `failed`: the transaction landed with an error;
 * - `timeout`: not confirmed in time — it MAY still land; check `signature` before acting again;
 * - `expired`: proven not landed and its blockhash expired (no `replan`, or `maxReplans` used up);
 * - `guard`: `opts.beforeSend` refused the step.
 */
export type SendFailure = 'sign' | 'send' | 'failed' | 'timeout' | 'expired' | 'guard';

export class SolanaSendError extends Error {
  constructor(
    /** 0-based index of the step that failed. */
    readonly index: number,
    /** Its name (`SendOptions.steps`) or `#<index>`. */
    readonly step: string,
    /** Its signature, when it was signed. */
    readonly signature: Signature | null,
    /** Signatures of the steps that LANDED before it. */
    readonly landed: readonly Signature[],
    readonly reason: SendFailure,
    readonly cause?: unknown,
  ) {
    const what = {
      sign: 'was not signed as planned',
      send: 'could not be sent',
      failed: 'failed on chain',
      timeout: 'was not confirmed in time (it may still land)',
      expired: 'expired with its blockhash',
      guard: 'was stopped before sending',
    }[reason];
    const why = cause ? `: ${cause instanceof Error ? cause.message : JSON.stringify(cause, (_, v) => (typeof v === 'bigint' ? v.toString() : v))}` : '';
    super(`step ${step} (${index + 1}) ${what}${why}`);
    this.name = 'SolanaSendError';
  }
}

/** The fee payer's signature: the first 64 bytes after the compact-u16 signature count. */
export function firstSignature(wire: Uint8Array): Signature {
  let at = 0;
  for (let shift = 0; ; shift += 7) {
    const b = wire[at++] ?? 0;
    if ((b & 0x80) === 0 || shift >= 14) break;
  }
  return getBase58Decoder().decode(wire.subarray(at, at + 64)) as Signature;
}

/**
 * Whether an RPC error's text names an unknown or expired blockhash. Informational only: a node that has not seen a
 * blockhash yet answers the same way, so the sender never reads this as proof of expiry (it compares block heights).
 */
export function isBlockhashExpired(e: unknown): boolean {
  const text = e instanceof Error ? `${e.message} ${String((e as { context?: unknown }).context ?? '')}` : JSON.stringify(e ?? '');
  return /blockhash not found|BlockhashNotFound|block height exceeded|TransactionExpired/i.test(text);
}

/** The JSON-RPC code of a preflight (simulation) refusal: the node did not forward the transaction. */
const PREFLIGHT_FAILURE = -32002;

/**
 * A send error after which the transaction was certainly NOT forwarded: a preflight (simulation) refusal that is not
 * about the blockhash. Anything else (a timeout, a dropped connection, a 5xx) may have delivered it.
 */
export function isPreflightRefusal(e: unknown): boolean {
  if (isBlockhashExpired(e)) return false;
  const ctx = (e as { context?: { __code?: unknown; code?: unknown } } | null)?.context;
  if (ctx && (ctx.__code === PREFLIGHT_FAILURE || ctx.code === PREFLIGHT_FAILURE)) return true;
  if ((e as { code?: unknown } | null)?.code === PREFLIGHT_FAILURE) return true;
  const text = e instanceof Error ? e.message : JSON.stringify(e ?? '');
  return /transaction simulation failed|preflight/i.test(text);
}

/** The most blocks a recent blockhash stays valid for after its own block. */
const MAX_PROCESSING_AGE = 150n;
/** Blocks to wait past `lastValidBlockHeight` before the final signature re-check. */
const SETTLE_BLOCKS = 2n;

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

function choosePath(wallet: SolanaWallet): SignPath | null {
  if (wallet.standard) return 'standard';
  if (wallet.signAllTransactions) return 'signAll';
  if (wallet.signTransaction) return 'oneByOne';
  return null;
}

/** Sign `unsigned` along `path`; returns the signed wires and the prompts it cost. */
async function signAll(wallet: SolanaWallet, path: SignPath, unsigned: Uint8Array[]): Promise<{ signed: readonly Uint8Array[]; prompts: number }> {
  if (path === 'standard') {
    const s = wallet.standard!;
    const out = await s.signTransaction(...unsigned.map((transaction) => ({ account: s.account, transaction, ...(s.chain ? { chain: s.chain } : {}) })));
    return { signed: out.map((o) => o.signedTransaction), prompts: 1 };
  }
  if (path === 'signAll') return { signed: await wallet.signAllTransactions!(unsigned), prompts: 1 };
  const out: Uint8Array[] = [];
  for (const u of unsigned) out.push(await wallet.signTransaction!(u));
  return { signed: out, prompts: unsigned.length };
}

const sameBytes = (a: Uint8Array, b: Uint8Array) => a.length === b.length && a.every((x, i) => x === b[i]);

/**
 * Check one wallet-returned transaction against the planned one: identical message bytes, the expected signer among
 * the required signers, and every required signature present and valid over the message.
 */
async function checkSigned(planned: Uint8Array, signed: Uint8Array, expected: Address | undefined): Promise<void> {
  const dec = getTransactionDecoder();
  let p, s;
  try {
    p = dec.decode(planned);
  } catch (e) {
    throw new Error(`the planned transaction does not decode (${e instanceof Error ? e.message : String(e)})`);
  }
  try {
    s = dec.decode(signed);
  } catch (e) {
    throw new Error(`the wallet returned bytes that do not decode as a transaction (${e instanceof Error ? e.message : String(e)})`);
  }
  if (!sameBytes(new Uint8Array(p.messageBytes), new Uint8Array(s.messageBytes))) {
    throw new Error('the wallet returned a different message than the one planned (instructions, accounts, fee payer or blockhash changed)');
  }
  const signers = Object.keys(s.signatures) as Address[];
  const who = expected ?? signers[0];
  if (!who || !signers.includes(who)) throw new Error(`the expected signer ${who ?? '(none)'} is not a required signer of this transaction`);
  for (const a of signers) {
    const sig = s.signatures[a] as SignatureBytes | null;
    if (!sig || sig.every((b) => b === 0)) throw new Error(`the signature of ${a} is missing`);
    const ok = await verifySignature(await getPublicKeyFromAddress(a), sig, s.messageBytes);
    if (!ok) throw new Error(`the signature of ${a} does not verify over the planned message`);
  }
}

/** The blockhash a wire transaction carries (its lifetime token). */
function blockhashOf(wire: Uint8Array): Blockhash {
  const tx = getTransactionDecoder().decode(wire);
  return getCompiledTransactionMessageDecoder().decode(tx.messageBytes).lifetimeToken as Blockhash;
}

/**
 * The block height after which `blockhash` can no longer land. The latest blockhash's own figure when it IS the
 * latest; otherwise an upper bound (the expiry decision also asks the node whether the blockhash is still valid).
 */
async function lastValidFor(rpc: SendRpc, blockhash: Blockhash, commitment: 'confirmed' | 'finalized'): Promise<bigint> {
  const { value } = await rpc.getLatestBlockhash({ commitment }).send();
  if (value.blockhash === blockhash) return value.lastValidBlockHeight;
  const height = await rpc.getBlockHeight({ commitment }).send();
  return height + 2n * MAX_PROCESSING_AGE;
}

type Status = { err: unknown; confirmationStatus?: string | null } | null;

async function statusOf(rpc: SendRpc, sig: Signature, history: boolean): Promise<Status> {
  const { value } = await rpc.getSignatureStatuses([sig], history ? { searchTransactionHistory: true } : undefined).send();
  return (value[0] ?? null) as Status;
}

/**
 * Sign every step with ONE wallet prompt (or one per step when the wallet cannot batch) and send them in order, each
 * confirmed before the next. Returns every signature and the signing path; throws a `SolanaSendError` naming the first
 * step that failed.
 */
export async function sendSolanaTransactions(wallet: SolanaWallet, steps: SendSteps, rpc: SendRpc, opts: SendOptions = {}): Promise<SendResult> {
  const list = 'transactions' in steps ? steps.transactions : (steps as readonly string[]);
  const name = (i: number) => opts.steps?.[i] ?? `#${i}`;
  const path = choosePath(wallet);
  if (!path) throw new SolanaSendError(0, name(0), null, [], 'sign', new Error('the wallet signs nothing (no Wallet Standard signer, signAllTransactions or signTransaction)'));
  const commitment = opts.commitment ?? 'confirmed';
  const timeout = opts.timeoutMs ?? 90_000;
  const poll = opts.pollMs ?? 500;
  const maxReplans = opts.maxReplans ?? 1;
  const expectedSigner = opts.signer ?? (wallet.standard?.account.address as Address | undefined);
  const b64in = getBase64Encoder();
  const b64out = getBase64Decoder();

  const landed: Signature[] = [];
  let prompts = 0;
  let replans = 0;
  let lastValid = opts.lastValidBlockHeight;
  let batch = list;
  let offset = 0; // index of batch[0] in the original steps

  for (;;) {
    const unsigned = batch.map((t) => new Uint8Array(b64in.encode(t)));
    let signed: readonly Uint8Array[];
    try {
      const r = await signAll(wallet, path, unsigned);
      signed = r.signed;
      prompts += r.prompts;
    } catch (e) {
      throw new SolanaSendError(offset, name(offset), null, [...landed], 'sign', e);
    }
    if (signed.length !== unsigned.length) {
      throw new SolanaSendError(offset, name(offset), null, [...landed], 'sign', new Error(`the wallet returned ${signed.length} of ${unsigned.length} transactions`));
    }
    for (let j = 0; j < signed.length; j++) {
      try {
        await checkSigned(unsigned[j]!, signed[j]!, expectedSigner);
      } catch (e) {
        throw new SolanaSendError(offset + j, name(offset + j), null, [...landed], 'sign', e);
      }
    }

    let expiredAt: number | null = null;
    let expiredSig: Signature | null = null;
    let expiredCause: unknown;
    for (let j = 0; j < signed.length; j++) {
      const i = offset + j;
      const wire = signed[j]!;
      const sig = firstSignature(wire);
      if (opts.beforeSend) {
        try {
          await opts.beforeSend({ index: i, step: name(i), landed: [...landed] });
        } catch (e) {
          throw new SolanaSendError(i, name(i), sig, [...landed], 'guard', e);
        }
      }
      const blockhash = blockhashOf(wire);
      if (lastValid === undefined) lastValid = await lastValidFor(rpc, blockhash, commitment);

      let sendError: unknown = null;
      try {
        await rpc.sendTransaction(b64out.decode(wire) as Base64EncodedWireTransaction, { encoding: 'base64', preflightCommitment: commitment }).send();
      } catch (e) {
        sendError = e;
      }
      if (sendError !== null && isPreflightRefusal(sendError)) {
        // The node simulated it and did not forward it; a re-check by signature settles the one doubt left.
        if (!(await statusOf(rpc, sig, true))) throw new SolanaSendError(i, name(i), sig, [...landed], 'send', sendError);
      }

      const until = Date.now() + timeout;
      let seen = false;
      let history = sendError !== null;
      let expired = false;
      for (;;) {
        const st = await statusOf(rpc, sig, history);
        if (st) {
          seen = true;
          if (st.err) throw new SolanaSendError(i, name(i), sig, [...landed], 'failed', st.err);
          if (st.confirmationStatus === commitment || st.confirmationStatus === 'finalized') break;
        } else if (!seen) {
          const height = await rpc.getBlockHeight({ commitment }).send();
          if (height > lastValid) {
            // Past the blockhash's last valid height: wait two more blocks, then re-check by signature with history.
            const settle = height + SETTLE_BLOCKS;
            while ((await rpc.getBlockHeight({ commitment }).send()) < settle) {
              if (Date.now() > until) throw new SolanaSendError(i, name(i), sig, [...landed], 'timeout', sendError ?? undefined);
              await sleep(poll);
            }
            const again = await statusOf(rpc, sig, true);
            if (again) {
              // it landed after all: wait for its confirmation as usual
              [seen, history] = [true, true];
              continue;
            }
            const { value: valid } = await rpc.isBlockhashValid(blockhash, { commitment }).send();
            if (!valid) {
              expired = true;
              break;
            }
          }
        }
        if (Date.now() > until) throw new SolanaSendError(i, name(i), sig, [...landed], 'timeout', sendError ?? undefined);
        await sleep(poll);
      }
      if (expired) {
        [expiredAt, expiredSig, expiredCause] = [i, sig, new Error(`not found by signature after block height passed ${lastValid}${sendError ? `; the send reported: ${sendError instanceof Error ? sendError.message : String(sendError)}` : ''}`)];
        break;
      }
      landed.push(sig);
    }
    if (expiredAt === null) return { signatures: landed, path, prompts, replans };

    if (!opts.replan || replans >= maxReplans) throw new SolanaSendError(expiredAt, name(expiredAt), expiredSig, [...landed], 'expired', expiredCause);
    replans++;
    const next = await opts.replan({ fromIndex: expiredAt, landed: [...landed] });
    if (next.transactions.length !== list.length - expiredAt) {
      throw new SolanaSendError(expiredAt, name(expiredAt), null, [...landed], 'sign', new Error(`replan returned ${next.transactions.length} transactions for ${list.length - expiredAt} remaining steps`));
    }
    batch = next.transactions;
    offset = expiredAt;
    lastValid = next.lastValidBlockHeight;
  }
}
