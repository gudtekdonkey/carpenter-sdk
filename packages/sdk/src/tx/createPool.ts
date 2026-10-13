/**
 * `planCreatePool`: create a pool and open its first position as ONE action — two transactions, `[initializePool,
 * addLiquidity]`, signed together with one wallet prompt and sent in order by `sendSolanaTransactions`.
 *
 * The two are separate transactions: together, without a lookup table, they exceed the 1,232-byte packet
 * (`test/create-pool.test.ts`), and the second reads the pool the first creates — so they are sent in order, the second
 * only after the first is confirmed. A wallet that simulates each transaction before signing shows the
 * second as failing (its pool does not exist yet); it lands once the first has.
 *
 * Amounts and decimals are given per INPUT mint (`A` / `B`) and mapped to the pool's sorted order here; the price is of
 * the sorted pool (currency1 per currency0), as `buildInitializePool` takes it.
 *
 * Between the two steps the pool exists and is EMPTY, and a swap against an empty pool moves its price anywhere for
 * free. The deposit then lands at that price, bounded only by `amountAMax` / `amountBMax`. Two defences, both on:
 * - `plan.beforeSend(src)` is a `sendSolanaTransactions` `beforeSend` hook that reads the pool before the deposit is
 *   sent and stops the run (reason `guard`, `PoolPriceMovedError`) when its price moved from the initialized one by
 *   more than `maxPriceMoveBps` (default 0: exact);
 * - the maximum amounts: keep them at what the position needs at the initialized price, plus the slippage you accept.
 *   They are the deposit's only bound once it is sent (a price can still move between the check and the deposit).
 *
 * One wallet signs both steps, so the position's owner and the initializer are the payer: a different `owner` or
 * `initializer` is refused.
 */

import { getBase64EncodedWireTransaction, type AccountMeta, type Address, type BlockhashLifetimeConstraint } from '@solana/kit';

import { addressToBytes, compareBytes } from '../bytes.ts';
import { findPool, findVault, DEFAULT_PROGRAM_IDS } from '../pda.ts';
import type { BeforeSendRequest } from '../send.ts';
import type { AccountSource } from '../source.ts';
import { getSlot0 } from '../stateView.ts';
import { buildInitializePool, buildModifyLiquidity, type BuildInitializePoolArgs } from './builders.ts';
import type { BuiltTransaction } from './common.ts';
import { compileSendable, type LookupTableContents } from './message.ts';

export interface PlanCreatePoolArgs extends BuildInitializePoolArgs {
  /** The position's owner. Must be the payer when given: one wallet signs both steps. */
  owner?: Address;
  decimalsA: number;
  decimalsB: number;
  position: {
    /** The position's salt (the third PDA seed); default 0. */
    salt?: bigint;
    /** Aligned to `tickSpacing`, `tickLower < tickUpper`. */
    tickLower: number;
    tickUpper: number;
    /** The liquidity to add (> 0); `quoteLiquidity` / `liquidityForAmounts` give it from amounts. */
    liquidity: bigint;
    /**
     * The most of each input mint the deposit may take: the deposit's only bound once it is sent. Keep them at what
     * the position needs at the initialized price plus the slippage you accept, never larger.
     */
    amountAMax: bigint;
    amountBMax: bigint;
    /** Set when the owner's token account for that mint has the Token-2022 CPI Guard on. */
    tokenAHasCpiGuard?: boolean;
    tokenBHasCpiGuard?: boolean;
    /** Wrap / unwrap native SOL for a wSOL side (default true). */
    wrapSol?: boolean;
  };
  /** REQUIRED on a hooked pool: the hook's accounts for `modify_liquidity` (the hook's package gives them). */
  modifyHookSlice?: readonly AccountMeta[];
  /** A Token-2022 transfer hook's extra accounts and compute, for a side whose mint carries one. */
  transferHookAccounts?: readonly AccountMeta[];
  transferHookCu?: number;
  /** The recent blockhash both transactions carry. */
  lifetime: BlockhashLifetimeConstraint;
  /** Lookup tables to compress through (must already exist on chain). */
  lookupTables?: readonly LookupTableContents[];
}

export interface PlannedStep {
  readonly name: 'initializePool' | 'addLiquidity';
  readonly built: BuiltTransaction;
  /** The compiled transaction, unsigned, and its wire size. */
  readonly compiled: { readonly transaction: Parameters<typeof getBase64EncodedWireTransaction>[0]; readonly bytes: number };
}

export interface PoolPriceGuardOptions {
  /** How far (in basis points of PRICE) the pool may have moved from the initialized price. Default 0: exact. */
  readonly maxPriceMoveBps?: number;
}

/** The pool's price is not the one it was initialized at (or the pool is missing): the deposit was not sent. */
export class PoolPriceMovedError extends Error {
  constructor(
    readonly pool: Address,
    readonly expectedSqrtPriceX64: bigint,
    readonly actualSqrtPriceX64: bigint | null,
  ) {
    super(
      actualSqrtPriceX64 === null
        ? `pool ${pool} was not found before the deposit`
        : `pool ${pool} moved from sqrt price ${expectedSqrtPriceX64} to ${actualSqrtPriceX64} before the deposit`,
    );
    this.name = 'PoolPriceMovedError';
  }
}

/** Whether `actual` is within `bps` of `expected`, compared as PRICES (the squares of the sqrt prices). */
export function priceWithin(expected: bigint, actual: bigint, bps: number): boolean {
  if (!Number.isInteger(bps) || bps < 0) throw new RangeError(`maxPriceMoveBps ${bps} must be a non-negative integer`);
  const e = expected * expected;
  const d = actual * actual - e;
  return (d < 0n ? -d : d) * 10_000n <= BigInt(bps) * e;
}

