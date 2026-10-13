// Transaction builders: every one prepends SetComputeUnitLimit and refuses > 64 accounts; a CPI-Guard source is approved
// immediately before the program instruction and revoked after; an exact-in swap, a removal and a deposit carry a
// slippage bound unless the caller opts out; a wSOL side goes through a temporary account the builder creates and closes
// in the same transaction, so the caller's own wSOL account is never closed.

import assert from 'node:assert/strict';

import { AccountRole, createAddressWithSeed, getProgramDerivedAddress, type Address, type Blockhash, type Instruction } from '@solana/kit';

import { BorshReader } from '../src/borsh.ts';
import { addressToBytes, U64_MAX } from '../src/bytes.ts';
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  COMPUTE_BUDGET_PROGRAM_ID,
  CU_LIMITS,
  DYNAMIC_FEE_FLAG,
  HOOK_FLAGS,
  MAX_ACCOUNT_LOCKS,
  MIN_SQRT_PRICE_X64,
  SPL_TOKEN_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  TOKEN_2022_PROGRAM_ID,
  WSOL_MINT,
} from '../src/constants.ts';
import { DEFAULT_PROGRAM_IDS, findAta, findBitmap, findEventAuthority, findHookSettle, findMintPools, findPosition, findTickArray, NO_HOOK } from '../src/pda.ts';
import {
  buildClosePosition,
  buildCloseTickArrays,
  buildInitializePool,
  buildInitializeTickArray,
  buildModifyLiquidity,
  buildSwap,
  buildSwapFromBalance,
  MAX_CLOSE_TICK_ARRAYS_ONE_POOL,
  TooManyAccountsError,
  uniqueAccounts,
  type BuiltTransaction,
} from '../src/tx/builders.ts';
import { DISC } from '../src/tx/instructions.ts';
import { buildV0Message, messageSize } from '../src/tx/message.ts';
import { TOKEN_ACCOUNT_LEN, TOKEN_ACCOUNT_RENT_LAMPORTS } from '../src/tx/spl.ts';
import { addr } from './helpers.ts';
import { test } from './node-test.ts';

const isCuLimit = (ix: Instruction, units: number) => ix.programAddress === COMPUTE_BUDGET_PROGRAM_ID && ix.data?.[0] === 2 && new DataView(ix.data.buffer, ix.data.byteOffset).getUint32(1, true) === units;
const hasDisc = (ix: Instruction, disc: Uint8Array) => ix.data !== undefined && ix.data.length >= 8 && disc.every((b, i) => ix.data![i] === b);
const dataOf = (ix: Instruction) => new BorshReader(ix.data!.subarray(8));
const ZERO32 = new Array(32).fill(0);

function plainPool(over: Partial<Parameters<typeof buildSwap>[0]['poolState'] & { tickSpacing: number }> = {}) {
  return {
    mint0: addr(20),
    mint1: addr(21),
    vault0: addr(22),
    vault1: addr(23),
    tokenProgram0: SPL_TOKEN_PROGRAM_ID,
    tokenProgram1: TOKEN_2022_PROGRAM_ID,
    hook: NO_HOOK,
    decimals0: 9,
    decimals1: 6,
    tickSpacing: 60,
    ...over,
  };
}

function checkCommon(b: BuiltTransaction, units: number, disc: Uint8Array): void {
  assert.equal(b.cuLimit, units);
  assert.ok(isCuLimit(b.instructions[0]!, units), 'SetComputeUnitLimit first');
  assert.ok(b.instructions.includes(b.main), 'main is in the body');
  assert.ok(hasDisc(b.main, disc), 'main carries its discriminator');
  assert.equal(b.addressCount, b.accounts.length);
  assert.ok(b.addressCount <= MAX_ACCOUNT_LOCKS);
  assert.deepEqual(b.accounts, uniqueAccounts(b.instructions));
}

