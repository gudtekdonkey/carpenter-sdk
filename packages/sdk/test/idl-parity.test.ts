// The SDK's wire equals the shipped IDL's: for every instruction encoder AND every builder's program instruction, the
// discriminator, the account names in order, each account's writable / signer flags, the argument names in order, and
// the argument bytes (decoded by the IDL's own types, consumed exactly). Borsh carries no field names, so ORDER is the
// whole contract: a reordered pair of same-typed fields is a silent wire break that no round trip of the SDK against
// itself can see.

import assert from 'node:assert/strict';

import { AccountRole, type Address } from '@solana/kit';

import { SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, WSOL_MINT } from '../src/constants.ts';
import { NO_HOOK } from '../src/pda.ts';
import * as builders from '../src/tx/builders.ts';
import * as encoders from '../src/tx/instructions.ts';
import type { ProgramInstruction } from '../src/tx/instructions.ts';
import { addr, decodeIdlArgs, everyInstruction, flatAccounts, readIdl, type IdlInstruction } from './helpers.ts';
import { test } from './node-test.ts';

const idl = readIdl();
const isWritable = (r: AccountRole) => r === AccountRole.WRITABLE || r === AccountRole.WRITABLE_SIGNER;
const isSigner = (r: AccountRole) => r === AccountRole.READONLY_SIGNER || r === AccountRole.WRITABLE_SIGNER;

/**
 * `initialize_pool.mint_pools0/1` carry no `mut` in the program (a quote mint's slot must stay read-only); the handler
 * checks `is_writable` where it writes, and the SDK marks the non-quote mint's slot writable.
 */
const SDK_DECIDES_WRITABLE = new Set(['initialize_pool.mint_pools0', 'initialize_pool.mint_pools1']);

function rowOf(ix: ProgramInstruction): IdlInstruction {
  const row = idl.instructions.find((x) => x.name === ix.name);
  assert.ok(row, `${ix.name} is in the IDL`);
  return row;
}

/** Discriminator, account names / order / flags, arg names, and arg bytes against the IDL. */
function checkAgainstIdl(what: string, ix: ProgramInstruction): Record<string, unknown> {
  const row = rowOf(ix);
  assert.deepEqual([...ix.data.subarray(0, 8)], row.discriminator, `${what}: discriminator`);
  const accounts = flatAccounts(row.accounts);
  assert.deepEqual([...ix.accountNames], accounts.map((a) => a.name), `${what}: account names, in order`);
  assert.ok(ix.accounts.length >= accounts.length, `${what}: remaining accounts follow the named ones`);
  accounts.forEach((a, i) => {
    const meta = ix.accounts[i]!;
    if (a.optional && meta.address === ix.programAddress) {
      assert.equal(meta.role, AccountRole.READONLY, `${what}.${a.name}: an absent optional account is the program id, read-only`);
      return;
    }
    if (!SDK_DECIDES_WRITABLE.has(`${ix.name}.${a.name}`)) assert.equal(isWritable(meta.role), !!a.writable, `${what}.${a.name}: writable`);
    assert.equal(isSigner(meta.role), !!a.signer, `${what}.${a.name}: signer`);
  });
  assert.deepEqual([...ix.argNames], row.args.map((a) => a.name), `${what}: argument names, in order`);
  return decodeIdlArgs(idl, row, ix.data.subarray(8));
}

test('every encoder instance matches the IDL: discriminator, accounts, flags, args', () => {
  const all = everyInstruction();
  for (const [key, ix] of Object.entries(all)) checkAgainstIdl(key, ix);
  assert.equal(Object.keys(all).length, 8);
});

test('every encoder instructions.ts exports has an instance in everyInstruction()', () => {
  const exported = Object.entries(encoders)
    .filter(([n, v]) => typeof v === 'function' && n.endsWith('Ix'))
    .map(([n]) => n)
    .sort();
  const names = Object.values(everyInstruction()).map((ix) => ix.name.replace(/_(\w)/g, (_, c: string) => c.toUpperCase()) + 'Ix').sort();
  assert.deepEqual(exported, names);
});

test('the decoded arguments are the values the encoders were given', () => {
  const all = everyInstruction();
  const swap = checkAgainstIdl('swap', all['carpenter_amm/swap']!);
  assert.deepEqual(swap, { zero_for_one: true, amount_specified: -5n, sqrt_price_limit_x64: 4295048017n, amount_limit: 0n, hook_data: new Uint8Array() });
  const sfb = checkAgainstIdl('swap_from_balance', all['carpenter_amm/swap_from_balance']!);
  assert.deepEqual(sfb, { zero_for_one: true, keep: 7n, sqrt_price_limit_x64: 4295048017n, amount_limit: 3n, limit_in_basis: 11n, hook_data: new Uint8Array() });
  const ml = checkAgainstIdl('modify_liquidity', all['carpenter_amm/modify_liquidity']!);
  assert.equal(ml['liquidity_delta'], 1000n);
  assert.deepEqual(ml['rent_owner'], { rent_owner: addr(18) });
  assert.deepEqual(ml['min_out'], { amount0_min: 0n, amount1_min: 0n });
  assert.deepEqual(checkAgainstIdl('initialize_tick_array', all['carpenter_amm/initialize_tick_array']!), { start_index: -3600, rent_owner: { rent_owner: addr(5) } });
  assert.deepEqual(checkAgainstIdl('transfer_position', all['carpenter_amm/transfer_position']!), { new_owner: addr(6) });
  const ip = checkAgainstIdl('initialize_pool', all['carpenter_amm/initialize_pool']!);
  assert.deepEqual(ip, { fee: 0x800000, tick_spacing: 60, hook: addr(20), sqrt_price_x64: 1n << 64n });
});

