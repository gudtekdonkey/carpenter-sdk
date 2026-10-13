// Account decoding: round trips through `encodeAccount`, strict length and discriminator checks,
// the u128-as-two-LE-limbs rule, and the bitmap's spacing-derived length.

import assert from 'node:assert/strict';
import { test } from './node-test.ts';

import {
  addressToBytes,

  readUintLE,
  SYSTEM_PROGRAM_ID,
  AccountDecodeError,
  AMM_CONFIG,
  decodeAccount,
  decodeAmmConfig,
  decodePool,
  decodePosition,
  decodeTickArray,
  decodeTickArrayBitmap,
  encodeAccount,
  encodeTickArray,
  hasDiscriminator,
  kindSize,
  POOL,
  poolPausedAt,
  POSITION,
  TICK_ARRAY,
  TICK_ARRAY_BITMAP,
  TICK_ARRAY_LEN_V1,
  TICK_ARRAY_LEN_V2,
  TICK_ARRAY_RENT_OWNER_OFFSET,
  tickArrayRentOwner,
  type Pool,
  bitmapAccountLen,
} from '../src/index.ts';
import {

  

  

  

  

  

  

  LAYOUTS,

  

  
} from '../src/layouts.ts';
import {

  

  
} from '../src/constants.ts';
import { addr } from './helpers.ts';

test('Pool: u128 fields are little-endian at their struct offsets (+8), and decode round-trips', () => {
  const max = 79_226_673_521_066_979_257_578_248_091n;
  const bytes = encodeAccount(POOL, {
    version: 1,
    tickSpacing: 60,
    fee: 0x800000,
    tickCurrent: -69081,
    mint0: addr(1),
    mint1: addr(2),
    hook: addr(3),
    sqrtPriceX64: max,
    liquidity: (1n << 100n) + 5n,
    hookClaims1: 0xdeadbeefn,
    swapCount: 42n,
    decimals0: 9,
    decimals1: 6,
  });
  assert.equal(bytes.length, 640);
  assert.equal(readUintLE(bytes, 8 + 280, 16), max);
  assert.equal(readUintLE(bytes, 8 + 368, 8), 0xdeadbeefn);
  const p = decodePool(bytes);
  assert.equal(p.sqrtPriceX64, max);
  assert.equal(p.liquidity, (1n << 100n) + 5n);
  assert.equal(p.tickCurrent, -69081);
  assert.equal(p.fee, 0x800000);
  assert.equal(p.mint1, addr(2));
  assert.equal(p.decimals1, 6);
  assert.equal(p.swapCount, 42n);
  assert.equal((p as Pool & { pad0?: unknown }).pad0, undefined, 'padding is not surfaced');
});

test('every layout round-trips zero and rejects a wrong length or discriminator', () => {
  for (const l of LAYOUTS) {
    const zero = encodeAccount(l, {});
    assert.equal(zero.length, 8 + l.size, l.name);
    assert.ok(hasDiscriminator(l, zero));
    if (!l.trailing) {
      decodeAccount(l, zero);
      assert.throws(() => decodeAccount(l, zero.subarray(0, zero.length - 1)), AccountDecodeError, `${l.name} short`);
      assert.throws(() => decodeAccount(l, new Uint8Array(zero.length + 1)), AccountDecodeError, `${l.name} long`);
    }
    const bad = zero.slice();
    bad[0] = (bad[0] ?? 0) ^ 0xff;
    assert.throws(() => decodeAccount(l, bad), /discriminator/, `${l.name} discriminator`);
    assert.equal(hasDiscriminator(l, bad), false);
  }
});

test('i32 and i128 fields carry their sign', () => {
  const t = encodeTickArray({
    pool: addr(9),
    startIndex: -443_640,
    initializedTickCount: 2,
    ticks: [{ liquidityGross: 7n, liquidityNet: -7n }, { liquidityNet: (1n << 127n) - 1n }],
  });
  const d = decodeTickArray(t);
  assert.equal(d.startIndex, -443_640);
  assert.equal(d.ticks.length, 60);
  assert.equal(d.ticks[0]?.liquidityNet, -7n);
  assert.equal(d.ticks[0]?.liquidityGross, 7n);
  assert.equal(d.ticks[1]?.liquidityNet, (1n << 127n) - 1n);
  assert.equal(d.ticks[59]?.liquidityNet, 0n);
  const p = decodePosition(encodeAccount(POSITION, { owner: addr(4), tickLower: -60, tickUpper: 60, salt: 100n }));
  assert.equal(p.tickLower, -60);
  assert.equal(p.salt, 100n);
  assert.equal(p.owner, addr(4));
});

test('the bitmap is the 40-byte header plus exactly N words for its spacing', () => {
  for (const s of [1, 10, 60, 200]) {
    const perSide = Math.ceil(443_636 / (60 * s));
    const n = Math.ceil((2 * perSide + 1) / 64);
    const words = Array.from({ length: n }, (_, i) => (BigInt(i + 1) * 0x9e3779b97f4a7c15n) & ((1n << 64n) - 1n));
    const bytes = encodeAccount(TICK_ARRAY_BITMAP, { pool: addr(1), tickSpacing: s, arraysPerSide: perSide }, words);
    assert.equal(bytes.length, bitmapAccountLen(s), `spacing ${s}`);
    const bm = decodeTickArrayBitmap(bytes);
    assert.equal(bm.words.length, n);
    assert.deepEqual(bm.words, words);
    assert.equal(bm.arraysPerSide, perSide);
    // one word too few / a lying header
    assert.throws(() => decodeTickArrayBitmap(bytes.subarray(0, bytes.length - 8)), AccountDecodeError);
    assert.throws(() => decodeTickArrayBitmap(encodeAccount(TICK_ARRAY_BITMAP, { tickSpacing: s, arraysPerSide: perSide + 1 }, words)), AccountDecodeError);
  }
  assert.equal(bitmapAccountLen(60), 80);
  assert.equal(bitmapAccountLen(1), 1_904);
});

