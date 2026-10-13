// Tick math pins: Raydium's `get_sqrt_price_at_tick` at its bounds, the inverse's definition,
// the spacing snaps at the ±443,636 edges (clamped one spacing inside the bound), bitmap sizing against
// floor-types' own tests, `open_tick` and `tranche_price_x64`.

import assert from 'node:assert/strict';
import { test } from './node-test.ts';

import {
  MAX_SQRT_PRICE_X64,
  MAX_TICK,
  MIN_SQRT_PRICE_X64,
  MIN_TICK,
  bitmapAccountLen,
  bitmapArraysPerSide,
  bitmapBit,
  bitmapWordCount,
  ceilToSpacing,
  floorToSpacing,
  getSqrtPriceAtTick,
  getTickAtSqrtPrice,
  initializedArrayStarts,
  isArrayInitialized,
  isqrt,
  tickArrayStartIndex,
} from '../src/index.ts';

test('sqrt price at the bounds and at 0 (Raydium tick_math.rs)', () => {
  assert.equal(getSqrtPriceAtTick(0), 1n << 64n);
  assert.equal(getSqrtPriceAtTick(MIN_TICK), MIN_SQRT_PRICE_X64);
  assert.equal(getSqrtPriceAtTick(MAX_TICK), MAX_SQRT_PRICE_X64);
  assert.throws(() => getSqrtPriceAtTick(MAX_TICK + 1), RangeError);
  assert.throws(() => getSqrtPriceAtTick(MIN_TICK - 1), RangeError);
  assert.throws(() => getSqrtPriceAtTick(0.5), RangeError);
  // monotone
  let prev = 0n;
  for (const t of [MIN_TICK, -300_000, -69_081, -1, 0, 1, 60, 6932, 100_000, MAX_TICK - 1, MAX_TICK]) {
    const s = getSqrtPriceAtTick(t);
    assert.ok(s > prev, `tick ${t}`);
    prev = s;
  }
});

test('tick at sqrt price is the greatest tick whose sqrt price ≤ the input', () => {
  for (const t of [MIN_TICK, -443_635, -100_000, -69_081, -60, -1, 0, 1, 59, 60, 6932, 250_000, MAX_TICK - 1]) {
    const s = getSqrtPriceAtTick(t);
    assert.equal(getTickAtSqrtPrice(s), t, `exact ${t}`);
    assert.equal(getTickAtSqrtPrice(s + 1n), t, `just above ${t}`);
    if (t > MIN_TICK) assert.equal(getTickAtSqrtPrice(s - 1n), t - 1, `just below ${t}`);
  }
  assert.equal(getTickAtSqrtPrice(MAX_SQRT_PRICE_X64 - 1n), MAX_TICK - 1);
  assert.throws(() => getTickAtSqrtPrice(MAX_SQRT_PRICE_X64), RangeError);
  assert.throws(() => getTickAtSqrtPrice(MIN_SQRT_PRICE_X64 - 1n), RangeError);
});

test('isqrt is the floor square root', () => {
  assert.equal(isqrt(0n), 0n);
  assert.equal(isqrt(1n), 1n);
  assert.equal(isqrt(15n), 3n);
  assert.equal(isqrt(16n), 4n);
  assert.equal(isqrt((1n << 128n) - 1n), (1n << 64n) - 1n);
  assert.equal(isqrt(1n << 192n), 1n << 96n);
  assert.throws(() => isqrt(-1n), RangeError);
});

test('ceil_to_spacing / floor_to_spacing: sign handling and the edge clamps (one spacing inside ±443,636)', () => {
  assert.equal(ceilToSpacing(0, 60), 0);
  assert.equal(ceilToSpacing(1, 60), 60);
  assert.equal(ceilToSpacing(60, 60), 60);
  assert.equal(ceilToSpacing(-1, 60), 0);
  assert.equal(ceilToSpacing(-61, 60), -60);
  assert.equal(floorToSpacing(0, 60), 0);
  assert.equal(floorToSpacing(59, 60), 0);
  assert.equal(floorToSpacing(-1, 60), -60);
  assert.equal(floorToSpacing(-60, 60), -60);
  assert.equal(floorToSpacing(-61, 60), -120);
  // clamps: result > MAX − s → (MAX / s)·s − s ; result < MIN + s → (MIN / s)·s + s
  assert.equal(ceilToSpacing(MAX_TICK, 60), Math.trunc(MAX_TICK / 60) * 60 - 60);
  assert.equal(ceilToSpacing(MAX_TICK - 59, 60), Math.trunc(MAX_TICK / 60) * 60 - 60);
  assert.equal(floorToSpacing(MIN_TICK, 60), Math.trunc(MIN_TICK / 60) * 60 + 60);
  assert.equal(floorToSpacing(MIN_TICK + 59, 60), Math.trunc(MIN_TICK / 60) * 60 + 60);
  // an in-range value is untouched by the clamp
  assert.equal(ceilToSpacing(443_400, 60), 443_400);
});

test('tick array start floors toward −∞ (floor-types layout.rs tests)', () => {
  assert.equal(tickArrayStartIndex(0, 60), 0);
  assert.equal(tickArrayStartIndex(3599, 60), 0);
  assert.equal(tickArrayStartIndex(3600, 60), 3600);
  assert.equal(tickArrayStartIndex(-1, 60), -3600);
  assert.equal(tickArrayStartIndex(-3600, 60), -3600);
  assert.equal(tickArrayStartIndex(-3601, 60), -7200);
});

test('bitmap sizing matches floor-types (124 / 4 / 80 at spacing 60; 240 at 10; 1,904 at 1)', () => {
  assert.equal(bitmapArraysPerSide(60), 124);
  assert.equal(bitmapWordCount(60), 4);
  assert.equal(bitmapAccountLen(60), 80);
  assert.equal(bitmapAccountLen(10), 240);
  assert.equal(bitmapAccountLen(1), 1_904);
  for (const s of [1, 10, 60, 200, 32_767]) {
    const lo = tickArrayStartIndex(MIN_TICK, s);
    const hi = tickArrayStartIndex(MAX_TICK, s);
    assert.ok(bitmapBit(lo, s) >= 0, `spacing ${s} low bit`);
    assert.ok(bitmapBit(hi, s) < 64 * bitmapWordCount(s), `spacing ${s} high bit`);
  }
  // set / list round trip
  const s = 60;
  const words = [0n, 0n, 0n, 0n];
  for (const start of [-7200, 0, 3600, 439_200]) {
    const bit = bitmapBit(start, s);
    words[Math.floor(bit / 64)]! |= 1n << BigInt(bit % 64);
  }
  assert.deepEqual(initializedArrayStarts(words, s), [-7200, 0, 3600, 439_200]);
  assert.equal(isArrayInitialized(words, 3600, s), true);
  assert.equal(isArrayInitialized(words, 7200, s), false);
});

