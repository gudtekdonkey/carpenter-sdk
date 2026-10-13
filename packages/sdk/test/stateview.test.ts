// stateView over `memorySource`: owner AND discriminator gate every read, the StateView-analogue
// getters, `positionsForOwner`'s memcmp at account offset 48, `listPools` paging, presence,
// `listLaunches` in both modes keyed by chainKey, and the quoter's byte adapter.

import assert from 'node:assert/strict';
import { test } from './node-test.ts';

import { type Address } from '@solana/kit';

import { readSwapQuoteAccounts } from '../src/adapters.ts';
import {
  addressToBytes,
  compareBytes,

  

  WSOL_MINT,
  encodeAccount,
  encodeTickArray,
  MINT_POOL_COUNT,
  POOL,
  POOL_INDEX_ENTRY,
  POSITION,

  TICK_ARRAY_BITMAP,
  TICK_ARRAY_LEN_V1,
  TICK_ARRAY_LEN_V2,
  TICK_ARRAY_RENT_OWNER_OFFSET,
  findBitmap,
  findMintPools,
  findTickArray,
  memorySource,
  type RawAccount,
  getFeeGrowthInside,
  getLiquidity,

  getMintPoolCount,
  getPool,
  getSlot0,
  getTickArray,
  getTickArraysAround,
  getTickInfo,
  listPools,
  positionsForOwner,
  bitmapBit,
} from '../src/index.ts';
import {
  DEFAULT_PROGRAM_IDS,

  

  

} from '../src/pda.ts';
import { addr,  } from './helpers.ts';

const ids = DEFAULT_PROGRAM_IDS;
const own = (owner: Address, data: Uint8Array): RawAccount => ({ owner, lamports: 1n, executable: false, data });

async function poolFixture() {
  const pool = addr(50);
  const s = 60;
  const words = [0n, 0n, 0n, 0n];
  const mark = (start: number) => {
    const bit = bitmapBit(start, s);
    words[Math.floor(bit / 64)]! |= 1n << BigInt(bit % 64);
  };
  for (const st of [-7200, -3600, 0, 3600, 36_000]) mark(st);
  const accounts = new Map<Address, RawAccount>();
  accounts.set(
    pool,
    own(ids.carpenterAmm, encodeAccount(POOL, { tickSpacing: s, tickCurrent: 10, sqrtPriceX64: 1n << 64n, liquidity: 500n, lpFee: 3000, feeGrowthGlobal0X64: 1000n, feeGrowthGlobal1X64: 2000n, mint0: WSOL_MINT, mint1: addr(51) })),
  );
  accounts.set((await findBitmap(pool, ids)).address, own(ids.carpenterAmm, encodeAccount(TICK_ARRAY_BITMAP, { pool, tickSpacing: s, arraysPerSide: 124 }, words)));
  for (const st of [-7200, -3600, 0, 3600, 36_000]) {
    const ticks = st === 0 ? [{ liquidityGross: 5n, liquidityNet: 5n, feeGrowthOutside0X64: 100n, feeGrowthOutside1X64: 200n }] : [];
    const upper = st === 0 ? { ...ticks[0]!, liquidityNet: -5n, feeGrowthOutside0X64: 300n, feeGrowthOutside1X64: 400n } : undefined;
    const arr = ticks.slice();
    if (upper) arr[1] = upper; // ticks 0 and 60
    accounts.set((await findTickArray(pool, st, ids)).address, own(ids.carpenterAmm, encodeTickArray({ pool, startIndex: st, ticks: arr })));
  }
  return { pool, accounts, s };
}

test('getPool / getSlot0 / getLiquidity check the owner: a lookalike owned elsewhere is null', async () => {
  const { pool, accounts } = await poolFixture();
  const src = memorySource(accounts, 77n);
  const p = await getPool(src, pool);
  assert.equal(p.slot, 77n);
  assert.equal(p.value?.tickCurrent, 10);
  assert.deepEqual((await getSlot0(src, pool)).value, { sqrtPriceX64: 1n << 64n, tick: 10, protocolFee: 0, lpFee: 3000 });
  assert.equal((await getLiquidity(src, pool)).value, 500n);
  const fake = new Map(accounts);
  fake.set(pool, { ...accounts.get(pool)!, owner: addr(1) });
  assert.equal((await getPool(memorySource(fake), pool)).value, null);
  assert.equal((await getPool(src, addr(2))).value, null);
});

