// `createQuoter` over a fake `QuoteFns`: the bigint ↔ decimal-string conversion, the
// `{"ok"}` / `{"err"}` envelope, tick-array concatenation, and that no number is computed here.
// The quoter's Rust crate is exercised by `crates/carpenter-quote/tests/vectors.rs`.

import assert from 'node:assert/strict';
import { test } from './node-test.ts';

import {
  concatTickArrays,
  QuoteError,
  TICK_ARRAY_ACCOUNT_LEN,
  TICK_ARRAY_ACCOUNT_LENS,
  TICK_ARRAY_BODY_LEN,
  tickArrayBody,
  toDecimal,
} from '../src/index.ts';
import { createQuoter, type QuoteFns } from '../src/quote/quoter.ts';

const VERSION = '{"carpenter_quote_wasm":"0.1.0","hook_interface":1}';

/** A fake that records every call and answers from a table. */
function fake(answers: Partial<Record<keyof QuoteFns, string | number>>): { fns: QuoteFns; calls: { fn: string; args: unknown[] }[] } {
  const calls: { fn: string; args: unknown[] }[] = [];
  const mk =
    (fn: keyof QuoteFns, fallback: string | number) =>
    (...args: unknown[]) => {
      calls.push({ fn, args });
      return answers[fn] ?? fallback;
    };
  const fns = {
    quote_swap: mk('quote_swap', '{"err":"AmountZero"}'),
    amounts_for_liquidity: mk('amounts_for_liquidity', '{"ok":{"amount0":"1","amount1":"2"}}'),
    quote_liquidity: mk('quote_liquidity', '{"ok":{"liquidity":"3"}}'),
    fees_owed: mk('fees_owed', '{"ok":{"fees0":"4","fees1":"5"}}'),
    sqrt_price_at_tick: mk('sqrt_price_at_tick', '{"ok":{"sqrt_price_x64":"18446744073709551616"}}'),
    tick_at_sqrt_price: mk('tick_at_sqrt_price', '{"ok":{"tick":0}}'),
    tick_at_price_ui: mk('tick_at_price_ui', '{"ok":{"tick":-69081}}'),
    price_ui_at_tick: mk('price_ui_at_tick', 1.5),
    version: mk('version', VERSION),
  } as unknown as QuoteFns;
  return { fns, calls };
}

/** A swap answer exactly as the wasm crate's `swap_json` writes it: u64/u128 as decimal STRINGS, ticks as numbers. */
const SWAP_OK =
  '{"ok":{"amount_in":"1000000000","amount_out":"99887766554433","fees":{"lp":"2500000","protocol":"49975","protocol_currency":0},' +
  '"sqrt_after":"18446744073709551616","tick_after":-1,"arrays_touched":[-3600,0],"all_arrays":[-3600,0]}}';

test('version is decoded once, at construction', () => {
  const { fns, calls } = fake({});
  const q = createQuoter(fns);
  assert.deepEqual(q.version, { carpenterQuoteWasm: '0.1.0', hookInterface: 1 });
  assert.equal(calls.length, 1);
  assert.equal(calls[0]?.fn, 'version');
});

test('quoteSwap: bigints cross as decimal strings, arrays concatenated, an empty bitmap and limit when omitted', () => {
  const { fns, calls } = fake({ quote_swap: SWAP_OK });
  const q = createQuoter(fns);
  const ta = (fill: number) => new Uint8Array(TICK_ARRAY_ACCOUNT_LEN).fill(fill);
  const poolKey = new Uint8Array(32).fill(7);
  const pool = new Uint8Array(640).fill(1);
  const r = q.quoteSwap({ poolKey, pool, tickArrays: [ta(1), ta(2)], zeroForOne: true, amountSpecified: -(1n << 63n) });
  const c = calls.find((x) => x.fn === 'quote_swap');
  assert.ok(c);
  assert.equal(c.args.length, 7);
  const [k, p, bm, arrays, z41, amt, limit] = c.args as [Uint8Array, Uint8Array, Uint8Array, Uint8Array, boolean, string, string];
  assert.equal(k, poolKey);
  assert.equal(p, pool);
  assert.equal(bm.length, 0);
  assert.equal(arrays.length, 2 * TICK_ARRAY_ACCOUNT_LEN);
  assert.equal(arrays[0], 1);
  assert.equal(arrays[TICK_ARRAY_ACCOUNT_LEN], 2);
  assert.equal(z41, true);
  assert.equal(amt, '-9223372036854775808');
  assert.equal(limit, '');
  assert.deepEqual(r, {
    amountIn: 1_000_000_000n,
    amountOut: 99_887_766_554_433n,
    fees: { lp: 2_500_000n, protocol: 49_975n, protocolCurrency: 0 },
    sqrtAfter: 1n << 64n,
    tickAfter: -1,
    arraysTouched: [-3600, 0],
    allArrays: [-3600, 0],
  });
});

