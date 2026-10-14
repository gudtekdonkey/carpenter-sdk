/**
 * `carpenter_amm` transaction builders: swap, swap_from_balance, modify_liquidity, positions, tick arrays and
 * initialize_pool. Each returns `{ instructions, accounts, cuLimit, addressCount }`; signing and sending stay with the
 * caller (`sendSolanaTransactions`).
 *
 * Every builder:
 * - prepends `SetComputeUnitLimit` from `CU_LIMITS` (an unset limit is 0 under v1 transactions);
 * - asserts ≤ 64 unique accounts (the account-lock limit) and throws BY NAME above it;
 * - adds ATA `CreateIdempotent` for an output account and wSOL wrap / unwrap where it moves SOL;
 * - on a CPI-Guard Token-2022 source puts `ApproveChecked(delegate = pool)` IMMEDIATELY before the program instruction,
 *   in the same message, and `Revoke` after it when a residual delegation can remain.
 * The builders are HOOK-AGNOSTIC: a hooked pool takes its `hookSlice` (and `cuUnits` on a swap) from the caller, and a
 * hook's SDK wraps them with its defaults (`@floorlaunchpad/sdk-solana` for FLOOR pools).
 */

import { AccountRole, type AccountMeta, type Address, type Blockhash, type Instruction } from '@solana/kit';

import { addressToBytes, compareBytes, U64_MAX } from '../bytes.ts';
import {
  CU_LIMITS,
  DYNAMIC_FEE_FLAG,
  LEGACY_TX_BYTES,
  MAX_SQRT_PRICE_X64,
  MAX_TX_CU,
  MIN_SQRT_PRICE_X64,
  SYSTEM_PROGRAM_ID,
  WSOL_MINT,
} from '../constants.ts';
import type { Pool } from '../layouts.ts';
import {
  DEFAULT_PROGRAM_IDS,
  findAmmConfig,
  findApprovedHook,
  findAta,
  findBitmap,
  findEventAuthority,
  findMintExclusivity,
  findMintPools,
  findPool,
  findPoolIndexEntry,
  findPosition,
  findProgramData,
  findQuoteMint,
  findTickArray,
  findVault,
  NO_HOOK,
  type ProgramIds,
} from '../pda.ts';
import { tickArrayStartIndex } from '../tickmath.ts';
import {
  closePositionIx,
  closeTickArrayIx,
  initializePoolIx,
  initializeTickArrayIx,
  modifyLiquidityIx,
  swapFromBalanceIx,
  swapIx,
} from './instructions.ts';
import { finalize, finalizeUnits, sumCu, TooManyAccountsError, type BuiltTransaction } from './common.ts';
import { buildV0Message, messageSize } from './message.ts';
import { approveCheckedIx, createAtaIdempotentIx, revokeIx, syncNativeIx, systemTransferIx, tempWsol } from './spl.ts';

export { finalize, finalizeUnits, lockCount, sumCu, TooManyAccountsError, uniqueAccounts } from './common.ts';
export type { BuiltTransaction } from './common.ts';

const isDefault = (a: Address) => a === NO_HOOK;
const wsolOpts = (a: { wsolSeed?: string; wsolRentLamports?: bigint }) => ({
  ...(a.wsolSeed !== undefined ? { seed: a.wsolSeed } : {}),
  ...(a.wsolRentLamports !== undefined ? { rentLamports: a.wsolRentLamports } : {}),
});

/**
 * **A destination with `MemoTransfer` is REFUSED BY NAME, never repaired**. The SDK
 * decodes `MemoTransfer.require_incoming_transfer_memos` (`token2022.ts`, surfaced as
 * `getAtaExtensions(...).memoRequired`): a swap into such an account reverts `TokenError::NoMemo` (36) on chain.
 * Not the CPI-Guard treatment (an `ApproveChecked` the builder inserts): the memo has to be the
 * transfer's immediately-preceding SIBLING, and the transfer here happens inside the program's own
 * CPI, so a memo the client prepends is not sibling to it, and the program never CPIs Memo on the
 * trader's behalf. The builder names the blocker instead.
 */
/**
 * A settling hook (`HOOK_FLAGS.SETTLES`) that mints into a vault needs that vault's MINT writable; the swap's own metas
 * carry both mints read-only, and the AMM forwards each with the writability the transaction gave it. Upgrades the
 * swap instruction's `mint0` / `mint1` metas (fixed positions 7 / 8 of `swap` and `swap_from_balance`) where asked.
 */
function withWritableMints<T extends Instruction>(ix: T, mint0: Address, mint1: Address, writable: readonly [boolean, boolean] | undefined): T {
  if (!writable || (!writable[0] && !writable[1])) return ix;
  const accounts = [...(ix.accounts ?? [])];
  for (const [i, mint, on] of [[7, mint0, writable[0]], [8, mint1, writable[1]]] as const) {
    if (!on) continue;
    const meta = accounts[i];
    if (!meta || meta.address !== mint) throw new Error(`writableMints: account ${i} is not mint${i - 7}`);
    accounts[i] = { ...meta, role: AccountRole.WRITABLE };
  }
  return { ...ix, accounts } as T;
}

const MEMO_REFUSAL = (fn: string, what: string): string =>
  `${fn}: ${what} requires an incoming transfer memo (Token-2022 MemoTransfer, TokenError::NoMemo); disable it on the account first`;

// ---- swap ------------------------------------------------------------------------

