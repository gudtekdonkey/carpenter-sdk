// planCreatePool: [initializePool, addLiquidity] as one step list for one signature — the pool the first creates is the
// pool the second deposits into, amounts follow the sorted order, and each step is a sendable v0 transaction.

import assert from 'node:assert/strict';
import { blockhash } from '@solana/kit';

import { SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, WSOL_MINT } from '../src/constants.ts';
import { DEFAULT_PROGRAM_IDS, findPool, NO_HOOK } from '../src/pda.ts';
import { getSqrtPriceAtTick } from '../src/tickmath.ts';
import { buildInitializePool, buildModifyLiquidity } from '../src/tx/builders.ts';
import { compileSendable, PacketTooLargeError } from '../src/tx/message.ts';
import { planCreatePool, PoolPriceMovedError, priceWithin } from '../src/tx/createPool.ts';
import { encodeAccount, POOL } from '../src/layouts.ts';
import { memorySource } from '../src/source.ts';
import { addr } from './helpers.ts';
import { test } from './node-test.ts';

const lifetime = { blockhash: blockhash('EtWTRABZaYq6iMfeYKouRu166VU2xqa1Cr7xE9bbG1vv'), lastValidBlockHeight: 1_000n };
const payer = addr(1);
const token = addr(200); // sorts above wSOL (0x06…), so wSOL is currency 0

const args = {
  payer,
  mintA: token,
  mintB: WSOL_MINT,
  tokenProgramA: TOKEN_2022_PROGRAM_ID,
  tokenProgramB: SPL_TOKEN_PROGRAM_ID,
  decimalsA: 6,
  decimalsB: 9,
  fee: 3_000,
  tickSpacing: 60,
  hook: NO_HOOK,
  sqrtPriceX64: getSqrtPriceAtTick(0),
  position: { tickLower: -600, tickUpper: 600, liquidity: 1_000_000n, amountAMax: 5_000n, amountBMax: 7_000n },
  lifetime,
};

test('two steps, in order, for one signature: the first creates the pool the second deposits into', async () => {
  const plan = await planCreatePool(args);
  assert.deepEqual(plan.stepNames, ['initializePool', 'addLiquidity']);
  assert.equal(plan.transactions.length, 2);
  const pool = (await findPool({ mint0: WSOL_MINT, mint1: token, fee: 3_000, tickSpacing: 60, hook: NO_HOOK })).address;
  assert.equal(plan.pool, pool);
  assert.equal(plan.steps[0]!.built.main.accountNames.indexOf('pool') >= 0, true);
  assert.equal(plan.steps[0]!.built.main.accounts[plan.steps[0]!.built.main.accountNames.indexOf('pool')]!.address, pool);
  const add = plan.steps[1]!.built.main;
  assert.equal(add.accounts[add.accountNames.indexOf('pool')]!.address, pool);
  for (const s of plan.steps) assert.ok(s.compiled.bytes <= 1_232, `${s.name} fits the packet`);
});

test('the deposit is the single builders\' output, with A/B mapped to the sorted 0/1', async () => {
  const plan = await planCreatePool(args);
  assert.deepEqual(plan.steps[0]!.built.main.data, (await buildInitializePool(args)).main.data);
  const pool = plan.pool;
  const plain = await buildModifyLiquidity({
    owner: payer, payer, pool,
    poolState: { mint0: WSOL_MINT, mint1: token, vault0: plan.steps[1]!.built.main.accounts[plan.steps[1]!.built.main.accountNames.indexOf('vault0')]!.address, vault1: plan.steps[1]!.built.main.accounts[plan.steps[1]!.built.main.accountNames.indexOf('vault1')]!.address, tokenProgram0: SPL_TOKEN_PROGRAM_ID, tokenProgram1: TOKEN_2022_PROGRAM_ID, hook: NO_HOOK, decimals0: 9, decimals1: 6, tickSpacing: 60 },
    salt: 0n, tickLower: -600, tickUpper: 600, liquidityDelta: 1_000_000n, amount0Max: 7_000n, amount1Max: 5_000n,
  });
  assert.deepEqual(plan.steps[1]!.built.main.data, plain.main.data, 'amount0Max is wSOL\'s (B), amount1Max the token\'s (A)');
});

test('the two do not fit one transaction, which is why they are two steps', async () => {
  const plan = await planCreatePool(args);
  const both = [...plan.steps[0]!.built.instructions, ...plan.steps[1]!.built.instructions];
  assert.throws(() => compileSendable('both', { feePayer: payer, instructions: both, lifetime }), PacketTooLargeError);
});

test('refusals: zero liquidity; a hooked pool without its hook accounts', async () => {
  await assert.rejects(planCreatePool({ ...args, position: { ...args.position, liquidity: 0n } }), /liquidity must be > 0/);
  await assert.rejects(planCreatePool({ ...args, hook: addr(9), fee: 3_000 }), /hookSlice/);
});

test('one wallet signs both steps: an owner or initializer other than the payer is refused', async () => {
  await assert.rejects(planCreatePool({ ...args, owner: addr(2) }), /owner must be the payer/);
  await assert.rejects(planCreatePool({ ...args, initializer: addr(2) }), /initializer must be the payer/);
  const same = await planCreatePool({ ...args, owner: payer, initializer: payer });
  assert.equal(same.transactions.length, 2);
});

test('the pre-send guard reads the pool before the deposit: exact by default, a tolerance in bps of price, a missing pool stops it', async () => {
  const plan = await planCreatePool(args);
  const sqrt = args.sqrtPriceX64;
  const at = (sqrtPriceX64: bigint | null) =>
    memorySource(
      sqrtPriceX64 === null
        ? new Map()
        : new Map([[plan.pool, { owner: DEFAULT_PROGRAM_IDS.carpenterAmm, lamports: 1n, executable: false, data: encodeAccount(POOL, { sqrtPriceX64, tickSpacing: 60, mint0: WSOL_MINT, mint1: token }) }]]),
    );
  const req = (index: number) => ({ index, step: plan.stepNames[index]!, landed: [] });
  // step 0 (the pool's creation) is never checked
  await plan.beforeSend(at(null))(req(0));
  // unchanged: passes
  await plan.beforeSend(at(sqrt))(req(1));
  // pushed by one unit of sqrt price: refused by default, accepted with a tolerance
  const moved = sqrt + (sqrt / 1_000n); // ~0.2% in price
  await assert.rejects(plan.beforeSend(at(moved))(req(1)), (e: unknown) => e instanceof PoolPriceMovedError && e.actualSqrtPriceX64 === moved);
  await assert.rejects(plan.beforeSend(at(sqrt + 1n))(req(1)), PoolPriceMovedError);
  await plan.beforeSend(at(moved), { maxPriceMoveBps: 25 })(req(1));
  await assert.rejects(plan.beforeSend(at(moved), { maxPriceMoveBps: 15 })(req(1)), PoolPriceMovedError);
  await assert.rejects(plan.beforeSend(at(null))(req(1)), /was not found/);
  assert.ok(priceWithin(100n, 100n, 0) && !priceWithin(100n, 101n, 0) && priceWithin(100n, 101n, 201) && !priceWithin(100n, 99n, 198));
});
