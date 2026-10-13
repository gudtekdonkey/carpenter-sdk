// sendSolanaTransactions: one signing call for the whole action, sends in order, each confirmed before the next, the
// first failure stops the run naming its step; every wallet-returned transaction is checked against the planned one;
// a step is never sent twice (a send error may have delivered it; expiry is proven by block height and a signature
// re-check, never by "Blockhash not found").

import assert from 'node:assert/strict';
import {
  appendTransactionMessageInstruction,
  compileTransaction,
  createTransactionMessage,
  generateKeyPair,
  getAddressFromPublicKey,
  getBase64Decoder,
  getBase64Encoder,
  getCompiledTransactionMessageDecoder,
  getTransactionDecoder,
  getTransactionEncoder,
  partiallySignTransaction,
  pipe,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
  type Address,
  type Blockhash,
} from '@solana/kit';

import {
  firstSignature,
  isBlockhashExpired,
  isPreflightRefusal,
  sendSolanaTransactions,
  SolanaSendError,
  type SendRpc,
  type SolanaWallet,
  type WalletStandardSigner,
} from '../src/send.ts';
import { addr } from './helpers.ts';
import { test } from './node-test.ts';

const BLOCKHASH = addr(900) as string as Blockhash;
const MEMO = addr(901);

const keys = await generateKeyPair();
const PAYER = await getAddressFromPublicKey(keys.publicKey);
const other = await generateKeyPair();

const b64 = (u: Uint8Array) => getBase64Decoder().decode(u);
const unb64 = (s: string) => new Uint8Array(getBase64Encoder().encode(s));

/** One unsigned step: a v0 message paid by PAYER whose one instruction carries `[tag]`. */
function step(tag: number, blockhash: Blockhash = BLOCKHASH, payer: Address = PAYER): string {
  const msg = pipe(
    createTransactionMessage({ version: 0 }),
    (m) => setTransactionMessageFeePayer(payer, m),
    (m) => setTransactionMessageLifetimeUsingBlockhash({ blockhash, lastValidBlockHeight: 0n }, m),
    (m) => appendTransactionMessageInstruction({ programAddress: MEMO, accounts: [], data: new Uint8Array([tag]) }, m),
  );
  return b64(new Uint8Array(getTransactionEncoder().encode(compileTransaction(msg))));
}
const steps = (n: number, from = 0, plus = 0, blockhash: Blockhash = BLOCKHASH) => Array.from({ length: n }, (_, i) => step(from + i + plus, blockhash));

const tagOfWire = (w: Uint8Array) => {
  const tx = getTransactionDecoder().decode(w);
  const m = getCompiledTransactionMessageDecoder().decode(tx.messageBytes) as { instructions: { data?: Uint8Array }[] };
  return m.instructions[0]!.data![0]!;
};

async function signWith(kp: CryptoKeyPair, u: Uint8Array): Promise<Uint8Array> {
  const tx = getTransactionDecoder().decode(u);
  return new Uint8Array(getTransactionEncoder().encode(await partiallySignTransaction([kp], tx)));
}
const signOne = (u: Uint8Array) => signWith(keys, u);

interface Log {
  sent: number[];
  history: number;
  /** the block height at each searchTransactionHistory re-check */
  historyHeights?: bigint[];
}

interface RpcOpts {
  /** tags whose transaction lands with an on-chain error */
  failAt?: number;
  /** tags whose send is refused at preflight (simulation) */
  refuseAt?: number;
  /** tags whose send throws a network error AFTER delivering the transaction */
  flakyLandsAt?: number;
  /** tags whose send throws a network error and the transaction never lands */
  flakyLostAt?: number;
  /** tags whose send throws "Blockhash not found" although the transaction is delivered */
  bhNotFoundAt?: number;
  /** tags delivered but never visible to a plain status poll (only a history search finds them) */
  lateAt?: number;
  /** tags that report `processed` once and then vanish */
  vanishAt?: number;
  /** tags never confirmed (always processed) */
  never?: boolean;
  /** polls before a delivered transaction reports `confirmed` */
  confirmAfter?: number;
  /** block height at the first call, and per `getBlockHeight` call */
  height?: bigint;
  heightStep?: bigint;
  /** what `getLatestBlockhash` answers */
  latest?: { blockhash: Blockhash; lastValidBlockHeight: bigint };
  /** what `isBlockhashValid` answers */
  valid?: boolean;
}