export interface BuildSwapArgs {
  trader: Address;
  pool: Address;
  poolState: Pick<Pool, 'mint0' | 'mint1' | 'vault0' | 'vault1' | 'tokenProgram0' | 'tokenProgram1' | 'hook' | 'decimals0' | 'decimals1'>;
  zeroForOne: boolean;
  /** v4 sign: `< 0` exact-in (spend `-amountSpecified`), `> 0` exact-out. */
  amountSpecified: bigint;
  /**
   * Exact-in: the minimum out. Exact-out: the maximum in. An exact-in minimum of 0 accepts any price and is refused
   * unless `allowNoMinimum` is set.
   */
  amountLimit: bigint;
  /** Accept an exact-in `amountLimit` of 0 (no minimum out). Off by default. */
  allowNoMinimum?: boolean;
  sqrtPriceLimitX64?: bigint;
  /** From `predict.swapTickArrays` (the quoter's walk ∪ its actions' walks). */
  tickArrayStarts: readonly number[];
  /**
   * Tick arrays to create-if-missing BEFORE the swap — on a FLOOR sell, the predicted ACTIONS'
   * arrays (`quote.actionArrays`). `initialize_tick_array` is idempotent, so an existing array is left untouched; one
   * its rent owner closed just before this sell (an empty array, which `close_tick_array` allows) is re-created here,
   * paid by the trader and recorded as theirs, instead of the in-swap bid re-place failing the sell
   * (`HookActionAccountMissing`). Each must also be in `tickArrayStarts`.
   */
  ensureTickArrayStarts?: readonly number[];
  actionPositions?: readonly Address[];
  /** Defaults: the trader's ATAs. */
  traderTokenIn?: Address;
  traderTokenOut?: Address;
  /** `getAtaExtensions(traderTokenIn).cpiGuardLocked`. */
  sourceHasCpiGuard?: boolean;
  /**
   * `getAtaExtensions(traderTokenOut).memoRequired` — the output leg is a transfer INTO that
   * account, and Token-2022 refuses one without a preceding Memo. REFUSED by name, never fixed
   * by prepending a Memo: the transfer happens inside the program's own CPI, where a client memo is not its sibling.
   */
  destinationRequiresMemo?: boolean;
  /**
   * The hook slice — REQUIRED on a hooked pool: the accounts the approved hook's callbacks read, in its order (Carpenter
   * names none of its own). a FLOOR pool's default slice is `@floorlaunchpad/sdk-solana`'s (`floorSwapHookSlice`,
   * and its own `buildSwap` fills it).
   */
  hookSlice?: readonly AccountMeta[];
  hookData?: Uint8Array;
  /**
   * Move SOL in and out as native SOL (default true). A wSOL side then goes through a TEMPORARY wSOL account created,
   * funded and closed inside this transaction (`tempWsol`): the input is funded with exactly what the swap may spend,
   * the output is closed back to SOL, and the trader's own wSOL account (if any) is neither used nor closed. With an
   * explicit `traderTokenIn` on a wSOL input the builder tops that account up (transfer + `SyncNative`) and never closes
   * it. `false`: the wSOL sides use the trader's wSOL ATA as is, nothing wrapped or closed — what a leg whose wSOL
   * output is the next leg's input (`buildSwapFromBalance`) needs.
   */
  wrapSol?: boolean;
  /** The temporary wSOL account's seed (default: random). */
  wsolSeed?: string;
  /** Its rent (default `TOKEN_ACCOUNT_RENT_LAMPORTS`; every lamport returns when it is closed). */
  wsolRentLamports?: bigint;
  /**
   * A Token-2022 TRANSFER HOOK's extra accounts for a leg whose mint carries one. FLOOR's `transferHook` option, which derives these for a
   * holders-hooked launch and opens the buyer's record, is `@floorlaunchpad/sdk-solana`'s `buildSwap`.
   */
  transferHookAccounts?: readonly AccountMeta[];
  /**
   * The compute-unit limit, REQUIRED on a hooked pool: the hook's callbacks are the hook's cost, which Carpenter cannot
   * know. Omitted on a plain pool: `CU_LIMITS.swapPlain`.
   */
  cuUnits?: number;
  /**
   * `[mint0, mint1]` writable — for a pool whose hook SETTLES (`HOOK_FLAGS.SETTLES`) and mints into that side's vault
   * inside the swap. Default: both read-only.
   */
  writableMints?: readonly [boolean, boolean];
  ids?: ProgramIds;
}