/** `cu` / `ata` / `sys<tag>` / `tok<tag>` / `main` per instruction. */
const kinds = (b: BuiltTransaction) =>
  b.instructions.map((ix) =>
    ix === b.main
      ? 'main'
      : ix.programAddress === COMPUTE_BUDGET_PROGRAM_ID
        ? 'cu'
        : ix.programAddress === ASSOCIATED_TOKEN_PROGRAM_ID
          ? 'ata'
          : ix.programAddress === SYSTEM_PROGRAM_ID
            ? `sys${new DataView(ix.data!.buffer, ix.data!.byteOffset).getUint32(0, true)}`
            : `tok${ix.data?.[0]}`,
  );
const slotOf = (b: BuiltTransaction, name: string) => b.main.accounts[b.main.accountNames.indexOf(name)]!.address;
/** Every account a `CloseAccount` (Token 9) in the transaction closes. */
const closed = (b: BuiltTransaction) => b.instructions.filter((ix) => ix.programAddress === SPL_TOKEN_PROGRAM_ID && ix.data?.[0] === 9).map((ix) => ix.accounts![0]!.address);
/** The lamports of the transaction's `CreateAccountWithSeed`, and the account it creates. */
function seededCreate(b: BuiltTransaction): { account: Address; lamports: bigint; space: bigint; seed: string } {
  const ix = b.instructions.find((i) => i.programAddress === SYSTEM_PROGRAM_ID && new DataView(i.data!.buffer, i.data!.byteOffset).getUint32(0, true) === 3)!;
  const d = ix.data!;
  const dv = new DataView(d.buffer, d.byteOffset);
  const n = Number(dv.getBigUint64(36, true));
  const at = 44 + n;
  return { account: ix.accounts![1]!.address, seed: new TextDecoder().decode(d.subarray(44, at)), lamports: dv.getBigUint64(at, true), space: dv.getBigUint64(at + 8, true) };
}

const trader = addr(3);

test('buildSwap: a wSOL input goes through a temporary account funded with exactly the input; the trader`s wSOL ATA is never closed', async () => {
  const pool = addr(30);
  const p = plainPool({ mint0: WSOL_MINT });
  const b = await buildSwap({ trader, pool, poolState: p, zeroForOne: true, amountSpecified: -1_000n, amountLimit: 1n, tickArrayStarts: [0, 3600], wsolSeed: 'wsolfixed' });
  checkCommon(b, CU_LIMITS.swapPlain, DISC.amm.swap);
  assert.deepEqual(kinds(b), ['cu', 'ata', 'sys3', 'tok18', 'main', 'tok9']);
  const temp = await createAddressWithSeed({ baseAddress: trader, programAddress: SPL_TOKEN_PROGRAM_ID, seed: 'wsolfixed' });
  const c = seededCreate(b);
  assert.deepEqual(c, { account: temp, seed: 'wsolfixed', lamports: TOKEN_ACCOUNT_RENT_LAMPORTS + 1_000n, space: BigInt(TOKEN_ACCOUNT_LEN) });
  assert.equal(slotOf(b, 'trader_token_in'), temp, 'the swap spends from the temporary account');
  assert.deepEqual(closed(b), [temp], 'only the account this transaction created is closed');
  assert.ok(!closed(b).includes((await findAta(trader, WSOL_MINT, SPL_TOKEN_PROGRAM_ID)).address));
  assert.equal(b.main.accounts.length, 12 + 2 + 2, 'named + event_cpi + two tick arrays, no hook slice');
  assert.equal(b.main.accounts[11]?.address, DEFAULT_PROGRAM_IDS.carpenterAmm, 'hook_program None → the program id');
  // two builds without a seed use two different temporary accounts (unpredictable, so no one can pre-fund one)
  const again = await buildSwap({ trader, pool, poolState: p, zeroForOne: true, amountSpecified: -1_000n, amountLimit: 1n, tickArrayStarts: [] });
  const again2 = await buildSwap({ trader, pool, poolState: p, zeroForOne: true, amountSpecified: -1_000n, amountLimit: 1n, tickArrayStarts: [] });
  assert.notEqual(slotOf(again, 'trader_token_in'), slotOf(again2, 'trader_token_in'));
});

