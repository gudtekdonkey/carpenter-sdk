// The shipped wasm: `QuoteFns` (what the wrapper calls) == `scripts/wasm-exports.mjs` (what sync-wasm requires) == the
// glue's exports; the shipped bytes hash to `WASM_SHA256` and `wasm/SHA256SUMS`; the binary and its glue name no local
// path; and every swap of `fixtures/quote-vectors/plain.json` (the Rust quoter's own answers) and `parity-plain.json`
// (chain-measured) quotes through the wasm to the same figures, or the same error name.

import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

import { REQUIRED_EXPORTS, WEB_TARGET_MARKERS } from '../scripts/wasm-exports.mjs';
import { hexToBytes } from '../src/bytes.ts';
import { QuoteError, type Quoter } from '../src/quote/quoter.ts';
import { initQuote, WASM_BYTES, WASM_SHA256 } from '../src/quote/wasm.ts';
import { SDK_ROOT } from './helpers.ts';
import { test } from './node-test.ts';

const wasmDir = join(SDK_ROOT, 'wasm');
const wasm = readFileSync(join(wasmDir, 'carpenter_quote_bg.wasm'));
const glue = readFileSync(join(wasmDir, 'carpenter_quote.js'), 'utf8');
const dts = readFileSync(join(wasmDir, 'carpenter_quote.d.ts'), 'utf8');
const vectors = (name: string) => JSON.parse(readFileSync(join(SDK_ROOT, '../../fixtures/quote-vectors', name), 'utf8'));