export async function buildSwap(a: BuildSwapArgs): Promise<BuiltTransaction> {
  const ids = a.ids ?? DEFAULT_PROGRAM_IDS;
  const p = a.poolState;
  if (a.amountSpecified === 0n) throw new Error('buildSwap: amountSpecified is 0 (AmountZero)');
  if (a.amountSpecified < 0n && a.amountLimit <= 0n && !a.allowNoMinimum) {
    throw new Error('buildSwap: amountLimit (the minimum out) is 0 — any price would fill; set a minimum, or allowNoMinimum: true');
  }
  const hooked = !isDefault(p.hook);
  if (a.destinationRequiresMemo) throw new Error(MEMO_REFUSAL('buildSwap', 'the output account'));

  const [mintIn, mintOut] = a.zeroForOne ? [p.mint0, p.mint1] : [p.mint1, p.mint0];
  const [tpIn, tpOut] = a.zeroForOne ? [p.tokenProgram0, p.tokenProgram1] : [p.tokenProgram1, p.tokenProgram0];
  const decimalsIn = a.zeroForOne ? p.decimals0 : p.decimals1;
  const maxIn = a.amountSpecified < 0n ? -a.amountSpecified : a.amountLimit;
  const wrap = a.wrapSol ?? true;
  // A temporary wSOL account for a wSOL side the caller did not name (only one side of a pool can be wSOL).
  const tempIn = wrap && mintIn === WSOL_MINT && !a.traderTokenIn ? await tempWsol(a.trader, maxIn, wsolOpts(a)) : null;
  const tempOut = wrap && mintOut === WSOL_MINT && !a.traderTokenOut ? await tempWsol(a.trader, 0n, wsolOpts(a)) : null;
  const tokenIn = tempIn?.account ?? a.traderTokenIn ?? (await findAta(a.trader, mintIn, tpIn)).address;
  const tokenOut = tempOut?.account ?? a.traderTokenOut ?? (await findAta(a.trader, mintOut, tpOut)).address;

  if (hooked && a.hookSlice === undefined) throw new Error('buildSwap: a hooked pool needs an explicit hookSlice (the hook names its own accounts)');
  if (hooked && a.cuUnits === undefined) throw new Error('buildSwap: a hooked pool needs cuUnits (the hook\'s callbacks are its own cost)');
  const hookSlice: readonly AccountMeta[] = a.hookSlice ?? [];

  const [bitmap, eventAuthority, arrays] = await Promise.all([
    findBitmap(a.pool, ids),
    findEventAuthority(ids.carpenterAmm),
    Promise.all(a.tickArrayStarts.map(async (s) => (await findTickArray(a.pool, s, ids)).address)),
  ]);

  const accounts = {
    program: ids.carpenterAmm,
    eventAuthority: eventAuthority.address,
    trader: a.trader,
    pool: a.pool,
    bitmap: bitmap.address,
    traderTokenIn: tokenIn,
    traderTokenOut: tokenOut,
    vault0: p.vault0,
    vault1: p.vault1,
    mint0: p.mint0,
    mint1: p.mint1,
    tokenProgram0: p.tokenProgram0,
    tokenProgram1: p.tokenProgram1,
    hookProgram: hooked ? p.hook : null,
    hookSlice,
    tickArrays: arrays,
    actionPositions: a.actionPositions ?? [],
  };
  const args = {
    zeroForOne: a.zeroForOne,
    amountSpecified: a.amountSpecified,
    sqrtPriceLimitX64: a.sqrtPriceLimitX64 ?? (a.zeroForOne ? MIN_SQRT_PRICE_X64 + 1n : MAX_SQRT_PRICE_X64 - 1n),
    amountLimit: a.amountLimit,
    hookData: a.hookData,
  };
  const main = withWritableMints(
    a.transferHookAccounts?.length ? swapIx({ ...accounts, transferHookAccounts: a.transferHookAccounts }, args) : swapIx(accounts, args),
    p.mint0,
    p.mint1,
    a.writableMints,
  );

  const ensure = [...new Set(a.ensureTickArrayStarts ?? [])];
  for (const st of ensure) {
    if (!a.tickArrayStarts.includes(st)) throw new Error(`buildSwap: ensureTickArrayStarts ${st} is not in tickArrayStarts`);
  }
  const ensureIxs = await Promise.all(
    ensure.map(async (st) =>
      initializeTickArrayIx({ program: ids.carpenterAmm, payer: a.trader, pool: a.pool, tickArray: (await findTickArray(a.pool, st, ids)).address, startIndex: st }),
    ),
  );
  const body: Instruction[] = [...ensureIxs, ...(tempOut ? tempOut.open : [createAtaIdempotentIx(a.trader, tokenOut, a.trader, mintOut, tpOut)])];
  if (tempIn) body.push(...tempIn.open);
  else if (wrap && mintIn === WSOL_MINT) body.push(systemTransferIx(a.trader, tokenIn, maxIn), syncNativeIx(tokenIn));
  if (a.sourceHasCpiGuard) body.push(approveCheckedIx(tokenIn, mintIn, a.pool, a.trader, maxIn, decimalsIn, tpIn));
  body.push(main);
  // The walk may consume less than the approval (a price-limit stop, an exact-out quote): revoke the rest.
  if (a.sourceHasCpiGuard) body.push(revokeIx(tokenIn, a.trader, tpIn));
  if (tempIn) body.push(tempIn.close);
  if (tempOut) body.push(tempOut.close);

  const units = hooked ? a.cuUnits! : CU_LIMITS.swapPlain;
  return finalizeUnits('buildSwap', Math.min(units + ensure.length * CU_LIMITS.initializeTickArray, MAX_TX_CU), body, main);
}

// ---- swap_from_balance -----------------------------------------------