function mockRpc(log: Log, o: RpcOpts = {}): SendRpc {
  const polls = new Map<string, number>();
  const tagOf = new Map<string, number>();
  let height = o.height ?? 0n;
  const send = <T>(v: () => T | Promise<T>) => ({ send: async () => v() });
  return {
    sendTransaction: (w: string) =>
      send(() => {
        const bytes = unb64(w);
        const tag = tagOfWire(bytes);
        const sig = firstSignature(bytes);
        if (o.refuseAt === tag) {
          throw Object.assign(new Error('Transaction simulation failed: Error processing Instruction 0: custom program error 0x1'), { context: { __code: -32002 } });
        }
        if (o.flakyLostAt === tag) throw new Error('fetch failed (socket hang up)');
        log.sent.push(tag);
        tagOf.set(sig, tag);
        if (o.flakyLandsAt === tag) throw new Error('fetch failed (request timed out)');
        if (o.bhNotFoundAt === tag) throw new Error('Transaction simulation failed: Blockhash not found');
        return sig;
      }),
    getSignatureStatuses: (sigs: readonly string[], cfg?: { searchTransactionHistory?: boolean }) =>
      send(() => {
        if (cfg?.searchTransactionHistory) {
          log.history++;
          (log.historyHeights ??= []).push(height);
        }
        return {
          value: sigs.map((s) => {
            const tag = tagOf.get(s);
            if (tag === undefined) return null;
            const n = (polls.get(s) ?? 0) + 1;
            polls.set(s, n);
            if (o.lateAt === tag && !cfg?.searchTransactionHistory) return null;
            if (o.vanishAt === tag) return n === 1 ? { err: null, confirmationStatus: 'processed' } : null;
            if (o.failAt === tag) return { err: { InstructionError: [0, { Custom: 6001 }] }, confirmationStatus: 'processed' };
            if (o.never) return { err: null, confirmationStatus: 'processed' };
            return { err: null, confirmationStatus: n > (o.confirmAfter ?? 0) ? 'confirmed' : 'processed' };
          }),
        };
      }),
    getBlockHeight: () =>
      send(() => {
        const h = height;
        height += o.heightStep ?? 0n;
        return h;
      }),
    getLatestBlockhash: () => send(() => ({ value: o.latest ?? { blockhash: BLOCKHASH, lastValidBlockHeight: 1_000n } })),
    isBlockhashValid: () => send(() => ({ value: o.valid ?? false })),
  } as unknown as SendRpc;
}

const newLog = (): Log => ({ sent: [], history: 0 });
const batchWallet = (counter: { calls: number } = { calls: 0 }): SolanaWallet => ({
  signAllTransactions: async (us) => {
    counter.calls++;
    return Promise.all(us.map(signOne));
  },
});

test('batch path (legacy signAllTransactions): one prompt, sent in order, each confirmed before the next', async () => {
  const c = { calls: 0 };
  const wallet: SolanaWallet = { ...batchWallet(c), signTransaction: async () => assert.fail('must not sign one by one') };
  const log = newLog();
  const r = await sendSolanaTransactions(wallet, steps(3), mockRpc(log, { confirmAfter: 1 }), { pollMs: 1, lastValidBlockHeight: 100n });
  assert.equal(c.calls, 1);
  assert.deepEqual([r.path, r.prompts, r.replans], ['signAll', 1, 0]);
  assert.deepEqual(log.sent, [0, 1, 2]);
  assert.equal(r.signatures.length, 3);
});

test('batch path (Wallet Standard solana:signTransaction): every step in ONE call, each input carrying the account and chain', async () => {
  const calls: number[] = [];
  const account = { address: PAYER as string };
  const standard: WalletStandardSigner = {
    account,
    chain: 'solana:devnet',
    signTransaction: async (...inputs) => {
      calls.push(inputs.length);
      for (const i of inputs) {
        assert.equal(i.account, account);
        assert.equal(i.chain, 'solana:devnet');
      }
      return Promise.all(inputs.map(async (i) => ({ signedTransaction: await signOne(i.transaction) })));
    },
  };
  const log = newLog();
  const r = await sendSolanaTransactions({ standard, signAllTransactions: async () => assert.fail('the standard feature wins') }, { transactions: steps(2) }, mockRpc(log), { pollMs: 1 });
  assert.deepEqual(calls, [2]);
  assert.deepEqual([r.path, r.prompts], ['standard', 1]);
  assert.deepEqual(log.sent, [0, 1]);
});