test('getTickInfo reads the tick from its array (zeros when absent) and refuses an unaligned tick', async () => {
  const { pool, accounts } = await poolFixture();
  const src = memorySource(accounts);
  assert.equal((await getTickInfo(src, pool, 0, 60)).value.liquidityGross, 5n);
  assert.equal((await getTickInfo(src, pool, 60, 60)).value.liquidityNet, -5n);
  assert.equal((await getTickInfo(src, pool, 120, 60)).value.liquidityGross, 0n, 'uninitialized in a present array');
  assert.equal((await getTickInfo(src, pool, 7200, 60)).value.liquidityGross, 0n, 'array absent → zeros');
  await assert.rejects(getTickInfo(src, pool, 30, 60), /multiple/);
});

test('getTickArraysAround follows the bitmap within the radius and drops a far array', async () => {
  const { pool, accounts } = await poolFixture();
  const r = await getTickArraysAround(memorySource(accounts), pool, 2);
  assert.deepEqual(r.value.map((a) => a.startIndex), [-7200, -3600, 0, 3600]);
  const one = await getTickArraysAround(memorySource(accounts), pool, 0);
  assert.deepEqual(one.value.map((a) => a.startIndex), [0]);
});

test('getFeeGrowthInside: v4 formula, wrapping mod 2^128', async () => {
  const { pool, accounts } = await poolFixture();
  const r = await getFeeGrowthInside(memorySource(accounts), pool, 0, 60);
  // tick 10 is inside [0, 60): below = outside(lower) = 100, above = outside(upper) = 300 → 1000 − 100 − 300
  assert.equal(r.value.feeGrowthInside0X64, 600n);
  assert.equal(r.value.feeGrowthInside1X64, 2000n - 200n - 400n);
  // a range above the current tick: below = global − lower_out; above = upper_out → global − (global − 100) − 300 wraps
  const above = await getFeeGrowthInside(memorySource(accounts), pool, 60, 120);
  const M = 1n << 128n;
  assert.equal(above.value.feeGrowthInside0X64, (((1000n - (1000n - 300n) - 0n) % M) + M) % M);
});

test('positionsForOwner memcmps the owner at account offset 48; listPools pages by (created_slot, pool)', async () => {
  const accounts = new Map<Address, RawAccount>();
  const owner = addr(60);
  for (let i = 0; i < 3; i++) accounts.set(addr(70 + i), own(ids.carpenterAmm, encodeAccount(POSITION, { owner: i === 1 ? addr(61) : owner, pool: addr(50), salt: BigInt(i) })));
  accounts.set(addr(80), own(addr(1), encodeAccount(POSITION, { owner }))); // wrong owner program
  const ps = await positionsForOwner(memorySource(accounts), owner);
  assert.deepEqual(ps.value.map((p) => p.position.salt).sort(), [0n, 2n]);
  for (let i = 0; i < 5; i++) accounts.set(addr(90 + i), own(ids.carpenterAmm, encodeAccount(POOL_INDEX_ENTRY, { pool: addr(100 + (4 - i)), createdSlot: BigInt(i % 2) })));
  const src = memorySource(accounts);
  const page1 = await listPools(src, { limit: 2 });
  assert.equal(page1.value.items.length, 2);
  assert.ok(page1.value.cursor);
  const page2 = await listPools(src, { limit: 2, cursor: page1.value.cursor });
  const page3 = await listPools(src, { limit: 2, cursor: page2.value.cursor });
  assert.equal(page3.value.cursor, null);
  const all = [...page1.value.items, ...page2.value.items, ...page3.value.items];
  assert.equal(all.length, 5);
  for (let i = 1; i < all.length; i++) {
    const a = all[i - 1]!;
    const b = all[i]!;
    assert.ok(a.createdSlot < b.createdSlot || (a.createdSlot === b.createdSlot && compareBytes(addressToBytes(a.pool), addressToBytes(b.pool)) < 0), 'ascending by (created_slot, pool)');
  }
  assert.equal((await listPools(src, { cursor: 'nope' })).value.items.length, 0, 'an unknown cursor is past the end');
});

test('getMintPoolCount is presence: right owner, length and discriminator, or null', async () => {
  const mint = addr(51);
  const pda = (await findMintPools(mint, ids)).address;
  const good = new Map<Address, RawAccount>([[pda, own(ids.carpenterAmm, encodeAccount(MINT_POOL_COUNT, { mint, count: 2n }))]]);
  assert.equal((await getMintPoolCount(memorySource(good), mint)).value?.count, 2n);
  const short = new Map<Address, RawAccount>([[pda, own(ids.carpenterAmm, encodeAccount(MINT_POOL_COUNT, { mint }).subarray(0, 60))]]);
  assert.equal((await getMintPoolCount(memorySource(short), mint)).value, null);
  const lamportsOnly = new Map<Address, RawAccount>([[pda, { owner: ids.carpenterAmm, lamports: 1n, executable: false, data: new Uint8Array(0) }]]);
  assert.equal((await getMintPoolCount(memorySource(lamportsOnly), mint)).value, null, 'lamports never mean presence');
});