test('quoteSwap: the bitmap and the limit are forwarded when given', () => {
  const { fns, calls } = fake({ quote_swap: SWAP_OK });
  const q = createQuoter(fns);
  const bitmap = new Uint8Array(80).fill(9);
  q.quoteSwap({ poolKey: new Uint8Array(32), pool: new Uint8Array(640), bitmap, tickArrays: [], zeroForOne: false, amountSpecified: -1n, sqrtPriceLimitX64: 4295048017n });
  const c = calls.find((x) => x.fn === 'quote_swap');
  const [, , bm, arrays, , , limit] = c!.args as unknown[];
  assert.equal(bm, bitmap);
  assert.equal((arrays as Uint8Array).length, 0);
  assert.equal(limit, '4295048017');
});

test('an {"err"} envelope becomes a QuoteError carrying the chain name and its extras', () => {
  const { fns } = fake({ quote_swap: '{"err":"TickArrayMissing","start_index":-3600}' });
  const q = createQuoter(fns);
  const input = { poolKey: new Uint8Array(32), pool: new Uint8Array(640), tickArrays: [], zeroForOne: true, amountSpecified: -1n };
  assert.throws(
    () => q.quoteSwap(input),
    (e: unknown) => e instanceof QuoteError && e.code === 'TickArrayMissing' && e.startIndex === -3600 && e.account === null,
  );
  const bad = createQuoter(fake({ quote_swap: '{"err":"QuoteBadAccount","account":"pool_key"}' }).fns);
  assert.throws(() => bad.quoteSwap(input), (e: unknown) => e instanceof QuoteError && e.code === 'QuoteBadAccount' && e.account === 'pool_key');
  const hooked = createQuoter(fake({ quote_swap: '{"err":"QuoteHookNotQuotable"}' }).fns);
  assert.throws(() => hooked.quoteSwap(input), (e: unknown) => e instanceof QuoteError && e.code === 'QuoteHookNotQuotable');
});

test('a malformed response is QuoteBadResponse, never a silent number', () => {
  for (const text of ['', 'null', '[]', '{"ok":1}', '{"ok":{"amount_in":1}}', '{"ok":{"liquidity":"x"}}']) {
    const q = createQuoter(fake({ quote_liquidity: text, quote_swap: text }).fns);
    assert.throws(() => q.quoteLiquidity(1n << 64n, -60, 60, 1n, 1n), (e: unknown) => e instanceof QuoteError && e.code === 'QuoteBadResponse', text);
  }
});

test('toDecimal is plain base-10 (what Rust parse::<i128> reads); concatTickArrays refuses a short account', () => {
  assert.equal(toDecimal(-(1n << 127n)), '-170141183460469231731687303715884105728');
  assert.equal(toDecimal(0n), '0');
  assert.throws(() => concatTickArrays([new Uint8Array(3887)]), (e: unknown) => e instanceof QuoteError && e.code === 'QuoteBadAccount');
  assert.equal(concatTickArrays([]).length, 0);
});

// `fees.protocol` / `fees.protocol_currency` are REQUIRED keys of the wasm's swap JSON. A response without
// them (an older wasm build) or with a currency that
// is not 0 / 1 is a bad response, never a silent 0 - a quote that drops the fee overstates what a trader gets.
test('quoteSwap: a missing Carpenter fee, or a currency that is not 0 or 1, is QuoteBadResponse', () => {
  const input = { poolKey: new Uint8Array(32), pool: new Uint8Array(640), tickArrays: [], zeroForOne: false, amountSpecified: -1n };
  const bad = (text: string) => {
    const { fns } = fake({ quote_swap: text });
    assert.throws(() => createQuoter(fns).quoteSwap(input), (e: unknown) => e instanceof QuoteError && e.code === 'QuoteBadResponse', text.slice(0, 80));
  };
  bad(SWAP_OK.replace(',"protocol":"49975","protocol_currency":0', ''));
  bad(SWAP_OK.replace('"protocol_currency":0', '"protocol_currency":2'));
  bad(SWAP_OK.replace('"protocol":"49975"', '"protocol":49975'));
  const { fns } = fake({ quote_swap: SWAP_OK.replace('"protocol":"49975","protocol_currency":0', '"protocol":"0","protocol_currency":1') });
  const r = createQuoter(fns).quoteSwap(input);
  assert.deepEqual([r.fees.protocol, r.fees.protocolCurrency], [0n, 1], 'a pool at 0 quotes 0 - a client shows no row');
});

// =================================================================================================
// A tick-array
// ACCOUNT is 3,888 (V1) or 3,920; the wasm is UNCHANGED and takes
// 3,888-byte BODIES, so the wrapper slices.
// =================================================================================================