test('fallback path: a wallet that cannot batch signs one by one, one prompt per step', async () => {
  let calls = 0;
  const wallet: SolanaWallet = {
    signTransaction: async (u) => {
      calls++;
      return signOne(u);
    },
  };
  const log = newLog();
  const r = await sendSolanaTransactions(wallet, steps(2), mockRpc(log), { pollMs: 1 });
  assert.equal(calls, 2);
  assert.deepEqual([r.path, r.prompts], ['oneByOne', 2]);
  assert.deepEqual(log.sent, [0, 1]);
});

test('the wallet must return the planned transaction: changed message, wrong key, missing signature, wrong signer → sign, nothing sent', async () => {
  const rpcLog = newLog();
  const rpc = mockRpc(rpcLog);
  // a wallet that swaps in a different message (another instruction byte)
  const tamper: SolanaWallet = { signAllTransactions: async (us) => Promise.all(us.map(async (u, i) => (i === 1 ? signOne(unb64(step(99))) : signOne(u)))) };
  const e1 = await sendSolanaTransactions(tamper, steps(2), rpc, { pollMs: 1 }).catch((e) => e);
  assert.ok(e1 instanceof SolanaSendError);
  assert.deepEqual([e1.reason, e1.index], ['sign', 1]);
  assert.match(e1.message, /different message/);
  // a signature by a key that is not the fee payer's
  const wrongKey: SolanaWallet = { signAllTransactions: async (us) => Promise.all(us.map((u) => signWith(other, u).catch(() => u))) };
  const e2 = await sendSolanaTransactions(wrongKey, steps(1), rpc, { pollMs: 1 }).catch((e) => e);
  assert.equal(e2.reason, 'sign');
  // a signature slot left zero
  const unsigned: SolanaWallet = { signAllTransactions: async (us) => us };
  const e3 = await sendSolanaTransactions(unsigned, steps(1), rpc, { pollMs: 1 }).catch((e) => e);
  assert.equal(e3.reason, 'sign');
  assert.match(e3.message, /missing/);
  // a valid-looking signature over other bytes
  const forged: SolanaWallet = {
    signAllTransactions: async (us) =>
      Promise.all(
        us.map(async (u) => {
          const s = await signOne(u);
          s[5] = s[5]! ^ 0xff;
          return s;
        }),
      ),
  };
  const e4 = await sendSolanaTransactions(forged, steps(1), rpc, { pollMs: 1 }).catch((e) => e);
  assert.equal(e4.reason, 'sign');
  assert.match(e4.message, /does not verify/);
  // the expected signer named explicitly but not a signer of the transaction
  const e5 = await sendSolanaTransactions(batchWallet(), steps(1), rpc, { pollMs: 1, signer: addr(5) }).catch((e) => e);
  assert.equal(e5.reason, 'sign');
  assert.match(e5.message, /not a required signer/);
  assert.deepEqual(rpcLog.sent, [], 'nothing was sent');
});

test('failure mid-way: an on-chain error stops the run and names the step; later steps are never sent', async () => {
  const log = newLog();
  const err = await sendSolanaTransactions(batchWallet(), steps(3), mockRpc(log, { failAt: 1 }), { pollMs: 1, steps: ['initializePool', 'addLiquidity', 'third'] }).catch((e) => e);
  assert.ok(err instanceof SolanaSendError);
  assert.deepEqual([err.index, err.step, err.reason], [1, 'addLiquidity', 'failed']);
  assert.equal(err.landed.length, 1, 'the first step landed');
  assert.deepEqual(log.sent, [0, 1]);
  assert.match(err.message, /step addLiquidity \(2\) failed on chain/);
});

test('a preflight refusal is re-checked by signature, then names its step; a refused or short signature sends nothing', async () => {
  const log = newLog();
  const err = await sendSolanaTransactions(batchWallet(), steps(2), mockRpc(log, { refuseAt: 1 }), { pollMs: 1 }).catch((e) => e);
  assert.deepEqual([err.reason, err.step], ['send', '#1']);
  assert.deepEqual(log.sent, [0]);
  assert.equal(log.history, 1, 'one re-check with searchTransactionHistory before giving up');

  const log2 = newLog();
  const rejecting: SolanaWallet = { signAllTransactions: async () => { throw new Error('User rejected the request'); } };
  const e2 = await sendSolanaTransactions(rejecting, steps(2), mockRpc(log2), { pollMs: 1 }).catch((e) => e);
  assert.equal(e2.reason, 'sign');
  assert.deepEqual(log2.sent, []);

  const short: SolanaWallet = { signAllTransactions: async (us) => Promise.all(us.slice(0, 1).map(signOne)) };
  const e3 = await sendSolanaTransactions(short, steps(2), mockRpc(newLog()), { pollMs: 1 }).catch((e) => e);
  assert.equal(e3.reason, 'sign');
  const none = await sendSolanaTransactions({}, steps(1), mockRpc(newLog())).catch((e) => e);
  assert.equal(none.reason, 'sign');
});