export interface BuildSwapFromBalanceArgs extends Omit<BuildSwapArgs, 'amountSpecified' | 'wrapSol' | 'tickArrayStarts' | 'actionPositions' | 'ensureTickArrayStarts'> {
  /**
   * NOT `buildSwap`'s "from `predict.swapTickArrays`" at the QUOTED amount. The resolved input is not known at the press and can be LARGER than the amount
   * this leg was quoted at — a favourable first leg, or an inflow earlier in the transaction — and
   * the walk needs the array holding EVERY next initialized tick it steps toward. A cover computed
   * at the quoted amount therefore reverts the WHOLE transaction `TickArrayMissing` the moment a
   * larger input crosses one more initialized tick;
   * the exact-in `swap` this replaces spent the first leg's minimum and could not fail this way.
   * Cover an UPPER BOUND of the resolved input: the union of `swapTickArrays` over the quote at the
   * expected amount AND at the favourable edge of the first leg plus any inflow the transaction
   * makes — or a reach cover (every initialized array from the pool's tick toward the price limit,
   * within the account cap).
   */
  tickArrayStarts: readonly number[];
  /**
   * The hook's action positions (WRITABLE). The same UPPER BOUND as `tickArrayStarts`: on a
   * FLOOR sell a resolved amount larger than the quote can reach the bid band and trigger the
   * in-swap re-place the quoted amount did not, and a missing position (or its arrays) is
   * `HookActionAccountMissing`, which fails the swap. Pass every action the upper bound can trigger.
   */
  actionPositions?: readonly Address[];
  /**
   * The trader's RAW balance of the input token at the press (the token account's `amount`, never
   * a `ScaledUiAmount` / `InterestBearing` UI figure); the swap spends everything ABOVE it.
   * `0n` ONLY when the input account does not exist yet (it is created earlier in the same
   * transaction). Never default it to `0n` on a FAILED read: that sells the trader's whole
   * pre-existing holding. A failed read refuses the press by name, in the caller.
   */
  keep: bigint;
  /**
   * REQUIRED, no default — the caller decides. `0n` = the absolute floor alone (exact-in's
   * `amountLimit`); `M` (the first leg's MINIMUM) = the rate clause: every unit above `M` must
   * clear the per-unit rate `amountLimit / M`.
   */
  limitInBasis: bigint;
  /**
   * Receive a wSOL OUTPUT as native SOL (default true): through a temporary wSOL account created and closed in this
   * transaction, unless `traderTokenOut` is given (an account the builder did not create is never closed). The INPUT is
   * never wrapped in this mode: its amount is unknown at the press.
   */
  unwrapOut?: boolean;
}

/**
 * `swap_from_balance`: `buildSwap`'s transaction for the
 * second leg of a composed route, whose input is whatever the first leg delivered. Line for line
 * `buildSwap`, except:
 * - it never wraps the input (a wSOL input must already be wrapped — and must still EXIST: a first
 *   leg that PAYS INTO wSOL must be built with `buildSwap({ wrapSol: false })`, because `buildSwap`'s
 *   one `wrapSol` flag also closes a wSOL OUTPUT by default, which would leave this leg's
 *   `trader_token_in` closed before it reads it);
 * - its `tickArrayStarts` / `actionPositions` cover the UPPER bound of the resolved input, never
 *   the quoted amount alone (see the field docs);
 * - `keep` / `limitInBasis` outside u64 throw by name;
 * - on a CPI-Guard source it approves the pool for `u64::MAX` (the resolved amount is unknown at
 *   the press) and `Revoke`s in the same transaction — safe because the pool PDA moves a guarded
 *   source only inside `transfer_checked_in`, for a swap the TRADER signs, and the delegation does
 *   not outlive the transaction. A finite bound would revert `DelegateMissing` on a
 *   favourable first leg for no gain.
 * - the CU key is `buildSwap`'s (the mode adds one pool load, two key compares, one compare and
 *   two u128 multiplies).
 * A composing caller takes the body and drops the per-leg compute-budget instruction, as with
 * `buildSwap`.
 */
