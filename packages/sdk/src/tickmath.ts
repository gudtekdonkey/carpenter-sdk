/**
 * Tick math the SDK needs without the wasm quoter: sqrt price ↔ tick, spacing snaps, tick-array starts and the bitmap.
 *
 * `getSqrtPriceAtTick` is Raydium CLMM `ed7c84a5` `programs/amm/src/libraries/tick_math.rs` (Apache-2.0), transliterated
 * to bigint. CHANGED: `anchor_lang` errors → `RangeError`; U128 limbs → bigint. Everything else (the magic factors, the
 * `U128::MAX / ratio` inversion) is byte-for-byte. `getTickAtSqrtPrice` is the same function's DEFINITION ("the
 * greatest tick value such that get_sqrt_price_at_tick(tick) <= ratio", tick_math.rs doc comment), computed by
 * bisection over the exact forward function, so it agrees with Raydium's log2 path wherever that path is correct.
 * The quoter is the authority for swaps; this file never walks liquidity.
 */

import { MAX_SQRT_PRICE_X64, MAX_TICK, MIN_SQRT_PRICE_X64, MIN_TICK, TICK_ARRAY_SIZE } from './constants.ts';

const U128_MAX = (1n << 128n) - 1n;

/** `2^64 / 1.0001^(2^(i-1))` for i in [1, 18], Raydium tick_math.rs. Index 0 is the odd-tick factor. */
const FACTORS: readonly bigint[] = [
  0xfffcb933bd6fb800n,
  0xfff97272373d4000n,
  0xfff2e50f5f657000n,
  0xffe5caca7e10f000n,
  0xffcb9843d60f7000n,
  0xff973b41fa98e800n,
  0xff2ea16466c9b000n,
  0xfe5dee046a9a3800n,
  0xfcbe86c7900bb000n,
  0xf987a7253ac65800n,
  0xf3392b0822bb6000n,
  0xe7159475a2caf000n,
  0xd097f3bdfd2f2000n,
  0xa9f746462d9f8000n,
  0x70d869a156f31c00n,
  0x31be135f97ed3200n,
  0x9aa508b5b85a500n,
  0x5d6af8dedc582cn,
  0x2216e584f5fan,
];

/** Q64.64 sqrt price at a tick (Raydium `get_sqrt_price_at_tick`). */
export function getSqrtPriceAtTick(tick: number): bigint {
  if (!Number.isInteger(tick) || tick < MIN_TICK || tick > MAX_TICK) throw new RangeError(`tick out of range: ${tick}`);
  const abs = Math.abs(tick);
  let ratio = abs & 1 ? (FACTORS[0] as bigint) : 1n << 64n;
  for (let i = 1; i < FACTORS.length; i++) {
    if (abs & (1 << i)) ratio = (ratio * (FACTORS[i] as bigint)) >> 64n;
  }
  if (tick > 0) ratio = U128_MAX / ratio;
  return ratio;
}

/** The greatest tick with `getSqrtPriceAtTick(tick) <= sqrtPriceX64`. Domain `[MIN, MAX)`. */
export function getTickAtSqrtPrice(sqrtPriceX64: bigint): number {
  if (sqrtPriceX64 < MIN_SQRT_PRICE_X64 || sqrtPriceX64 >= MAX_SQRT_PRICE_X64) {
    throw new RangeError(`sqrt price out of range: ${sqrtPriceX64}`);
  }
  let lo = MIN_TICK; // sqrt_at(lo) <= p
  let hi = MAX_TICK; // sqrt_at(hi) > p
  while (hi - lo > 1) {
    const mid = Math.floor((lo + hi) / 2);
    if (getSqrtPriceAtTick(mid) <= sqrtPriceX64) lo = mid;
    else hi = mid;
  }
  return lo;
}