// `pause_guardian` @104, then `pad1`, 40 bytes of padding @136; `quote_caps_admitted` @176.
test('AmmConfig: pause_guardian @104 then 40 bytes of padding; quote_caps_admitted stays @176', () => {
  const f = AMM_CONFIG.fields.find((x) => x.name === 'pad1');
  assert.ok(f, 'AmmConfig.pad1 exists');
  assert.deepEqual([f.offset, kindSize(f.kind)], [136, 40]);
  assert.equal(AMM_CONFIG.fields.find((x) => x.name === 'pause_guardian')?.offset, 104);
  assert.equal(AMM_CONFIG.fields.find((x) => x.name === 'quote_caps_admitted')?.offset, 176);
  assert.ok(!AMM_CONFIG.fields.some((x) => x.name === 'quote_mints' || x.name === 'quote_mint_count'));
  const c = decodeAmmConfig(encodeAccount(AMM_CONFIG, { authority: addr(1), pauseGuardian: addr(2), quoteCapsAdmitted: 5 }));
  assert.equal(c.authority, addr(1));
  assert.equal(c.pauseGuardian, addr(2));
  assert.equal(c.quoteCapsAdmitted, 5);
  assert.ok(!('quoteMints' in c), 'no quote array on the decode');
});

test('a pool is paused by its authority until unpaused, by its guardian until guardian_paused_until', () => {
  const p = decodePool(encodeAccount(POOL, { paused: 0, guardianPausedUntil: 1_000n }));
  assert.equal(p.guardianPausedUntil, 1_000n);
  assert.equal(POOL.fields.find((x) => x.name === 'guardian_paused_until')?.offset, 624);
  assert.equal(poolPausedAt(p, 999n), true, 'one second before the lapse');
  assert.equal(poolPausedAt(p, 1_000n), false, 'the guardian pause lapses AT paused_until');
  assert.equal(poolPausedAt({ paused: 1, guardianPausedUntil: 0n }, 10n ** 12n), true, 'the authority pause never lapses');
  assert.equal(poolPausedAt({ paused: 0, guardianPausedUntil: 0n }, 0n), false);
});

// =================================================================================================

// =================================================================================================

const TA_FIELDS = {
  pool: addr(9),
  startIndex: -3600,
  initializedTickCount: 1,
  ticks: [{ liquidityGross: 5n, liquidityNet: -5n, feeGrowthOutside0X64: 11n }],
};

test('the two tick-array lengths: V1 3,888 (the body), V2 3,920', () => {
  assert.equal(TICK_ARRAY_LEN_V1, 3_888);
  assert.equal(TICK_ARRAY_LEN_V1, 8 + TICK_ARRAY.size);
  assert.equal(TICK_ARRAY_RENT_OWNER_OFFSET, 3_888);
  assert.equal(TICK_ARRAY_LEN_V2, 3_920);
  assert.equal(TICK_ARRAY.size, 3_880, 'the STRUCT excludes the rent owner');
});

test('decodeTickArray: the 3,920-byte account reads its rent owner; a zero tail reads null; 3,888 (V1) is refused', () => {
  const v2 = encodeTickArray(TA_FIELDS, addr(77));
  assert.equal(v2.length, 3_920);
  assert.deepEqual(v2.subarray(0, 3_888), encodeAccount(TICK_ARRAY, TA_FIELDS), 'the body, then the tail');
  assert.deepEqual(v2.subarray(3_888), addressToBytes(addr(77)));
  const d2 = decodeTickArray(v2);
  assert.equal(d2.rentOwner, addr(77));
  assert.equal(d2.startIndex, -3600);
  assert.equal(d2.ticks[0]?.liquidityNet, -5n);
  assert.equal(tickArrayRentOwner(v2), addr(77));

  const zero = encodeTickArray(TA_FIELDS, SYSTEM_PROGRAM_ID);
  assert.equal(zero.length, 3_920);
  assert.deepEqual([...zero.subarray(3_888)], Array.from({ length: 32 }, () => 0));
  assert.equal(decodeTickArray(zero).rentOwner, null, 'an all-zero tail is "no rent owner"');
  assert.equal(tickArrayRentOwner(zero), null);
  assert.deepEqual(encodeTickArray(TA_FIELDS, null), zero, 'null writes the zero tail');
  assert.deepEqual(encodeTickArray(TA_FIELDS), zero, 'omitted writes the zero tail');
  // a 3,888-byte V1 account is refused by the programs, and by the reader
  assert.throws(() => decodeTickArray(v2.subarray(0, 3_888)), /account length 3888, expected 3920/);
});

test('decodeTickArray: 3,887 / 3,888 / 3,921 / 3,952 are refused BY NAME, never truncated to a body', () => {
  const v2 = encodeTickArray(TA_FIELDS, addr(77));
  for (const len of [3_887, 3_888, 3_921, 3_952, 0, 3_880]) {
    const data = new Uint8Array(len);
    data.set(v2.subarray(0, Math.min(len, v2.length)));
    assert.throws(
      () => decodeTickArray(data),
      (e: unknown) => e instanceof AccountDecodeError && e.message === `TickArray: account length ${len}, expected 3920`,
      `${len} B`,
    );
    assert.equal(tickArrayRentOwner(data), null, `${len} B has no rent owner to read`);
  }
  // the discriminator is still checked on the body of a V2
  const bad = v2.slice();
  bad[0] = (bad[0] ?? 0) ^ 0xff;
  assert.throws(() => decodeTickArray(bad), /discriminator/);
});