export async function buildSwapFromBalance(a: BuildSwapFromBalanceArgs): Promise<BuiltTransaction> {
  const ids = a.ids ?? DEFAULT_PROGRAM_IDS;
  const p = a.poolState;
  if (a.keep < 0n || a.keep > U64_MAX) throw new Error(`buildSwapFromBalance: keep ${a.keep} is outside u64`);
  if (a.limitInBasis < 0n || a.limitInBasis > U64_MAX) throw new Error(`buildSwapFromBalance: limitInBasis ${a.limitInBasis} is outside u64`);
  if (a.destinationRequiresMemo) throw new Error(MEMO_REFUSAL('buildSwapFromBalance', 'the output account'));
  if (a.amountLimit <= 0n && !a.allowNoMinimum) {
    throw new Error('buildSwapFromBalance: amountLimit (the minimum out) is 0 — any price would fill; set a minimum, or allowNoMinimum: true');
  }
  const hooked = !isDefault(p.hook);

  const [mintIn, mintOut] = a.zeroForOne ? [p.mint0, p.mint1] : [p.mint1, p.mint0];
  const [tpIn, tpOut] = a.zeroForOne ? [p.tokenProgram0, p.tokenProgram1] : [p.tokenProgram1, p.tokenProgram0];
  const decimalsIn = a.zeroForOne ? p.decimals0 : p.decimals1;
  const tempOut = (a.unwrapOut ?? true) && mintOut === WSOL_MINT && !a.traderTokenOut ? await tempWsol(a.trader, 0n, wsolOpts(a)) : null;
  const tokenIn = a.traderTokenIn ?? (await findAta(a.trader, mintIn, tpIn)).address;
  const tokenOut = tempOut?.account ?? a.traderTokenOut ?? (await findAta(a.trader, mintOut, tpOut)).address;

  if (hooked && a.hookSlice === undefined) throw new Error('buildSwapFromBalance: a hooked pool needs an explicit hookSlice (the hook names its own accounts)');
  if (hooked && a.cuUnits === undefined) throw new Error('buildSwapFromBalance: a hooked pool needs cuUnits (the hook\'s callbacks are its own cost)');
  const hookSlice: readonly AccountMeta[] = a.hookSlice ?? [];

  const [bitmap, eventAuthority, arrays] = await Promise.all([
    findBitmap(a.pool, ids),
    findEventAuthority(ids.carpenterAmm),
    Promise.all(a.tickArrayStarts.map(async (s) => (await findTickArray(a.pool, s, ids)).address)),
  ]);

  const accounts = {
    program: ids.carpenterAmm,
    eventAuthority: eventAuthority.address,
    trader: a.trader,
    pool: a.pool,
    bitmap: bitmap.address,
    traderTokenIn: tokenIn,
    traderTokenOut: tokenOut,
    vault0: p.vault0,
    vault1: p.vault1,
    mint0: p.mint0,
    mint1: p.mint1,
    tokenProgram0: p.tokenProgram0,
    tokenProgram1: p.tokenProgram1,
    hookProgram: hooked ? p.hook : null,
    hookSlice,
    tickArrays: arrays,
    actionPositions: a.actionPositions ?? [],
  };
  const args = {
    zeroForOne: a.zeroForOne,
    keep: a.keep,
    sqrtPriceLimitX64: a.sqrtPriceLimitX64 ?? (a.zeroForOne ? MIN_SQRT_PRICE_X64 + 1n : MAX_SQRT_PRICE_X64 - 1n),
    amountLimit: a.amountLimit,
    limitInBasis: a.limitInBasis,
    hookData: a.hookData,
  };
  const main = withWritableMints(
    a.transferHookAccounts?.length
      ? swapFromBalanceIx({ ...accounts, transferHookAccounts: a.transferHookAccounts }, args)
      : swapFromBalanceIx(accounts, args),
    p.mint0,
    p.mint1,
    a.writableMints,
  );

  const body: Instruction[] = tempOut ? [...tempOut.open] : [createAtaIdempotentIx(a.trader, tokenOut, a.trader, mintOut, tpOut)];
  if (a.sourceHasCpiGuard) body.push(approveCheckedIx(tokenIn, mintIn, a.pool, a.trader, U64_MAX, decimalsIn, tpIn));
  body.push(main);
  if (a.sourceHasCpiGuard) body.push(revokeIx(tokenIn, a.trader, tpIn));
  if (tempOut) body.push(tempOut.close);

  return hooked ? finalizeUnits('buildSwapFromBalance', a.cuUnits!, body, main) : finalize('buildSwapFromBalance', 'swapPlain', body, main);
}

// ---- modify_liquidity -------------------------------------------------------------

export interface BuildModifyLiquidityArgs {
  owner: Address;
  pool: Address;
  poolState: Pick<Pool, 'mint0' | 'mint1' | 'vault0' | 'vault1' | 'tokenProgram0' | 'tokenProgram1' | 'hook' | 'decimals0' | 'decimals1' | 'tickSpacing'>;
  salt: bigint;
  tickLower: number;
  tickUpper: number;
  liquidityDelta: bigint;
  amount0Max: bigint;
  amount1Max: bigint;
  /**
   * The least the owner must RECEIVE of each side (the program's trailing `min_out`). A REMOVAL must set at least one
   * (an out-of-range position pays one side only), or the withdrawal can be sandwiched at a pushed price: a removal
   * with neither is refused unless `allowNoMinimum` is set. A deposit is bounded by `amount0Max` / `amount1Max`
   * instead, and a deposit whose maximum is `u64::MAX` (no bound) is refused the same way.
   */
  amount0Min?: bigint;
  amount1Min?: bigint;
  /** Accept a removal with no minimum received, or a deposit with an unbounded maximum. Off by default. */
  allowNoMinimum?: boolean;
  ownerToken0?: Address;
  ownerToken1?: Address;
  token0HasCpiGuard?: boolean;
  token1HasCpiGuard?: boolean;
  /** Pays for a new position or tick array; defaults to the owner. */
  payer?: Address;
  /**
   * For `close_tick_array`: the rent owner every tick array this call CREATES records — the
   * only signer that may later close it. Omitted (the zero key), the program records the PAYER. Ignored
   * when both arrays already exist; an existing array is never re-owned.
   */
  rentOwner?: Address | null;
  /**
   * The position's IDENTITY — the third PDA seed AND a re-checked FIELD on the account. Defaults to the OWNER, which is
   * the ordinary wallet LP and the case the owner's own signature proves (the program's `modify_liquidity`). A different id needs
   * `positionAuthority` to sign for it at creation; that is a hook's shape (`position_id = hook_authority`) and is not
   * built here.
   */
  positionId?: Address;
  /** REQUIRED on a hooked pool. */
  hookSlice?: readonly AccountMeta[];
  hookData?: Uint8Array;
  /**
   * Move a wSOL side as native SOL (default true): through a temporary wSOL account created, funded (a deposit: with
   * that side's maximum) and closed inside this transaction, unless that side's `ownerToken0` / `ownerToken1` is given.
   * The owner's own wSOL account is neither used nor closed. `false`: the owner's wSOL ATA as is.
   */
  wrapSol?: boolean;
  /** The temporary wSOL account's seed (default: random). */
  wsolSeed?: string;
  /** Its rent (default `TOKEN_ACCOUNT_RENT_LAMPORTS`; every lamport returns when it is closed). */
  wsolRentLamports?: bigint;
  /**
   * A Token-2022 TRANSFER HOOK's extra accounts for a leg whose mint carries one, and the
   * compute their `execute` adds (`transferHookCu`, default 0). FLOOR's `transferHook` option is
   * `@floorlaunchpad/sdk-solana`'s `buildModifyLiquidity`.
   */
  transferHookAccounts?: readonly AccountMeta[];
  transferHookCu?: number;
  ids?: ProgramIds;
}