// =================================================================================================

// =================================================================================================

/** The fixture with every tick array's tail naming a rent owner — the SAME body, then the 32-byte owner. */
async function ownedPoolFixture(owner: Address) {
  const f = await poolFixture();
  for (const st of [-7200, -3600, 0, 3600, 36_000]) {
    const k = (await findTickArray(f.pool, st, ids)).address;
    const a = f.accounts.get(k)!;
    assert.equal(a.data.length, TICK_ARRAY_LEN_V2);
    const data = a.data.slice();
    data.set(addressToBytes(owner), TICK_ARRAY_RENT_OWNER_OFFSET);
    f.accounts.set(k, { ...a, data });
  }
  return f;
}

test('readSwapQuoteAccounts hands the quoter each array\'s 3,888-byte body and SKIPS a 3,888-byte V1 account', async () => {
  const z = await poolFixture();
  const o = await ownedPoolFixture(addr(88));
  const a = await readSwapQuoteAccounts(memorySource(z.accounts), z.pool, { radius: 1 });
  const b = await readSwapQuoteAccounts(memorySource(o.accounts), o.pool, { radius: 1 });
  assert.deepEqual(b.value.tickArrayStarts, [-3600, 0, 3600], 'every array in the window is used');
  assert.deepEqual(b.value.tickArrayStarts, a.value.tickArrayStarts);
  assert.ok(b.value.tickArrays.every((t) => t.length === TICK_ARRAY_LEN_V1), 'bodies, the quoter contract');
  b.value.tickArrays.forEach((t, i) => assert.deepEqual(t, a.value.tickArrays[i], `array ${i}: the owner tail is never a quote input`));
  // a V1 (3,888-byte) account, and any other length, is skipped — never used, never truncated into a body
  const k0 = (await findTickArray(z.pool, 0, ids)).address;
  for (const len of [TICK_ARRAY_LEN_V1, 3_921]) {
    const odd = new Map(z.accounts);
    const base = z.accounts.get(k0)!;
    odd.set(k0, { ...base, data: len === TICK_ARRAY_LEN_V1 ? base.data.slice(0, len) : new Uint8Array(len) });
    const r = await readSwapQuoteAccounts(memorySource(odd), z.pool, { starts: [-3600, 0, 3600] });
    assert.deepEqual(r.value.tickArrayStarts, [-3600, 3600], `${len} B skipped`);
  }
});

test('getTickArray / getTickInfo / getTickArraysAround / getFeeGrowthInside read 3,920-byte arrays; getTickArray returns the rentOwner', async () => {
  const z = await poolFixture();
  const o = await ownedPoolFixture(addr(88));
  const s1 = memorySource(z.accounts);
  const s2 = memorySource(o.accounts);
  const t2 = (await getTickArray(s2, o.pool, 0)).value;
  assert.equal(t2?.rentOwner, addr(88), 'the tail names its rent owner');
  const t1 = (await getTickArray(s1, z.pool, 0)).value;
  assert.equal(t1?.rentOwner, null, 'a zero tail records none');
  assert.deepEqual({ ...t2, rentOwner: null }, t1, 'the body decodes identically');
  assert.equal((await getTickInfo(s2, o.pool, 60, 60)).value.liquidityNet, -5n);
  assert.deepEqual(
    (await getTickArraysAround(s2, o.pool, 2)).value.map((t) => t.startIndex),
    (await getTickArraysAround(s1, z.pool, 2)).value.map((t) => t.startIndex),
  );
  assert.deepEqual((await getFeeGrowthInside(s2, o.pool, 0, 60)).value, (await getFeeGrowthInside(s1, z.pool, 0, 60)).value);
});

test('the stateView object carries the quote-roster and protocol-treasury reads', async () => {
  const sv = await import('../src/stateView.ts');
  assert.equal(sv.stateView.getQuoteMint, sv.getQuoteMint);
  assert.equal(sv.stateView.getProtocolTreasury, sv.getProtocolTreasury);
  assert.equal((await sv.getQuoteMint(memorySource({}), addr(5))).value, null, 'not on the roster');
  assert.equal((await sv.getProtocolTreasury(memorySource({}))).value, null, 'not initialized');
});