test('buildSwap: a wSOL output is received in a temporary account and closed to SOL; a named wSOL account is topped up, never closed', async () => {
  const pool = addr(30);
  const p = plainPool({ mint0: WSOL_MINT });
  const out = await buildSwap({ trader, pool, poolState: p, zeroForOne: false, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: [] });
  assert.deepEqual(kinds(out), ['cu', 'sys3', 'tok18', 'main', 'tok9']);
  assert.equal(seededCreate(out).lamports, TOKEN_ACCOUNT_RENT_LAMPORTS, 'an output account holds only its rent');
  assert.deepEqual(closed(out), [slotOf(out, 'trader_token_out')]);
  const mine = addr(77);
  const named = await buildSwap({ trader, pool, poolState: p, zeroForOne: true, amountSpecified: -9n, amountLimit: 1n, tickArrayStarts: [], traderTokenIn: mine });
  assert.deepEqual(kinds(named), ['cu', 'ata', 'sys2', 'tok17', 'main'], 'transfer + SyncNative into it, no close');
  assert.equal(slotOf(named, 'trader_token_in'), mine);
  assert.deepEqual(closed(named), []);
  // wrapSol: false uses the trader's wSOL ATA as is
  const raw = await buildSwap({ trader, pool, poolState: p, zeroForOne: true, amountSpecified: -9n, amountLimit: 1n, tickArrayStarts: [], wrapSol: false });
  assert.deepEqual(kinds(raw), ['cu', 'ata', 'main']);
  assert.equal(slotOf(raw, 'trader_token_in'), (await findAta(trader, WSOL_MINT, SPL_TOKEN_PROGRAM_ID)).address);
});

test('buildSwap: CPI Guard approves the pool immediately before and revokes after; the args are the wire', async () => {
  const pool = addr(30);
  const g = await buildSwap({ trader, pool, poolState: plainPool(), zeroForOne: false, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: [], sourceHasCpiGuard: true, wrapSol: false });
  assert.deepEqual(kinds(g), ['cu', 'ata', 'tok13', 'main', 'tok5']);
  assert.equal(g.instructions[2]?.accounts?.[2]?.address, pool, 'delegate = pool');
  const r = dataOf(g.main);
  assert.equal(r.bool(), false);
  assert.equal(r.i128(), -5n);
  r.u128();
  assert.equal(r.u64(), 1n);
  assert.deepEqual([...r.vecU8()], []);
  r.finish();
});

test('buildSwap refusals: no minimum out (unless allowNoMinimum), zero amount, memo-required output, a hooked pool without its slice or CU', async () => {
  const pool = addr(31);
  const base = { trader, pool, poolState: plainPool(), zeroForOne: true, tickArrayStarts: [] as number[] };
  await assert.rejects(buildSwap({ ...base, amountSpecified: -5n, amountLimit: 0n }), /minimum out\) is 0/);
  const open = await buildSwap({ ...base, amountSpecified: -5n, amountLimit: 0n, allowNoMinimum: true });
  assert.ok(hasDisc(open.main, DISC.amm.swap));
  // exact-out's limit is the maximum IN: 0 is not a missing minimum
  await buildSwap({ ...base, amountSpecified: 5n, amountLimit: 7n });
  await assert.rejects(buildSwap({ ...base, amountSpecified: 0n, amountLimit: 1n }), /AmountZero/);
  await assert.rejects(buildSwap({ ...base, amountSpecified: -5n, amountLimit: 1n, destinationRequiresMemo: true }), /MemoTransfer, TokenError::NoMemo/);
  const hooked = plainPool({ hook: addr(99) });
  await assert.rejects(buildSwap({ ...base, poolState: hooked, amountSpecified: -5n, amountLimit: 1n }), /hookSlice/);
  await assert.rejects(buildSwap({ ...base, poolState: hooked, amountSpecified: -5n, amountLimit: 1n, hookSlice: [] }), /cuUnits/);
  const slice = [{ address: addr(98), role: AccountRole.WRITABLE }];
  const h = await buildSwap({ ...base, poolState: hooked, amountSpecified: -5n, amountLimit: 1n, hookSlice: slice, cuUnits: 250_000, tickArrayStarts: [0] });
  assert.equal(h.cuLimit, 250_000);
  const named = h.main.accountNames.length;
  assert.equal(h.main.accounts[11]?.address, addr(99), 'hook_program is the pool`s hook');
  assert.deepEqual(h.main.accounts[named], slice[0], 'the hook slice first after the named accounts');
  assert.equal(h.main.accounts[named + 1]?.address, (await findTickArray(pool, 0)).address, 'then the tick arrays');
});