export async function buildModifyLiquidity(a: BuildModifyLiquidityArgs): Promise<BuiltTransaction> {
  const ids = a.ids ?? DEFAULT_PROGRAM_IDS;
  const p = a.poolState;
  const s = p.tickSpacing;
  if (a.tickLower >= a.tickUpper || a.tickLower % s !== 0 || a.tickUpper % s !== 0) throw new Error('buildModifyLiquidity: ticks must be aligned and lower < upper (TickInvalid)');
  const hooked = !isDefault(p.hook);
  if (hooked && a.hookSlice === undefined) throw new Error('buildModifyLiquidity: a hooked pool needs an explicit hookSlice');
  const hookSlice = a.hookSlice ?? [];
  const adding = a.liquidityDelta > 0n;
  const removing = a.liquidityDelta < 0n;
  if (removing && !a.allowNoMinimum && (a.amount0Min ?? 0n) <= 0n && (a.amount1Min ?? 0n) <= 0n) {
    throw new Error('buildModifyLiquidity: a removal with no minimum received (amount0Min / amount1Min) accepts any price; set one, or allowNoMinimum: true');
  }
  if (adding && !a.allowNoMinimum && (a.amount0Max >= U64_MAX || a.amount1Max >= U64_MAX)) {
    throw new Error('buildModifyLiquidity: a deposit with a u64::MAX maximum has no bound; set amount0Max / amount1Max, or allowNoMinimum: true');
  }

  const wrap = a.wrapSol ?? true;
  const temp0 = wrap && p.mint0 === WSOL_MINT && !a.ownerToken0 ? await tempWsol(a.owner, adding ? a.amount0Max : 0n, wsolOpts(a)) : null;
  const temp1 = wrap && p.mint1 === WSOL_MINT && !a.ownerToken1 ? await tempWsol(a.owner, adding ? a.amount1Max : 0n, wsolOpts(a)) : null;
  const t0 = temp0?.account ?? a.ownerToken0 ?? (await findAta(a.owner, p.mint0, p.tokenProgram0)).address;
  const t1 = temp1?.account ?? a.ownerToken1 ?? (await findAta(a.owner, p.mint1, p.tokenProgram1)).address;
  const positionId = a.positionId ?? a.owner;
  const [bitmap, position, lower, upper, eventAuthority] = await Promise.all([
    findBitmap(a.pool, ids),
    findPosition(a.pool, positionId, a.salt, ids),
    findTickArray(a.pool, tickArrayStartIndex(a.tickLower, s), ids),
    findTickArray(a.pool, tickArrayStartIndex(a.tickUpper, s), ids),
    findEventAuthority(ids.carpenterAmm),
  ]);
  const payer = a.payer ?? a.owner;
  const hookExtra = a.transferHookAccounts ?? [];
  const main = modifyLiquidityIx(
    {
      program: ids.carpenterAmm,
      eventAuthority: eventAuthority.address,
      owner: a.owner,
      payer,
      pool: a.pool,
      bitmap: bitmap.address,
      position: position.address,
      tickArrayLower: lower.address,
      tickArrayUpper: upper.address,
      ownerToken0: t0,
      ownerToken1: t1,
      vault0: p.vault0,
      vault1: p.vault1,
      mint0: p.mint0,
      mint1: p.mint1,
      tokenProgram0: p.tokenProgram0,
      tokenProgram1: p.tokenProgram1,
      hookProgram: hooked ? p.hook : null,
      // The ordinary wallet LP is its own `position_id`, so the
      // owner's signature proves it and no second signer is needed.
      // A hook's position (`position_id = hook_authority`) is not built here.
      positionAuthority: null,
      hookSlice,
      ...(hookExtra.length ? { transferHookAccounts: hookExtra } : {}),
    },
    {
      positionId: addressToBytes(positionId),
      salt: a.salt,
      tickLower: a.tickLower,
      tickUpper: a.tickUpper,
      liquidityDelta: a.liquidityDelta,
      amount0Max: a.amount0Max,
      amount1Max: a.amount1Max,
      hookData: a.hookData,
      rentOwner: a.rentOwner ?? null,
      ...(a.amount0Min !== undefined ? { amount0Min: a.amount0Min } : {}),
      ...(a.amount1Min !== undefined ? { amount1Min: a.amount1Min } : {}),
    },
  );

  const body: Instruction[] = [
    ...(temp0 ? temp0.open : [createAtaIdempotentIx(a.owner, t0, a.owner, p.mint0, p.tokenProgram0)]),
    ...(temp1 ? temp1.open : [createAtaIdempotentIx(a.owner, t1, a.owner, p.mint1, p.tokenProgram1)]),
  ];
  // an explicitly named wSOL account is topped up for a deposit, never closed
  if (adding && wrap && p.mint0 === WSOL_MINT && !temp0) body.push(systemTransferIx(a.owner, t0, a.amount0Max), syncNativeIx(t0));
  if (adding && wrap && p.mint1 === WSOL_MINT && !temp1) body.push(systemTransferIx(a.owner, t1, a.amount1Max), syncNativeIx(t1));
  if (adding && a.token0HasCpiGuard) body.push(approveCheckedIx(t0, p.mint0, a.pool, a.owner, a.amount0Max, p.decimals0, p.tokenProgram0));
  if (adding && a.token1HasCpiGuard) body.push(approveCheckedIx(t1, p.mint1, a.pool, a.owner, a.amount1Max, p.decimals1, p.tokenProgram1));
  body.push(main);
  if (adding && a.token0HasCpiGuard) body.push(revokeIx(t0, a.owner, p.tokenProgram0));
  if (adding && a.token1HasCpiGuard) body.push(revokeIx(t1, a.owner, p.tokenProgram1));
  if (temp0) body.push(temp0.close);
  if (temp1) body.push(temp1.close);
  // The main row PLUS the two ATA CreateIdempotents every modify carries (one per side): a first add that opens the
  // owner's wSOL account measured 94,476 CU against a flat 90,000. A leg whose mint has a transfer hook adds
  // `transferHookCu`.
  return finalizeUnits('buildModifyLiquidity', Math.min(sumCu('modifyLiquidity', 'ataCreate', 'ataCreate') + (a.transferHookCu ?? 0), MAX_TX_CU), body, main);
}