export interface CreatePoolPlan {
  readonly pool: Address;
  /** The sqrt price the pool is initialized at. */
  readonly sqrtPriceX64: bigint;
  readonly steps: readonly PlannedStep[];
  /** The base64 wire transactions, in send order — `sendSolanaTransactions(wallet, plan.transactions, rpc, { steps: plan.stepNames })`. */
  readonly transactions: readonly string[];
  readonly stepNames: readonly string[];
  /**
   * The pre-send check for `sendSolanaTransactions` (`opts.beforeSend`): before the deposit (step 1) it reads the pool
   * through `src` and throws `PoolPriceMovedError` when the price moved beyond `maxPriceMoveBps` (default: exact).
   */
  readonly beforeSend: (src: AccountSource, opts?: PoolPriceGuardOptions) => (req: BeforeSendRequest) => Promise<void>;
}

export async function planCreatePool(a: PlanCreatePoolArgs): Promise<CreatePoolPlan> {
  const ids = a.ids ?? DEFAULT_PROGRAM_IDS;
  if (a.owner !== undefined && a.owner !== a.payer) {
    throw new Error('planCreatePool: owner must be the payer (one wallet signs both steps); build a different owner\'s deposit with buildModifyLiquidity');
  }
  if (a.initializer !== undefined && a.initializer !== a.payer) {
    throw new Error('planCreatePool: initializer must be the payer (one wallet signs both steps)');
  }
  const owner = a.payer;
  if (a.position.liquidity <= 0n) throw new Error('planCreatePool: position.liquidity must be > 0');
  const aFirst = compareBytes(addressToBytes(a.mintA), addressToBytes(a.mintB)) < 0;
  const [mint0, mint1] = aFirst ? [a.mintA, a.mintB] : [a.mintB, a.mintA];
  const [tp0, tp1] = aFirst ? [a.tokenProgramA, a.tokenProgramB] : [a.tokenProgramB, a.tokenProgramA];
  const [d0, d1] = aFirst ? [a.decimalsA, a.decimalsB] : [a.decimalsB, a.decimalsA];
  const p = a.position;
  const [max0, max1] = aFirst ? [p.amountAMax, p.amountBMax] : [p.amountBMax, p.amountAMax];
  const [guard0, guard1] = aFirst ? [p.tokenAHasCpiGuard, p.tokenBHasCpiGuard] : [p.tokenBHasCpiGuard, p.tokenAHasCpiGuard];

  const init = await buildInitializePool(a);
  const pool = (await findPool({ mint0, mint1, fee: a.fee, tickSpacing: a.tickSpacing, hook: a.hook }, ids)).address;
  const [vault0, vault1] = await Promise.all([findVault(pool, mint0, ids), findVault(pool, mint1, ids)]);
  const add = await buildModifyLiquidity({
    owner,
    payer: a.payer,
    pool,
    poolState: { mint0, mint1, vault0: vault0.address, vault1: vault1.address, tokenProgram0: tp0, tokenProgram1: tp1, hook: a.hook, decimals0: d0, decimals1: d1, tickSpacing: a.tickSpacing },
    salt: p.salt ?? 0n,
    tickLower: p.tickLower,
    tickUpper: p.tickUpper,
    liquidityDelta: p.liquidity,
    amount0Max: max0,
    amount1Max: max1,
    ...(guard0 ? { token0HasCpiGuard: true } : {}),
    ...(guard1 ? { token1HasCpiGuard: true } : {}),
    ...(p.wrapSol !== undefined ? { wrapSol: p.wrapSol } : {}),
    ...(a.modifyHookSlice ? { hookSlice: a.modifyHookSlice } : {}),
    ...(a.transferHookAccounts ? { transferHookAccounts: a.transferHookAccounts } : {}),
    ...(a.transferHookCu !== undefined ? { transferHookCu: a.transferHookCu } : {}),
    ids,
  });

  const lookupTables = a.lookupTables ?? [];
  const steps: PlannedStep[] = [
    { name: 'initializePool', built: init, compiled: compileSendable('planCreatePool/initializePool', { feePayer: a.payer, instructions: init.instructions, lifetime: a.lifetime, lookupTables }) },
    { name: 'addLiquidity', built: add, compiled: compileSendable('planCreatePool/addLiquidity', { feePayer: a.payer, instructions: add.instructions, lifetime: a.lifetime, lookupTables }) },
  ];
  const sqrtPriceX64 = a.sqrtPriceX64;
  const beforeSend =
    (src: AccountSource, opts: PoolPriceGuardOptions = {}) =>
    async (req: BeforeSendRequest): Promise<void> => {
      if (req.index !== 1) return;
      const bps = opts.maxPriceMoveBps ?? 0;
      const { value } = await getSlot0(src, pool, ids);
      if (!value) throw new PoolPriceMovedError(pool, sqrtPriceX64, null);
      if (!priceWithin(sqrtPriceX64, value.sqrtPriceX64, bps)) throw new PoolPriceMovedError(pool, sqrtPriceX64, value.sqrtPriceX64);
    };
  return {
    pool,
    sqrtPriceX64,
    beforeSend,
    steps,
    transactions: steps.map((s) => getBase64EncodedWireTransaction(s.compiled.transaction)),
    stepNames: steps.map((s) => s.name),
  };
}