test('a settling hook: findHookSettle is ["hook_settle", pool] under the AMM, first in the slice; writableMints upgrades only the named mints', async () => {
  const pool = addr(32);
  const settle = await findHookSettle(pool);
  const raw = await getProgramDerivedAddress({ programAddress: DEFAULT_PROGRAM_IDS.carpenterAmm, seeds: [new TextEncoder().encode('hook_settle'), addressToBytes(pool)] });
  assert.equal(settle.address, raw[0]);
  assert.equal(HOOK_FLAGS.SETTLES, 0x8000, 'the last bit');
  const p = plainPool({ hook: addr(99) });
  const slice = [{ address: settle.address, role: AccountRole.READONLY }];
  const base = { trader, pool, poolState: p, zeroForOne: true, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: [] as number[], hookSlice: slice, cuUnits: 300_000 };
  const plain = await buildSwap(base);
  const role = (b: { main: Instruction }, i: number) => b.main.accounts?.[i]?.role;
  assert.equal(plain.main.accounts?.[7]?.address, p.mint0);
  assert.equal(plain.main.accounts?.[8]?.address, p.mint1);
  assert.deepEqual([role(plain, 7), role(plain, 8)], [AccountRole.READONLY, AccountRole.READONLY], 'read-only by default');
  const one = await buildSwap({ ...base, writableMints: [false, true] });
  assert.deepEqual([role(one, 7), role(one, 8)], [AccountRole.READONLY, AccountRole.WRITABLE]);
  assert.equal(one.main.accounts?.[plain.main.accountNames.length]?.address, settle.address, 'the settle signer leads the slice');
  const fb = await buildSwapFromBalance({ ...base, keep: 0n, limitInBasis: 0n, writableMints: [true, false] });
  assert.deepEqual([role(fb, 7), role(fb, 8)], [AccountRole.WRITABLE, AccountRole.READONLY]);
});

test('buildSwap: ensureTickArrayStarts prepends an idempotent initialize_tick_array per array, each in tickArrayStarts', async () => {
  const pool = addr(32);
  const b = await buildSwap({ trader, pool, poolState: plainPool(), zeroForOne: true, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: [0, 3600], ensureTickArrayStarts: [3600, 3600] });
  assert.equal(b.cuLimit, CU_LIMITS.swapPlain + CU_LIMITS.initializeTickArray);
  const inits = b.instructions.filter((ix) => hasDisc(ix, DISC.amm.initialize_tick_array));
  assert.equal(inits.length, 1, 'deduplicated');
  assert.equal(dataOf(inits[0]!).i32(), 3600);
  await assert.rejects(buildSwap({ trader, pool, poolState: plainPool(), zeroForOne: true, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: [0], ensureTickArrayStarts: [3600] }), /not in tickArrayStarts/);
});