export async function buildClosePosition(a: { owner: Address; pool: Address; salt: bigint; ids?: ProgramIds }): Promise<BuiltTransaction> {
  const ids = a.ids ?? DEFAULT_PROGRAM_IDS;
  const [position, ev] = await Promise.all([findPosition(a.pool, a.owner, a.salt, ids), findEventAuthority(ids.carpenterAmm)]);
  const main = closePositionIx({ program: ids.carpenterAmm, eventAuthority: ev.address, owner: a.owner, position: position.address });
  return finalize('buildClosePosition', 'closePosition', [main], main);
}

/**
 * `initialize_tick_array`. `rentOwner` is the key the new
 * (3,920-byte) array records as the ONLY signer that may close it and take its rent back; omitted,
 * the program records the PAYER.
 */
export async function buildInitializeTickArray(a: { payer: Address; pool: Address; startIndex: number; rentOwner?: Address | null; ids?: ProgramIds }): Promise<BuiltTransaction> {
  const ids = a.ids ?? DEFAULT_PROGRAM_IDS;
  const ta = await findTickArray(a.pool, a.startIndex, ids);
  const main = initializeTickArrayIx({ program: ids.carpenterAmm, payer: a.payer, pool: a.pool, tickArray: ta.address, startIndex: a.startIndex, rentOwner: a.rentOwner ?? null });
  return finalize('buildInitializeTickArray', 'initializeTickArray', [main], main);
}

/** One tick array `buildCloseTickArrays` closes: its pool and start index (the PDA is derived). */
export interface CloseTickArrayTarget {
  pool: Address;
  startIndex: number;
}

/**
 * `close_tick_array` × N in ONE transaction: each array's
 * rent — ALL its lamports — goes back to `rentOwner`, who signs and pays the fee.
 *
 * **Packed under the 1,232-byte PACKET, refused by name over it.** Each close names one UNIQUE
 *    account (its tick array) beside its pool and bitmap, so the packet fills long before the 64 locks:
 *    measured through `@solana/kit`'s own compile, a v0 message with NO lookup
 *    table carries {@link MAX_CLOSE_TICK_ARRAYS_ONE_POOL} closes of one pool and refuses one more with
 *    {@link TooManyAccountsError} (its `packetBytes` set). A lookup table only shrinks a message, so a
 *    batch this builder returns fits with or without one.
 * Duplicates are refused by name (the second close of one array would fail on chain).
 * The builder checks SHAPE only. Whether each array is closable — an array whose tail names this
 *    signer, no initialized tick, zero tick bytes, its bitmap bit clear — is the chain's to answer
 *    (`closeTickArrayIx` lists the refusals): SIMULATE before sending. `getTickArray(...).rentOwner`
 *    says who may close one; a zero tail reads `null` and never can.
 * A hook may own tick arrays whose rent owner no wallet can sign as (FLOOR's prepared graduation arrays); those are
 *    closed through the hook's own instruction, not this one.
 * `CU_LIMITS.closeTickArray` per close, `[est.]`.
 */
export async function buildCloseTickArrays(a: { rentOwner: Address; arrays: readonly CloseTickArrayTarget[]; ids?: ProgramIds }): Promise<BuiltTransaction> {
  const ids = a.ids ?? DEFAULT_PROGRAM_IDS;
  if (a.arrays.length === 0) throw new Error('buildCloseTickArrays: no arrays to close');
  const seen = new Set<string>();
  for (const t of a.arrays) {
    const k = `${t.pool}:${t.startIndex}`;
    if (seen.has(k)) throw new Error(`buildCloseTickArrays: the array ${t.startIndex} of pool ${t.pool} is named twice — the second close would fail on chain`);
    seen.add(k);
  }
  const ev = await findEventAuthority(ids.carpenterAmm);
  const ixs = await Promise.all(
    a.arrays.map(async (t) => {
      const [bitmap, ta] = await Promise.all([findBitmap(t.pool, ids), findTickArray(t.pool, t.startIndex, ids)]);
      return closeTickArrayIx({ program: ids.carpenterAmm, eventAuthority: ev.address, rentOwner: a.rentOwner, pool: t.pool, bitmap: bitmap.address, tickArray: ta.address });
    }),
  );
  const main = ixs[0]!;
  const built = finalizeUnits('buildCloseTickArrays', ixs.length * CU_LIMITS.closeTickArray, ixs, main);
  // The packet gate, measured on the WIRE form with no lookup table (the conservative bound). The
  // blockhash is a placeholder: its value never changes a message's size.
  const { bytes } = messageSize(
    buildV0Message({ feePayer: a.rentOwner, instructions: built.instructions, lifetime: { blockhash: SIZE_ONLY_BLOCKHASH, lastValidBlockHeight: 0n } }),
  );
  if (bytes > LEGACY_TX_BYTES) throw new TooManyAccountsError('buildCloseTickArrays', built.addressCount, bytes);
  return built;
}