const pool = addr(30);
const trader = addr(3);
const poolState = (over: Record<string, unknown> = {}) => ({
  mint0: WSOL_MINT,
  mint1: addr(21),
  vault0: addr(22),
  vault1: addr(23),
  tokenProgram0: SPL_TOKEN_PROGRAM_ID,
  tokenProgram1: TOKEN_2022_PROGRAM_ID,
  hook: NO_HOOK as Address,
  decimals0: 9,
  decimals1: 6,
  tickSpacing: 60,
  ...over,
});
const hooked = poolState({ hook: addr(99) });
const slice = [{ address: addr(98), role: AccountRole.WRITABLE }];

/** One or more calls of every `build*` export whose main instruction is a `carpenter_amm` instruction. */
async function everyBuilder(): Promise<Record<string, ProgramInstruction>> {
  const out: Record<string, ProgramInstruction> = {};
  const put = async (key: string, b: Promise<{ main: ProgramInstruction }>) => {
    out[key] = (await b).main;
  };
  await put('buildSwap/plain-in', builders.buildSwap({ trader, pool, poolState: poolState(), zeroForOne: true, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: [0, 3600] }));
  await put('buildSwap/plain-out', builders.buildSwap({ trader, pool, poolState: poolState(), zeroForOne: false, amountSpecified: 5n, amountLimit: 9n, tickArrayStarts: [0] }));
  await put('buildSwap/hooked', builders.buildSwap({ trader, pool, poolState: hooked, zeroForOne: true, amountSpecified: -5n, amountLimit: 1n, tickArrayStarts: [0], hookSlice: slice, cuUnits: 200_000 }));
  await put('buildSwapFromBalance', builders.buildSwapFromBalance({ trader, pool, poolState: poolState(), zeroForOne: false, keep: 0n, amountLimit: 1n, limitInBasis: 0n, tickArrayStarts: [0] }));
  await put('buildSwapFromBalance/hooked', builders.buildSwapFromBalance({ trader, pool, poolState: hooked, zeroForOne: false, keep: 0n, amountLimit: 1n, limitInBasis: 0n, tickArrayStarts: [0], hookSlice: slice, cuUnits: 200_000 }));
  await put('buildModifyLiquidity/add', builders.buildModifyLiquidity({ owner: trader, pool, poolState: poolState(), salt: 1n, tickLower: -60, tickUpper: 60, liquidityDelta: 10n, amount0Max: 5n, amount1Max: 5n }));
  await put('buildModifyLiquidity/remove', builders.buildModifyLiquidity({ owner: trader, pool, poolState: poolState(), salt: 1n, tickLower: -60, tickUpper: 60, liquidityDelta: -10n, amount0Max: 0n, amount1Max: 0n, amount0Min: 1n }));
  await put('buildModifyLiquidity/hooked', builders.buildModifyLiquidity({ owner: trader, pool, poolState: hooked, salt: 1n, tickLower: -60, tickUpper: 60, liquidityDelta: 10n, amount0Max: 5n, amount1Max: 5n, hookSlice: slice }));
  await put('buildClosePosition', builders.buildClosePosition({ owner: trader, pool, salt: 1n }));
  await put('buildInitializeTickArray', builders.buildInitializeTickArray({ payer: trader, pool, startIndex: -3600 }));
  await put('buildCloseTickArrays', builders.buildCloseTickArrays({ rentOwner: trader, arrays: [{ pool, startIndex: 0 }] }));
  const ip = { payer: trader, mintA: addr(21), mintB: WSOL_MINT, tokenProgramA: TOKEN_2022_PROGRAM_ID, tokenProgramB: SPL_TOKEN_PROGRAM_ID, fee: 3000, tickSpacing: 60, sqrtPriceX64: 1n << 64n };
  await put('buildInitializePool/plain', builders.buildInitializePool({ ...ip, hook: NO_HOOK }));
  await put('buildInitializePool/hooked', builders.buildInitializePool({ ...ip, hook: addr(99), hookSlice: slice }));
  return out;
}

test('every builder`s program instruction matches the IDL: accounts in order, signer and writable flags, args', async () => {
  const built = await everyBuilder();
  for (const [key, ix] of Object.entries(built)) checkAgainstIdl(key, ix);
  // every exported `build*` whose main instruction is carpenter_amm's is covered
  const exported = Object.entries(builders)
    .filter(([n, v]) => typeof v === 'function' && n.startsWith('build'))
    .map(([n]) => n)
    .sort();
  const covered = [...new Set(Object.keys(built).map((k) => k.split('/')[0]!))].sort();
  assert.deepEqual(exported, covered, 'a build* export without an IDL case');
});