test('buildSwapFromBalance: swap`s accounts, its own args; never wraps the input; wSOL out through a temporary account; CPI Guard u64::MAX; refusals', async () => {
  const pool = addr(30);
  const pIn = plainPool({ mint0: WSOL_MINT });
  const a = await buildSwapFromBalance({ trader, pool, poolState: pIn, zeroForOne: true, keep: 1_000n, amountLimit: 7n, limitInBasis: 9n, tickArrayStarts: [0, 3600] });
  checkCommon(a, CU_LIMITS.swapPlain, DISC.amm.swap_from_balance);
  assert.deepEqual(kinds(a), ['cu', 'ata', 'main'], 'no wrap, no unwrap');
  const s = await buildSwap({ trader, pool, poolState: pIn, zeroForOne: true, amountSpecified: -1_000n, amountLimit: 7n, tickArrayStarts: [0, 3600], wrapSol: false });
  assert.deepEqual(a.main.accounts, s.main.accounts, 'the same Swap accounts struct');
  assert.deepEqual(a.main.accountNames, s.main.accountNames);
  assert.equal(a.main.data.length, 53);
  const r = dataOf(a.main);
  assert.equal(r.bool(), true);
  assert.equal(r.u64(), 1_000n, 'keep');
  assert.equal(r.u128(), MIN_SQRT_PRICE_X64 + 1n, 'the widest limit by default');
  assert.equal(r.u64(), 7n, 'amount_limit');
  assert.equal(r.u64(), 9n, 'limit_in_basis');
  assert.deepEqual([...r.vecU8()], []);
  r.finish();

  // a wSOL OUTPUT: a temporary account, closed; `unwrapOut: false` keeps the ATA; a named account is never closed
  const out = await buildSwapFromBalance({ trader, pool, poolState: pIn, zeroForOne: false, keep: 0n, amountLimit: 1n, limitInBasis: 0n, tickArrayStarts: [] });
  assert.deepEqual(kinds(out), ['cu', 'sys3', 'tok18', 'main', 'tok9']);
  assert.deepEqual(closed(out), [slotOf(out, 'trader_token_out')]);
  const kept = await buildSwapFromBalance({ trader, pool, poolState: pIn, zeroForOne: false, keep: 0n, amountLimit: 1n, limitInBasis: 0n, tickArrayStarts: [], unwrapOut: false });
  assert.deepEqual(kinds(kept), ['cu', 'ata', 'main']);
  const named = await buildSwapFromBalance({ trader, pool, poolState: pIn, zeroForOne: false, keep: 0n, amountLimit: 1n, limitInBasis: 0n, tickArrayStarts: [], traderTokenOut: addr(77) });
  assert.deepEqual(closed(named), []);

  const g = await buildSwapFromBalance({ trader, pool, poolState: plainPool(), zeroForOne: false, keep: 5n, amountLimit: 1n, limitInBasis: 0n, tickArrayStarts: [], sourceHasCpiGuard: true });
  assert.deepEqual(kinds(g), ['cu', 'ata', 'tok13', 'main', 'tok5']);
  const approve = g.instructions[2]!;
  assert.equal(new DataView(approve.data!.buffer, approve.data!.byteOffset).getBigUint64(1, true), U64_MAX, 'approve u64::MAX');

  const base = { trader, pool, poolState: pIn, zeroForOne: true, tickArrayStarts: [] as number[] };
  await assert.rejects(buildSwapFromBalance({ ...base, keep: -1n, amountLimit: 1n, limitInBasis: 0n }), /keep -1 is outside u64/);
  await assert.rejects(buildSwapFromBalance({ ...base, keep: 1n << 64n, amountLimit: 1n, limitInBasis: 0n }), /outside u64/);
  await assert.rejects(buildSwapFromBalance({ ...base, keep: 0n, amountLimit: 1n, limitInBasis: -1n }), /limitInBasis -1 is outside u64/);
  await assert.rejects(buildSwapFromBalance({ ...base, keep: 0n, amountLimit: 0n, limitInBasis: 0n }), /minimum out\) is 0/);
  await buildSwapFromBalance({ ...base, keep: 0n, amountLimit: 0n, limitInBasis: 0n, allowNoMinimum: true });
  await assert.rejects(buildSwapFromBalance({ ...base, keep: 0n, amountLimit: 1n, limitInBasis: 0n, destinationRequiresMemo: true }), /MemoTransfer/);
  await assert.rejects(buildSwapFromBalance({ ...base, poolState: plainPool({ hook: addr(99) }), keep: 0n, amountLimit: 1n, limitInBasis: 0n }), /hookSlice/);
});