/**
 * The most closes of ONE pool `buildCloseTickArrays` packs into a transaction with no lookup table —
 * measured through `@solana/kit`'s compile: 304 + 49·N bytes, so 18 closes are 1,186 B and
 * a 19th makes 1,235 B. Each
 * further pool in a batch costs its pool and bitmap keys (64 B), so a mixed batch fits fewer.
 */
export const MAX_CLOSE_TICK_ARRAYS_ONE_POOL = 18;

/** A syntactically valid blockhash for MEASURING a message; never sent. */
const SIZE_ONLY_BLOCKHASH = SYSTEM_PROGRAM_ID as string as Blockhash;

// ---- initialize_pool --------------------------------------------------------------

export interface BuildInitializePoolArgs {
  payer: Address;
  /** `before_initialize`'s sender; defaults to the payer. */
  initializer?: Address;
  mintA: Address;
  mintB: Address;
  tokenProgramA: Address;
  tokenProgramB: Address;
  fee: number;
  tickSpacing: number;
  /** `NO_HOOK` for a plain pool. */
  hook: Address;
  /** Price of the SORTED pool (currency1 per currency0), Q64.64. */
  sqrtPriceX64: bigint;
  // wSOL is the ONE implied quote (`implied_quote`), whose `mint_pools` slot is read-only; every other quote is a ROSTER
  // quote (`list_quote_mint`) whose `mint_pools` slot is WRITABLE like any base mint's (`ConstraintMut` otherwise).
  hookSlice?: readonly AccountMeta[];
  ids?: ProgramIds;
}

export async function buildInitializePool(a: BuildInitializePoolArgs): Promise<BuiltTransaction> {
  const ids = a.ids ?? DEFAULT_PROGRAM_IDS;
  const aFirst = compareBytes(addressToBytes(a.mintA), addressToBytes(a.mintB)) < 0;
  const [mint0, mint1] = aFirst ? [a.mintA, a.mintB] : [a.mintB, a.mintA];
  const [tp0, tp1] = aFirst ? [a.tokenProgramA, a.tokenProgramB] : [a.tokenProgramB, a.tokenProgramA];
  const hooked = !isDefault(a.hook);
  if (hooked && a.hookSlice === undefined) throw new Error('buildInitializePool: a hooked pool needs an explicit hookSlice (the hook SDK provides it)');
  if (!hooked && a.fee === DYNAMIC_FEE_FLAG) throw new Error('buildInitializePool: a hookless pool must have a static fee (v4 rule)');

  const pool = (await findPool({ mint0, mint1, fee: a.fee, tickSpacing: a.tickSpacing, hook: a.hook }, ids)).address;
  const [config, bitmap, vault0, vault1, excl0, excl1, pools0, pools1, indexEntry, ev, quote0, quote1] = await Promise.all([
    findAmmConfig(ids),
    findBitmap(pool, ids),
    findVault(pool, mint0, ids),
    findVault(pool, mint1, ids),
    findMintExclusivity(mint0, ids),
    findMintExclusivity(mint1, ids),
    findMintPools(mint0, ids),
    findMintPools(mint1, ids),
    findPoolIndexEntry(pool, ids),
    findEventAuthority(ids.carpenterAmm),
    //  roster entries. Derived unconditionally: the slots are REQUIRED, and an
    // absent account is how the program reads "not on the roster".
    findQuoteMint(mint0, ids),
    findQuoteMint(mint1, ids),
  ]);
  const hookSlice = a.hookSlice ?? [];
  const main = initializePoolIx(
    {
      program: ids.carpenterAmm,
      eventAuthority: ev.address,
      payer: a.payer,
      initializer: a.initializer ?? a.payer,
      config: config.address,
      pool,
      bitmap: bitmap.address,
      vault0: vault0.address,
      vault1: vault1.address,
      mint0,
      mint1,
      tokenProgram0: tp0,
      tokenProgram1: tp1,
      mintExclusivity0: excl0.address,
      mintExclusivity1: excl1.address,
      mintPools0: pools0.address,
      mintPools1: pools1.address,
      mintPools0Writable: mint0 !== WSOL_MINT,
      mintPools1Writable: mint1 !== WSOL_MINT,
      poolIndexEntry: indexEntry.address,
      approvedHook: hooked ? (await findApprovedHook(a.hook, ids)).address : null,
      hookProgram: hooked ? a.hook : null,
      hookProgramdata: hooked ? (await findProgramData(a.hook)).address : null,
      quoteMint0: quote0.address,
      quoteMint1: quote1.address,
      hookSlice,
    },
    { fee: a.fee, tickSpacing: a.tickSpacing, hook: a.hook, sqrtPriceX64: a.sqrtPriceX64 },
  );
  return finalize('buildInitializePool', 'initializePool', [main], main);
}