/** A V1 account (a recognisable body) and its V2 twin: the same body, then a non-zero rent owner. */
function v1v2(fill: number): [Uint8Array, Uint8Array] {
  const v1 = new Uint8Array(3_888).map((_, i) => (i * 7 + fill) & 0xff);
  const v2 = new Uint8Array(3_920);
  v2.set(v1, 0);
  v2.fill(0xee, 3_888);
  return [v1, v2];
}

test('the lengths: TICK_ARRAY_BODY_LEN 3,888, the two account lengths, and the deprecated alias IS the body length', () => {
  assert.equal(TICK_ARRAY_BODY_LEN, 3_888);
  assert.deepEqual([...TICK_ARRAY_ACCOUNT_LENS], [3_888, 3_920]);
  assert.equal(TICK_ARRAY_ACCOUNT_LEN, TICK_ARRAY_BODY_LEN, 'the alias is the body length');
});

test('the pack: a V2 account packs as its body; mixed V1 / V2 packs identically to all-V1 bodies', () => {
  const [a1, a2] = v1v2(1);
  const [b1, b2] = v1v2(2);
  const [c1, c2] = v1v2(3);
  const allV1 = concatTickArrays([a1, b1, c1]);
  assert.equal(allV1.length, 3 * 3_888);
  assert.deepEqual(concatTickArrays([a2]), a1, 'one V2 = its body');
  assert.deepEqual(concatTickArrays([a2, b2, c2]), allV1, 'all V2');
  assert.deepEqual(concatTickArrays([a1, b2, c1]), allV1, 'mixed');
  assert.deepEqual(concatTickArrays([a2, b1, c2]), allV1, 'mixed the other way');
  // and through quoteSwap: the wasm receives the SAME bytes either way
  const { fns, calls } = fake({ quote_swap: SWAP_OK });
  const q = createQuoter(fns);
  const base = { poolKey: new Uint8Array(32), pool: new Uint8Array(640), zeroForOne: true, amountSpecified: -5n };
  q.quoteSwap({ ...base, tickArrays: [a1, b1, c1] });
  q.quoteSwap({ ...base, tickArrays: [a2, b1, c2] });
  const sent = calls.filter((c) => c.fn === 'quote_swap').map((c) => c.args[3] as Uint8Array);
  assert.equal(sent.length, 2);
  assert.deepEqual(sent[1], sent[0]);
});

test('the pack refuses 3,921 (and every other length) QuoteBadAccount, naming the index and "not 3888 or 3920"', () => {
  const [a1] = v1v2(1);
  for (const len of [3_921, 3_887, 3_952, 0, 7_776]) {
    assert.throws(
      () => concatTickArrays([a1, new Uint8Array(len)]),
      (e: unknown) => e instanceof QuoteError && e.code === 'QuoteBadAccount' && e.account === `tick_arrays[1] is ${len} B, not 3888 or 3920`,
      `${len} B`,
    );
  }
  assert.throws(() => tickArrayBody(new Uint8Array(3_921), 'x'), (e: unknown) => e instanceof QuoteError && e.account === 'x is 3921 B, not 3888 or 3920');
  assert.equal(tickArrayBody(new Uint8Array(3_920), 'x').length, 3_888);
});

test('feesOwed with V2 arrays sends the wasm exactly what V1 bodies send; any other length goes through for the wasm to judge', () => {
  const [lo1, lo2] = v1v2(4);
  const [up1, up2] = v1v2(5);
  const { fns, calls } = fake({});
  const q = createQuoter(fns);
  const key = new Uint8Array(32).fill(1);
  const pool = new Uint8Array(640);
  const pos = new Uint8Array(208);
  assert.deepEqual(q.feesOwed(key, pool, pos, lo1, up1), { fees0: 4n, fees1: 5n });
  assert.deepEqual(q.feesOwed(key, pool, pos, lo2, up2), { fees0: 4n, fees1: 5n });
  assert.deepEqual(q.feesOwed(key, pool, pos, lo2, lo2), { fees0: 4n, fees1: 5n }, 'one array holding both ticks');
  const sent = calls.filter((c) => c.fn === 'fees_owed').map((c) => c.args as Uint8Array[]);
  assert.deepEqual(sent[1]?.[3], sent[0]?.[3], 'lower: the V2 went in as its body');
  assert.deepEqual(sent[1]?.[4], sent[0]?.[4], 'upper: the V2 went in as its body');
  assert.equal(sent[1]?.[3]?.length, 3_888);
  assert.deepEqual(sent[2]?.[3], lo1);
  // no pre-check: a length that is neither goes through UNCHANGED
  const odd = new Uint8Array(3_921);
  q.feesOwed(key, pool, pos, odd, up1);
  assert.equal((calls.filter((c) => c.fn === 'fees_owed').at(-1)?.args[3] as Uint8Array).length, 3_921);
});