test('buildSwapFromBalance after buildSwap: a wSOL intermediate survives leg 1 with wrapSol: false', async () => {
  const intoWsol = plainPool({ mint1: WSOL_MINT, tokenProgram1: SPL_TOKEN_PROGRAM_ID });
  const outOfWsol = plainPool({ mint0: WSOL_MINT, tokenProgram0: SPL_TOKEN_PROGRAM_ID, mint1: addr(24) });
  const leg2 = await buildSwapFromBalance({ trader, pool: addr(31), poolState: outOfWsol, zeroForOne: true, keep: 0n, amountLimit: 1n, limitInBasis: 0n, tickArrayStarts: [] });
  const seam = slotOf(leg2, 'trader_token_in');
  const leg1 = await buildSwap({ trader, pool: addr(30), poolState: intoWsol, zeroForOne: true, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: [], wrapSol: false });
  assert.equal(slotOf(leg1, 'trader_token_out'), seam, 'leg 1 pays into the account leg 2 reads');
  assert.deepEqual(closed(leg1), []);
  assert.deepEqual(closed(leg2), []);
});

test('too many accounts throws TooManyAccountsError by count', async () => {
  const many = Array.from({ length: 70 }, (_, i) => i * 3600);
  await assert.rejects(
    buildSwap({ trader, pool: addr(30), poolState: plainPool(), zeroForOne: true, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: many }),
    (e: unknown) => e instanceof TooManyAccountsError && e.count > MAX_ACCOUNT_LOCKS,
  );
});

test('buildModifyLiquidity: alignment, a temporary wSOL account funded with the maximum on add, Approve/Revoke per guarded side, the wire', async () => {
  const pool = addr(32);
  const p = plainPool({ mint0: WSOL_MINT });
  const base = { owner: trader, pool, poolState: p, salt: 1n };
  await assert.rejects(buildModifyLiquidity({ ...base, tickLower: -61, tickUpper: 60, liquidityDelta: 1n, amount0Max: 1n, amount1Max: 1n }), /TickInvalid/);
  await assert.rejects(buildModifyLiquidity({ ...base, tickLower: 60, tickUpper: 60, liquidityDelta: 1n, amount0Max: 1n, amount1Max: 1n }), /TickInvalid/);
  const add = await buildModifyLiquidity({ ...base, tickLower: -60, tickUpper: 60, liquidityDelta: 10n, amount0Max: 100n, amount1Max: 200n, token1HasCpiGuard: true });
  const mlCu = CU_LIMITS.modifyLiquidity + 2 * CU_LIMITS.ataCreate;
  checkCommon(add, mlCu, DISC.amm.modify_liquidity);
  assert.deepEqual(kinds(add), ['cu', 'sys3', 'tok18', 'ata', 'tok13', 'main', 'tok5', 'tok9']);
  assert.equal(seededCreate(add).lamports, TOKEN_ACCOUNT_RENT_LAMPORTS + 100n);
  assert.deepEqual(closed(add), [slotOf(add, 'owner_token0')]);
  assert.ok(!closed(add).includes((await findAta(trader, WSOL_MINT, SPL_TOKEN_PROGRAM_ID)).address));
  assert.equal(slotOf(add, 'position'), (await findPosition(pool, trader, 1n)).address, 'the owner is its own position id');
  const remove = await buildModifyLiquidity({ ...base, tickLower: -60, tickUpper: 60, liquidityDelta: -10n, amount0Max: 0n, amount1Max: 0n, amount0Min: 5n, token1HasCpiGuard: true });
  assert.deepEqual(kinds(remove), ['cu', 'sys3', 'tok18', 'ata', 'main', 'tok9'], 'a removal: no approval, the temporary account holds only rent');
  assert.equal(seededCreate(remove).lamports, TOKEN_ACCOUNT_RENT_LAMPORTS);
  const r = dataOf(add.main);
  assert.deepEqual([...r.fixed(32)], [...addressToBytes(trader)]);
  assert.equal(r.u64(), 1n);
  assert.equal(r.i32(), -60);
  assert.equal(r.i32(), 60);
  assert.equal(r.i128(), 10n);
  assert.equal(r.u64(), 100n);
  assert.equal(r.u64(), 200n);
  r.vecU8();
  assert.equal(r.pubkey(), SYSTEM_PROGRAM_ID, 'rent_owner: the zero key = the payer');
  assert.equal(r.u64(), 0n);
  assert.equal(r.u64(), 0n);
  r.finish();
  // a named wSOL account is topped up for a deposit and never closed
  const named = await buildModifyLiquidity({ ...base, tickLower: -60, tickUpper: 60, liquidityDelta: 10n, amount0Max: 100n, amount1Max: 1n, ownerToken0: addr(77) });
  assert.deepEqual(kinds(named), ['cu', 'ata', 'ata', 'sys2', 'tok17', 'main']);
  assert.deepEqual(closed(named), []);
});