test('QuoteFns, the required-export list and the glue name the same functions; the glue is --target web', () => {
  const src = readFileSync(join(SDK_ROOT, 'src/quote/quoter.ts'), 'utf8');
  const body = /export interface QuoteFns \{([\s\S]*?)\n\}/.exec(src)![1]!;
  const fnsNames = [...body.matchAll(/^\s{2}(\w+)\(/gm)].map((m) => m[1]!).sort();
  assert.deepEqual(fnsNames, [...REQUIRED_EXPORTS].sort());
  const glueNames = [...glue.matchAll(/^export function (\w+)\(/gm)].map((m) => m[1]!).filter((n) => n !== 'initSync').sort();
  assert.deepEqual(glueNames, [...REQUIRED_EXPORTS].sort());
  for (const [what, re] of WEB_TARGET_MARKERS) assert.match(glue, re, what);
  // arities: QuoteFns against the shipped .d.ts
  for (const name of REQUIRED_EXPORTS) {
    const ours = new RegExp(`\\b${name}\\(([^)]*)\\)`).exec(body)![1]!.split(',').filter((x) => x.trim()).length;
    const theirs = new RegExp(`export function ${name}\\(([^)]*)\\)`).exec(dts)![1]!.split(',').filter((x) => x.trim()).length;
    assert.equal(ours, theirs, `${name} arity`);
  }
});

test('the shipped wasm hashes to WASM_SHA256 and SHA256SUMS; neither it nor its glue names a local path', () => {
  const sha = createHash('sha256').update(wasm).digest('hex');
  assert.equal(sha, WASM_SHA256);
  assert.equal(wasm.length, WASM_BYTES);
  assert.equal(readFileSync(join(wasmDir, 'SHA256SUMS'), 'utf8'), `${sha}  carpenter_quote_bg.wasm\n`);
  for (const [name, text] of [['wasm', wasm.toString('latin1')], ['glue', glue], ['d.ts', dts]] as const) {
    assert.doesNotMatch(text, /\/(tmp|root|home|build)\//, `${name} names a local path`);
    assert.doesNotMatch(text, /[Cc]laude/, name);
    // the crate's doc comments are stripped before the build: none of their text reaches the glue
    if (name !== 'wasm') assert.doesNotMatch(text, /Quote a swap|Display only|Fees owed to a position|exact-in/, `${name} carries the crate's doc comments`);
  }
});

let quoter: Quoter;
const q = async () => (quoter ??= await initQuote(new Uint8Array(wasm)));

test('the wasm loads, reports its version, and refuses by the chain`s names', async () => {
  const w = await q();
  assert.deepEqual(w.version, { carpenterQuoteWasm: '0.1.0', hookInterface: w.version.hookInterface });
  assert.equal(w.sqrtPriceAtTick(0), 1n << 64n);
  assert.equal(w.tickAtSqrtPrice(1n << 64n), 0);
  assert.throws(() => w.sqrtPriceAtTick(1_000_000), (e: unknown) => e instanceof QuoteError && e.code === 'TickOutOfRange');
  assert.throws(
    () => w.quoteSwap({ poolKey: new Uint8Array(3), pool: new Uint8Array(0), tickArrays: [], zeroForOne: true, amountSpecified: -1n }),
    (e: unknown) => e instanceof QuoteError && e.code === 'QuoteBadAccount' && e.account === 'pool_key',
  );
});

test('a paused pool is refused PoolPaused by the wasm, as by the program', async () => {
  const w = await q();
  const c = vectors('plain.json').cases[0] as { pool_key: string; pool: string; bitmap: string; tick_arrays: string[] };
  const pool = hexToBytes(c.pool);
  const input = { poolKey: hexToBytes(c.pool_key), pool, bitmap: hexToBytes(c.bitmap), tickArrays: c.tick_arrays.map(hexToBytes), zeroForOne: true, amountSpecified: -1_000n };
  assert.ok(w.quoteSwap(input).amountOut > 0n);
  const paused = pool.slice();
  paused[8 + 504] = 1; // `Pool.paused`, after the discriminator
  assert.throws(() => w.quoteSwap({ ...input, pool: paused }), (e: unknown) => e instanceof QuoteError && e.code === 'PoolPaused');
});

interface PlainSwap {
  zero_for_one: boolean;
  amount_specified: string;
  sqrt_price_limit_x64: string;
  expect: { ok?: Record<string, unknown>; err?: string };
}

test('plain.json: every swap through the wasm equals carpenter-quote`s recorded answer', async () => {
  const w = await q();
  const v = vectors('plain.json') as { cases: { name: string; pool_key: string; pool: string; bitmap: string; tick_arrays: string[]; swaps: PlainSwap[] }[] };
  let n = 0;
  for (const c of v.cases) {
    for (const s of c.swaps) {
      const input = {
        poolKey: hexToBytes(c.pool_key),
        pool: hexToBytes(c.pool),
        bitmap: hexToBytes(c.bitmap),
        tickArrays: c.tick_arrays.map(hexToBytes),
        zeroForOne: s.zero_for_one,
        amountSpecified: BigInt(s.amount_specified),
        sqrtPriceLimitX64: BigInt(s.sqrt_price_limit_x64),
      };
      const what = `${c.name} #${n}`;
      if (s.expect.err) {
        assert.throws(() => w.quoteSwap(input), (e: unknown) => e instanceof QuoteError && e.code === s.expect.err, what);
      } else {
        const r = w.quoteSwap(input);
        const e = s.expect.ok!;
        assert.deepEqual(
          [r.amountIn, r.amountOut, r.fees.lp, r.sqrtAfter, r.tickAfter, r.arraysTouched],
          [BigInt(e['amount_in'] as string), BigInt(e['amount_out'] as string), BigInt(e['fee_lp'] as string), BigInt(e['sqrt_after'] as string), e['tick_after'], e['arrays_touched']],
          what,
        );
      }
      n++;
    }
  }
  assert.equal(n, 13, 'every recorded swap was replayed');
});

test('parity-plain.json: every chain-measured swap through the wasm equals the chain', async () => {
  const w = await q();
  const v = vectors('parity-plain.json') as {
    swaps: (PlainSwap & { pool_key: string; pool: string; bitmap: string; tick_arrays: string[]; floor_pool: string | null })[];
  };
  let n = 0;
  for (const s of v.swaps) {
    assert.equal(s.floor_pool, null, 'plain pools only');
    const input = {
      poolKey: hexToBytes(s.pool_key),
      pool: hexToBytes(s.pool),
      bitmap: hexToBytes(s.bitmap),
      tickArrays: s.tick_arrays.map(hexToBytes),
      zeroForOne: s.zero_for_one,
      amountSpecified: BigInt(s.amount_specified),
      sqrtPriceLimitX64: BigInt(s.sqrt_price_limit_x64),
    };
    if (s.expect.err) {
      assert.throws(() => w.quoteSwap(input), (e: unknown) => e instanceof QuoteError && e.code === s.expect.err, `#${n}`);
    } else {
      const r = w.quoteSwap(input);
      const e = s.expect.ok!;
      assert.deepEqual(
        [r.amountIn, r.amountOut, r.fees.lp, r.sqrtAfter, r.tickAfter, r.allArrays],
        [BigInt(e['amount_in'] as string), BigInt(e['amount_out'] as string), BigInt(e['fee_lp'] as string), BigInt(e['sqrt_after'] as string), e['tick_after'], e['all_arrays']],
        `#${n}`,
      );
    }
    n++;
  }
  assert.equal(n, 40, 'every chain-measured swap was replayed');
});