test('a send error that delivered the transaction is not a failure: the step is found by signature and the run goes on', async () => {
  const log = newLog();
  let replanned = 0;
  const r = await sendSolanaTransactions(batchWallet(), steps(2), mockRpc(log, { flakyLandsAt: 0 }), {
    pollMs: 1,
    lastValidBlockHeight: 100n,
    replan: async () => {
      replanned++;
      return { transactions: steps(2, 0, 16) };
    },
  });
  assert.deepEqual(log.sent, [0, 1], 'step 0 sent once, never again');
  assert.equal(replanned, 0);
  assert.equal(r.signatures.length, 2);
  assert.ok(log.history >= 1, 'the doubtful step is polled with searchTransactionHistory');
});

test('"Blockhash not found" alone is not expiry: before lastValidBlockHeight the step keeps waiting and lands', async () => {
  const log = newLog();
  let replanned = 0;
  const r = await sendSolanaTransactions(batchWallet(), steps(2), mockRpc(log, { bhNotFoundAt: 0, confirmAfter: 3, height: 10n, heightStep: 1n }), {
    pollMs: 1,
    lastValidBlockHeight: 100n,
    replan: async () => {
      replanned++;
      return { transactions: steps(2, 0, 16) };
    },
  });
  assert.equal(replanned, 0, 'no replan: the block height never passed 100');
  assert.deepEqual(log.sent, [0, 1]);
  assert.deepEqual([r.signatures.length, r.replans], [2, 0]);
});

test('expiry is proven before a replan: past lastValidBlockHeight, two more blocks, a history re-check, the blockhash invalid', async () => {
  const log = newLog();
  const asked: { fromIndex: number; landed: number }[] = [];
  const c = { calls: 0 };
  const r = await sendSolanaTransactions(batchWallet(c), steps(3), mockRpc(log, { flakyLostAt: 1, height: 99n, heightStep: 1n, valid: false }), {
    pollMs: 1,
    lastValidBlockHeight: 100n,
    replan: async (req) => {
      asked.push({ fromIndex: req.fromIndex, landed: req.landed.length });
      return { transactions: steps(2, 1, 16), lastValidBlockHeight: 10_000n };
    },
  });
  assert.deepEqual(asked, [{ fromIndex: 1, landed: 1 }]);
  assert.deepEqual(log.sent, [0, 17, 18], 'the lost step is replaced, the landed one is never re-sent');
  assert.deepEqual([r.signatures.length, r.prompts, r.replans, r.path, c.calls], [3, 2, 1, 'signAll', 2]);
  assert.ok(log.history >= 1, 'the decision re-checked the signature with searchTransactionHistory');
  // the height first read past 100 is 101; the decisive re-check waits for two more blocks
  assert.ok(log.historyHeights!.some((h) => h >= 103n), `a re-check at least two blocks past expiry (${log.historyHeights!.join(', ')})`);
  assert.ok(!log.historyHeights!.some((h) => h > 101n && h < 103n), 'no decisive re-check before the two blocks');
});

test('a step the history re-check finds after the height passed is NOT replanned; a still-valid blockhash is not expired', async () => {
  const log = newLog();
  let replanned = 0;
  const replan = async () => {
    replanned++;
    return { transactions: steps(1, 0, 16) };
  };
  // delivered, invisible to the plain poll, found by the history search
  const r = await sendSolanaTransactions(batchWallet(), steps(1), mockRpc(log, { lateAt: 0, height: 101n, heightStep: 1n }), { pollMs: 1, lastValidBlockHeight: 100n, replan });
  assert.deepEqual([replanned, r.signatures.length, log.sent.length], [0, 1, 1]);

  // lost, but the node still calls the blockhash valid (its lastValidBlockHeight was an under-estimate): keep waiting
  const e = await sendSolanaTransactions(batchWallet(), steps(1), mockRpc(newLog(), { flakyLostAt: 0, height: 101n, heightStep: 1n, valid: true }), {
    pollMs: 1,
    timeoutMs: 40,
    lastValidBlockHeight: 100n,
    replan,
  }).catch((x) => x);
  assert.equal(replanned, 0);
  assert.equal(e.reason, 'timeout', 'reported as may-still-land, never as expired');
});