test('buildModifyLiquidity slippage: a removal needs a minimum, a deposit a bounded maximum, unless allowNoMinimum; a collect needs neither', async () => {
  const base = { owner: trader, pool: addr(33), poolState: plainPool(), salt: 0n, tickLower: -60, tickUpper: 60 };
  await assert.rejects(buildModifyLiquidity({ ...base, liquidityDelta: -10n, amount0Max: 0n, amount1Max: 0n }), /removal with no minimum/);
  await assert.rejects(buildModifyLiquidity({ ...base, liquidityDelta: -10n, amount0Max: 0n, amount1Max: 0n, amount0Min: 0n, amount1Min: 0n }), /removal with no minimum/);
  await buildModifyLiquidity({ ...base, liquidityDelta: -10n, amount0Max: 0n, amount1Max: 0n, amount1Min: 1n });
  await buildModifyLiquidity({ ...base, liquidityDelta: -10n, amount0Max: 0n, amount1Max: 0n, allowNoMinimum: true });
  await assert.rejects(buildModifyLiquidity({ ...base, liquidityDelta: 10n, amount0Max: U64_MAX, amount1Max: 1n }), /no bound/);
  await buildModifyLiquidity({ ...base, liquidityDelta: 10n, amount0Max: U64_MAX, amount1Max: 1n, allowNoMinimum: true });
  await buildModifyLiquidity({ ...base, liquidityDelta: 0n, amount0Max: 0n, amount1Max: 0n });
});

test('buildClosePosition, buildInitializeTickArray: the position and array PDAs, the rent owner', async () => {
  const pool = addr(60);
  const cp = await buildClosePosition({ owner: trader, pool, salt: 1n });
  checkCommon(cp, CU_LIMITS.closePosition, DISC.amm.close_position);
  assert.deepEqual(cp.main.accountNames, ['owner', 'position', 'event_authority', 'program']);
  assert.equal(slotOf(cp, 'position'), (await findPosition(pool, trader, 1n)).address);
  assert.equal(cp.main.data.length, 8);
  const dflt = await buildInitializeTickArray({ payer: trader, pool, startIndex: -3600 });
  checkCommon(dflt, CU_LIMITS.initializeTickArray, DISC.amm.initialize_tick_array);
  assert.equal(dflt.main.data.length, 44, 'disc 8 + start_index 4 + rent_owner 32');
  assert.deepEqual([...dflt.main.data.subarray(12)], ZERO32, 'omitted = zero = "record the payer"');
  const named = await buildInitializeTickArray({ payer: trader, pool, startIndex: -3600, rentOwner: addr(9) });
  const r = dataOf(named.main);
  assert.equal(r.i32(), -3600);
  assert.equal(r.pubkey(), addr(9));
  r.finish();
  assert.deepEqual(named.main.accountNames, ['payer', 'pool', 'tick_array', 'system_program']);
  assert.equal(named.main.accounts[2]?.address, (await findTickArray(pool, -3600)).address);
});