/** Integer square root (floor). */
export function isqrt(n: bigint): bigint {
  if (n < 0n) throw new RangeError('isqrt of a negative');
  if (n < 2n) return n;
  let x = 1n << BigInt(Math.ceil(n.toString(2).length / 2));
  for (;;) {
    const y = (x + n / x) >> 1n;
    if (y >= x) return x;
    x = y;
  }
}

/** Solidity `int24 / int24`: truncation toward zero. `|| 0` turns JavaScript's `-0` into `0`. */
const tdiv = (a: number, b: number): number => Math.trunc(a / b) || 0;

/**
 * Round `tick` up to a multiple of `spacing` (truncating division, then +1 for a positive remainder). Above
 * `MAX_TICK − spacing` the result is clamped to `trunc(MAX_TICK / spacing)·spacing − spacing`, so `[result, result + spacing]`
 * stays inside the tick bounds.
 */
export function ceilToSpacing(tick: number, spacing: number): number {
  let compressed = tdiv(tick, spacing);
  if (tick > 0 && tick % spacing !== 0) compressed += 1;
  let result = compressed * spacing || 0;
  if (result > MAX_TICK - spacing) result = tdiv(MAX_TICK, spacing) * spacing - spacing;
  return result;
}

/**
 * Round `tick` down to a multiple of `spacing` (truncating division, then −1 for a negative remainder). Below
 * `MIN_TICK + spacing` the result is clamped to `trunc(MIN_TICK / spacing)·spacing + spacing`, so `[result − spacing, result]`
 * stays inside the tick bounds.
 */
export function floorToSpacing(tick: number, spacing: number): number {
  let compressed = tdiv(tick, spacing);
  if (tick < 0 && tick % spacing !== 0) compressed -= 1;
  let result = compressed * spacing || 0;
  if (result < MIN_TICK + spacing) result = tdiv(MIN_TICK, spacing) * spacing + spacing;
  return result;
}

/** `start_index = floor(tick / (60·s)) · 60·s`, floor toward −∞. */
export function tickArrayStartIndex(tick: number, spacing: number): number {
  const span = TICK_ARRAY_SIZE * spacing;
  return Math.floor(tick / span) * span;
}

/** `arrays_per_side = ceil(443,636 / (60·s))`. */
export function bitmapArraysPerSide(spacing: number): number {
  return Math.ceil(MAX_TICK / (TICK_ARRAY_SIZE * spacing));
}

/** `N = ceil((2·arrays_per_side + 1) / 64)`. */
export function bitmapWordCount(spacing: number): number {
  return Math.ceil((2 * bitmapArraysPerSide(spacing) + 1) / 64);
}

/** The bitmap account is exactly `8 + 40 + 8·N` bytes. */
export function bitmapAccountLen(spacing: number): number {
  return 8 + 40 + 8 * bitmapWordCount(spacing);
}

/** The bit of the array starting at `startIndex` is `start/(60·s) + arrays_per_side`. */
export function bitmapBit(startIndex: number, spacing: number): number {
  return tdiv(startIndex, TICK_ARRAY_SIZE * spacing) + bitmapArraysPerSide(spacing);
}

/** Is the array starting at `startIndex` marked initialized in the bitmap words? */
export function isArrayInitialized(words: readonly bigint[], startIndex: number, spacing: number): boolean {
  const bit = bitmapBit(startIndex, spacing);
  const word = words[Math.floor(bit / 64)];
  return word !== undefined && ((word >> BigInt(bit % 64)) & 1n) === 1n;
}

/** Every initialized array start index in the bitmap, ascending. */
export function initializedArrayStarts(words: readonly bigint[], spacing: number): number[] {
  const perSide = bitmapArraysPerSide(spacing);
  const span = TICK_ARRAY_SIZE * spacing;
  const out: number[] = [];
  for (let w = 0; w < words.length; w++) {
    let word = words[w] as bigint;
    for (let b = 0; word !== 0n; b++, word >>= 1n) {
      if (word & 1n) out.push((w * 64 + b - perSide) * span);
    }
  }
  return out;
}