test('a step once seen processed is never replanned, even when it vanishes past expiry', async () => {
  let replanned = 0;
  const e = await sendSolanaTransactions(batchWallet(), steps(1), mockRpc(newLog(), { vanishAt: 0, height: 200n, heightStep: 1n }), {
    pollMs: 1,
    timeoutMs: 40,
    lastValidBlockHeight: 100n,
    replan: async () => {
      replanned++;
      return { transactions: steps(1, 0, 16) };
    },
  }).catch((x) => x);
  assert.equal(replanned, 0);
  assert.equal(e.reason, 'timeout');
});

test('without lastValidBlockHeight the sender asks the node; expired without replan fails as expired; maxReplans bounds retries', async () => {
  // the latest blockhash is the step's own: its lastValidBlockHeight (5) is used, and the height (10) is past it
  const log = newLog();
  const e = await sendSolanaTransactions(batchWallet(), steps(2), mockRpc(log, { flakyLostAt: 0, height: 10n, heightStep: 1n, latest: { blockhash: BLOCKHASH, lastValidBlockHeight: 5n } }), { pollMs: 1 }).catch((x) => x);
  assert.deepEqual([e.reason, e.index, e.landed.length], ['expired', 0, 0]);
  assert.deepEqual(log.sent, []);

  // another blockhash is the latest: an upper bound (height + 300) is used, so a height of 10 is not expiry
  const e2 = await sendSolanaTransactions(batchWallet(), steps(1), mockRpc(newLog(), { flakyLostAt: 0, height: 10n, latest: { blockhash: addr(902) as string as Blockhash, lastValidBlockHeight: 5n } }), { pollMs: 1, timeoutMs: 30 }).catch((x) => x);
  assert.equal(e2.reason, 'timeout');

  const twice = await sendSolanaTransactions(batchWallet(), steps(1), mockRpc(newLog(), { flakyLostAt: 0, height: 101n, heightStep: 1n }), {
    pollMs: 1,
    lastValidBlockHeight: 100n,
    replan: async () => ({ transactions: steps(1), lastValidBlockHeight: 100n }),
  }).catch((x) => x);
  assert.equal(twice.reason, 'expired', 'maxReplans (1) bounds the retries');
});

test('beforeSend runs before each step; throwing stops the run as guard and sends nothing more', async () => {
  const log = newLog();
  const seen: number[] = [];
  const e = await sendSolanaTransactions(batchWallet(), steps(3), mockRpc(log), {
    pollMs: 1,
    steps: ['a', 'b', 'c'],
    beforeSend: ({ index, landed }) => {
      seen.push(index);
      assert.equal(landed.length, index);
      if (index === 1) throw new Error('price moved');
    },
  }).catch((x) => x);
  assert.deepEqual([e.reason, e.step], ['guard', 'b']);
  assert.match(e.message, /price moved/);
  assert.deepEqual(seen, [0, 1]);
  assert.deepEqual(log.sent, [0]);
});

test('a step never confirmed times out by name', async () => {
  const err = await sendSolanaTransactions(batchWallet(), steps(1), mockRpc(newLog(), { never: true }), { pollMs: 1, timeoutMs: 20, lastValidBlockHeight: 100n }).catch((e) => e);
  assert.deepEqual([err.reason, err.index], ['timeout', 0]);
  assert.match(err.message, /may still land/);
});

test('isBlockhashExpired reads the text only; isPreflightRefusal separates a refusal from a doubtful send', () => {
  assert.ok(isBlockhashExpired(new Error('Transaction simulation failed: Blockhash not found')));
  assert.ok(isBlockhashExpired({ err: 'BlockhashNotFound' }));
  assert.ok(isBlockhashExpired(new Error('block height exceeded')));
  assert.ok(!isBlockhashExpired(new Error('insufficient funds')));
  assert.ok(isPreflightRefusal(Object.assign(new Error('x'), { context: { __code: -32002 } })));
  assert.ok(isPreflightRefusal(new Error('Transaction simulation failed: custom program error 0x1')));
  assert.ok(!isPreflightRefusal(new Error('Transaction simulation failed: Blockhash not found')), 'a blockhash refusal is not proof of anything');
  assert.ok(!isPreflightRefusal(new Error('fetch failed')));
});