test('buildCloseTickArrays: accounts and roles; packs N closes under the packet and throws over it; duplicates refused', async () => {
  const pool = addr(63);
  const one = await buildCloseTickArrays({ rentOwner: trader, arrays: [{ pool, startIndex: 3600 }] });
  checkCommon(one, CU_LIMITS.closeTickArray, DISC.amm.close_tick_array);
  assert.deepEqual([...one.main.data], [...DISC.amm.close_tick_array]);
  const ev = (await findEventAuthority(DEFAULT_PROGRAM_IDS.carpenterAmm)).address;
  assert.deepEqual(
    one.main.accounts.map((m) => [m.address, m.role]),
    [
      [trader, AccountRole.WRITABLE_SIGNER],
      [pool, AccountRole.READONLY],
      [(await findBitmap(pool)).address, AccountRole.READONLY],
      [(await findTickArray(pool, 3600)).address, AccountRole.WRITABLE],
      [ev, AccountRole.READONLY],
      [DEFAULT_PROGRAM_IDS.carpenterAmm, AccountRole.READONLY],
    ],
  );
  const lifetime = { blockhash: SYSTEM_PROGRAM_ID as string as Blockhash, lastValidBlockHeight: 0n };
  const arrays = (n: number, p = pool) => Array.from({ length: n }, (_, i) => ({ pool: p, startIndex: (i - 9) * 3600 }));
  const n = MAX_CLOSE_TICK_ARRAYS_ONE_POOL;
  const full = await buildCloseTickArrays({ rentOwner: trader, arrays: arrays(n) });
  assert.equal(full.instructions.length, 1 + n);
  assert.equal(full.cuLimit, n * CU_LIMITS.closeTickArray);
  assert.ok(messageSize(buildV0Message({ feePayer: trader, instructions: full.instructions, lifetime })).bytes <= 1_232);
  await assert.rejects(buildCloseTickArrays({ rentOwner: trader, arrays: arrays(n + 1) }), (e: unknown) => e instanceof TooManyAccountsError && e.packetBytes !== null && e.packetBytes > 1_232);
  await assert.rejects(buildCloseTickArrays({ rentOwner: trader, arrays: [...arrays(n - 1), ...arrays(1, addr(65))] }), TooManyAccountsError);
  await assert.rejects(buildCloseTickArrays({ rentOwner: trader, arrays: [] }), /no arrays/);
  await assert.rejects(buildCloseTickArrays({ rentOwner: trader, arrays: [{ pool, startIndex: 0 }, { pool, startIndex: 0 }] }), /named twice/);
});

test('buildInitializePool sorts the mints, marks a quote mint`s mint_pools read-only, refuses a hookless dynamic fee and a hooked pool without its slice', async () => {
  const args = { payer: trader, mintA: addr(21), mintB: WSOL_MINT, tokenProgramA: TOKEN_2022_PROGRAM_ID, tokenProgramB: SPL_TOKEN_PROGRAM_ID, fee: 3000, tickSpacing: 60, hook: NO_HOOK, sqrtPriceX64: 1n << 64n };
  const b = await buildInitializePool(args);
  checkCommon(b, CU_LIMITS.initializePool, DISC.amm.initialize_pool);
  assert.equal(b.main.accounts[7]?.address, WSOL_MINT, 'wSOL sorts first');
  assert.equal(b.main.accounts[8]?.address, addr(21));
  assert.equal(b.main.accounts[9]?.address, SPL_TOKEN_PROGRAM_ID, 'token programs follow their mints');
  assert.equal(b.main.accounts[13]?.address, (await findMintPools(WSOL_MINT)).address);
  assert.equal(b.main.accounts[13]?.role, AccountRole.READONLY, 'wSOL is the implied quote: its slot is never written');
  assert.equal(b.main.accounts[14]?.role, AccountRole.WRITABLE);
  assert.equal(b.main.accounts[16]?.address, DEFAULT_PROGRAM_IDS.carpenterAmm, 'approved_hook None');
  await assert.rejects(buildInitializePool({ ...args, fee: DYNAMIC_FEE_FLAG }), /static fee/);
  await assert.rejects(buildInitializePool({ ...args, hook: addr(99) }), /hookSlice/);
  const hooked = await buildInitializePool({ ...args, hook: addr(99), fee: DYNAMIC_FEE_FLAG, hookSlice: [{ address: addr(98), role: AccountRole.WRITABLE }] });
  assert.equal(hooked.main.accounts[17]?.address, addr(99));
  assert.equal(hooked.main.accounts.length, 24 + 1, 'the named accounts, event_cpi, then the slice');
});
