/**
 * The typed quoter over `carpenter_quote_bg.wasm` (`wasm/`, built from `wasm/carpenter-quote-wasm` at the repository
 * root): the plain-pool swap quote, the liquidity and position maths, and price ↔ tick.
 *
 * THE CONTRACT with the wasm crate's exports:
 * - accounts go in as their RAW BYTES exactly as `getAccountInfo` returns them (discriminator included); tick arrays go
 *   in CONCATENATED, each a 3,888-byte BODY in a 3,888-byte stride; an empty `bitmap` means "not supplied". A
 *   tick-array ACCOUNT is 3,920 bytes; the wasm takes bodies, so this wrapper slices (`concatTickArrays`, `feesOwed`);
 * - every integer that can exceed 2^53 crosses as a DECIMAL STRING; this file converts bigints; ticks and decimals are
 *   numbers;
 * - every function returns ONE JSON string, `{"ok": …}` or `{"err": "<Name>", …}` (`version` alone returns its object
 *   bare), and never throws. `<Name>` is the chain's own error name; it becomes `QuoteError.code`.
 * Nothing here computes a number: every figure is the wasm's, i.e. `carpenter-quote`'s.
 * A pool whose hook takes part in swaps is refused (`QuoteHookNotQuotable`): its hook's SDK quotes it (FLOOR pools:
 * `@floorsupply/sdk-solana`). The quoter is advisory; `simulateTransaction` stays the final check before a wallet signs.
 * This module never imports the wasm glue (`./wasm.ts` does), so tests run it over a fake `QuoteFns`.
 */

/** The wasm-bindgen exports the wrapper calls, as `--target web` emits them (`wasm/carpenter_quote.d.ts`). */
export interface QuoteFns {
  quote_swap(pool_key: Uint8Array, pool: Uint8Array, bitmap: Uint8Array, tick_arrays: Uint8Array, zero_for_one: boolean, amount_specified: string, sqrt_price_limit_x64: string): string;
  amounts_for_liquidity(sqrt_price_x64: string, tick_current: number, tick_lower: number, tick_upper: number, liquidity_delta: string): string;
  quote_liquidity(sqrt_price_x64: string, tick_lower: number, tick_upper: number, amount0_max: string, amount1_max: string): string;
  fees_owed(pool_key: Uint8Array, pool: Uint8Array, position: Uint8Array, lower_array: Uint8Array, upper_array: Uint8Array): string;
  sqrt_price_at_tick(tick: number): string;
  tick_at_sqrt_price(sqrt_price_x64: string): string;
  tick_at_price_ui(price_ui: number, decimals0: number, decimals1: number): string;
  price_ui_at_tick(tick: number, decimals0: number, decimals1: number): number;
  version(): string;
}

/** A quoter refusal. `code` is the chain's error name (`TickArrayMissing`, `ExactOutNotSupported`, …). */
export class QuoteError extends Error {
  readonly code: string;
  /** `TickArrayMissing` / `HookActionAccountMissing`: the array start the caller must fetch and retry with. */
  readonly startIndex: number | null;
  /** `QuoteBadAccount`: which input was refused. */
  readonly account: string | null;
  constructor(code: string, extra: { startIndex?: number; account?: string; message?: string } = {}) {
    super(extra.message ?? (extra.startIndex !== undefined ? `${code} (start_index ${extra.startIndex})` : extra.account ? `${code} (${extra.account})` : code));
    this.name = 'QuoteError';
    this.code = code;
    this.startIndex = extra.startIndex ?? null;
    this.account = extra.account ?? null;
  }
}

// ---- inputs ---------------------------------------------------------------------------------

/**
 * `TickArray::ACCOUNT_LEN` — the 3,888-byte BODY (discriminator ‖ struct). The wasm's
 * contract is BODIES: every tick array crossing the boundary is exactly this, in a 3,888-byte stride
 */
export const TICK_ARRAY_BODY_LEN = 3_888;

/**
 * The two lengths a caller may hand for one tick array: the 3,888-byte BODY (what a reader that already
 * sliced holds) or the 3,920-byte ACCOUNT as `getAccountInfo`
 * returns it; `concatTickArrays` and `feesOwed` pass the body. there is no V1 ACCOUNT any
 * more (the programs refuse a 3,888-byte one) — a 3,888-byte input is a body.
 */
export const TICK_ARRAY_ACCOUNT_LENS: readonly number[] = [3_888, 3_920];

/**
 * @deprecated — it meant "the account length", and an account is 3,920. It is
 * the BODY length: read {@link TICK_ARRAY_BODY_LEN} (or {@link TICK_ARRAY_ACCOUNT_LENS} for what an
 * account may be). Kept for existing importers.
 */
export const TICK_ARRAY_ACCOUNT_LEN = TICK_ARRAY_BODY_LEN;

/**
 * The 3,888-byte body of one tick array (its account or its body), or a `QuoteBadAccount` naming `what` and
 * the length — never a truncated "body" of any other length. A view, not a copy.
 */
export function tickArrayBody(a: Uint8Array, what: string): Uint8Array {
  if (!TICK_ARRAY_ACCOUNT_LENS.includes(a.length)) {
    throw new QuoteError('QuoteBadAccount', { account: `${what} is ${a.length} B, not ${TICK_ARRAY_ACCOUNT_LENS.join(' or ')}` });
  }
  return a.subarray(0, TICK_ARRAY_BODY_LEN);
}

/** A 3,920-byte account → its 3,888-byte body; ANY other input unchanged (the wasm judges it). */
function accountToBody(a: Uint8Array): Uint8Array {
  return a.length === 3_920 ? a.subarray(0, TICK_ARRAY_BODY_LEN) : a;
}

export interface SwapQuoteInput {
  /** The pool's address bytes (32), the seed of every tick-array PDA the walk looks up. */
  poolKey: Uint8Array;
  /** `Pool` account data. */
  pool: Uint8Array;
  /** `TickArrayBitmap` account data; omit only when every needed array is supplied and known. */
  bitmap?: Uint8Array | null;
  /** Every tick array the walk may read; each the 3,920-byte account (or its body). */
  tickArrays: readonly Uint8Array[];
  zeroForOne: boolean;
  /** v4 sign: `< 0` exact-in (spend `−amountSpecified`), `> 0` exact-out. */
  amountSpecified: bigint;
  /** Omit for "no limit" (the wasm picks the direction's bound). */
  sqrtPriceLimitX64?: bigint | null;
}

export interface SwapQuote {
  /** What the trader pays. */
  amountIn: bigint;
  /** What the trader receives. */
  amountOut: bigint;
  /**
   * `lp`: the LPs' fee, in the input currency (the Carpenter Platform fee is never inside it). `protocol` /
   * `protocolCurrency`: the Carpenter Platform fee the walk pays (`Swapped.protocol_fee_total`) and the currency (0 / 1)
   * it is in — the input when the fee rides the input, else the output (then `amountOut` is already net of it). 0 on a
   * pool at a 0 rate. Never add a fee paid on the output to the input's fee total.
   */
  fees: { lp: bigint; protocol: bigint; protocolCurrency: 0 | 1 };
  /** The pool price after the swap. */
  sqrtAfter: bigint;
  tickAfter: number;
  /** Start indices of the tick arrays the walk read — what `predict.swapTickArrays` passes. */
  arraysTouched: number[];
  /** Every array the transaction must carry (on a plain pool, `arraysTouched`). */
  allArrays: number[];
}

export interface QuoterVersion {
  /** The wasm crate's version. */
  carpenterQuoteWasm: string;
  /** The hook-interface version the quoter replays. */
  hookInterface: number;
}

export interface Quoter {
  readonly version: QuoterVersion;
  quoteSwap(input: SwapQuoteInput): SwapQuote;
  amountsForLiquidity(sqrtPriceX64: bigint, tickCurrent: number, tickLower: number, tickUpper: number, liquidityDelta: bigint): { amount0: bigint; amount1: bigint };
  quoteLiquidity(sqrtPriceX64: bigint, tickLower: number, tickUpper: number, amount0Max: bigint, amount1Max: bigint): bigint;
  /**
   * `lowerArray` / `upperArray`: the arrays holding the position's two ticks (the same bytes twice when they share
   * one) — 3,920-byte accounts, each going to the wasm as its 3,888-byte body. `poolKey` is the pool's address bytes
   * and every account must NAME it: the wasm refuses a position or an array of another pool rather than returning a
   * plausible, wrong figure.
   */
  feesOwed(poolKey: Uint8Array, pool: Uint8Array, position: Uint8Array, lowerArray: Uint8Array, upperArray: Uint8Array): { fees0: bigint; fees1: bigint };
  sqrtPriceAtTick(tick: number): bigint;
  tickAtSqrtPrice(sqrtPriceX64: bigint): number;
  tickAtPriceUi(priceUi: number, decimals0: number, decimals1: number): number;
  /** Display only; `NaN` for a tick out of range. */
  priceUiAtTick(tick: number, decimals0: number, decimals1: number): number;
}

// ---- the wire -----------------------------------------------------------------------------------

type Json = Record<string, unknown>;

function parse(fn: string, text: string): Json {
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    throw new QuoteError('QuoteBadResponse', { message: `${fn}: the wasm returned non-JSON` });
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw new QuoteError('QuoteBadResponse', { message: `${fn}: the wasm returned ${text}` });
  const o = v as Json;
  if (typeof o['err'] === 'string') {
    const extra: { startIndex?: number; account?: string } = {};
    if (typeof o['start_index'] === 'number') extra.startIndex = o['start_index'];
    if (typeof o['account'] === 'string') extra.account = o['account'];
    throw new QuoteError(o['err'], extra);
  }
  const ok = o['ok'];
  if (!ok || typeof ok !== 'object' || Array.isArray(ok)) throw new QuoteError('QuoteBadResponse', { message: `${fn}: no "ok" in ${text}` });
  return ok as Json;
}

const DECIMAL = /^-?\d+$/;

function big(o: Json, k: string): bigint {
  const v = o[k];
  if (typeof v !== 'string' || !DECIMAL.test(v)) throw new QuoteError('QuoteBadResponse', { message: `field ${k} is not a decimal string: ${String(v)}` });
  return BigInt(v);
}

function num(o: Json, k: string): number {
  const v = o[k];
  if (typeof v !== 'number' || !Number.isFinite(v)) throw new QuoteError('QuoteBadResponse', { message: `field ${k} is not a number: ${String(v)}` });
  return v;
}

function bool(o: Json, k: string): boolean {
  const v = o[k];
  if (typeof v !== 'boolean') throw new QuoteError('QuoteBadResponse', { message: `field ${k} is not a boolean: ${String(v)}` });
  return v;
}

/** A currency index, 0 or 1 — a JSON integer (the wasm's `Obj::num`); anything else is a bad response, never a default. */
function side(o: Json, k: string): 0 | 1 {
  const v = num(o, k);
  if (v !== 0 && v !== 1) throw new QuoteError('QuoteBadResponse', { message: `field ${k} is not a currency index (0 or 1): ${String(v)}` });
  return v;
}

function str(o: Json, k: string): string {
  const v = o[k];
  if (typeof v !== 'string') throw new QuoteError('QuoteBadResponse', { message: `field ${k} is not a string: ${String(v)}` });
  return v;
}

function ints(o: Json, k: string): number[] {
  const v = o[k];
  if (!Array.isArray(v) || !v.every((x) => typeof x === 'number' && Number.isInteger(x))) {
    throw new QuoteError('QuoteBadResponse', { message: `field ${k} is not an integer array` });
  }
  return v as number[];
}

function obj(o: Json, k: string): Json {
  const v = o[k];
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw new QuoteError('QuoteBadResponse', { message: `field ${k} is not an object` });
  return v as Json;
}

/** The wasm parses `&str` integers with Rust's `parse`, so only a plain decimal is acceptable. */
export function toDecimal(v: bigint): string {
  return v.toString(10);
}

/**
 * Concatenate tick arrays for the boundary: each a 3,920-byte account (or its 3,888-byte body), packed
 * as its 3,888-byte BODY in a 3,888-byte stride — the wasm's contract. Any other length
 * is refused `QuoteBadAccount`, naming the index and the length.
 */
export function concatTickArrays(arrays: readonly Uint8Array[]): Uint8Array {
  const out = new Uint8Array(arrays.length * TICK_ARRAY_BODY_LEN);
  arrays.forEach((a, i) => {
    out.set(tickArrayBody(a, `tick_arrays[${i}]`), i * TICK_ARRAY_BODY_LEN);
  });
  return out;
}

const EMPTY = new Uint8Array(0);

export function decodeSwapQuote(o: Json): SwapQuote {
  const fees = obj(o, 'fees');
  return {
    amountIn: big(o, 'amount_in'),
    amountOut: big(o, 'amount_out'),
    fees: { lp: big(fees, 'lp'), protocol: big(fees, 'protocol'), protocolCurrency: side(fees, 'protocol_currency') },
    sqrtAfter: big(o, 'sqrt_after'),
    tickAfter: num(o, 'tick_after'),
    arraysTouched: ints(o, 'arrays_touched'),
    allArrays: ints(o, 'all_arrays'),
  };
}

/** `version()` is the one export WITHOUT the `{"ok"}` envelope (lib.rs writes the object bare). */
export function decodeVersion(text: string): QuoterVersion {
  let v: unknown;
  try {
    v = JSON.parse(text);
  } catch {
    throw new QuoteError('QuoteBadResponse', { message: `version: the wasm returned non-JSON` });
  }
  if (!v || typeof v !== 'object' || Array.isArray(v)) throw new QuoteError('QuoteBadResponse', { message: `version: the wasm returned ${text}` });
  const o = v as Json;
  return { carpenterQuoteWasm: str(o, 'carpenter_quote_wasm'), hookInterface: num(o, 'hook_interface') };
}

/** Build the typed `Quoter` over the wasm's exports (or a fake, in tests). */
export function createQuoter(fns: QuoteFns): Quoter {
  const version = decodeVersion(fns.version());
  return {
    version,
    quoteSwap(input) {
      // No pre-checks: `AmountZero`, a bad pool key and a hooked pool are the wasm's answers, surfaced unchanged.
      const text = fns.quote_swap(
        input.poolKey,
        input.pool,
        input.bitmap ?? EMPTY,
        concatTickArrays(input.tickArrays),
        input.zeroForOne,
        toDecimal(input.amountSpecified),
        input.sqrtPriceLimitX64 === null || input.sqrtPriceLimitX64 === undefined ? '' : toDecimal(input.sqrtPriceLimitX64),
      );
      return decodeSwapQuote(parse('quote_swap', text));
    },
    amountsForLiquidity(sqrtPriceX64, tickCurrent, tickLower, tickUpper, liquidityDelta) {
      const o = parse('amounts_for_liquidity', fns.amounts_for_liquidity(toDecimal(sqrtPriceX64), tickCurrent, tickLower, tickUpper, toDecimal(liquidityDelta)));
      return { amount0: big(o, 'amount0'), amount1: big(o, 'amount1') };
    },
    quoteLiquidity(sqrtPriceX64, tickLower, tickUpper, amount0Max, amount1Max) {
      return big(parse('quote_liquidity', fns.quote_liquidity(toDecimal(sqrtPriceX64), tickLower, tickUpper, toDecimal(amount0Max), toDecimal(amount1Max))), 'liquidity');
    },
    feesOwed(poolKey, pool, position, lowerArray, upperArray) {
      // The wasm decodes each array EXACT-LENGTH (3,888), so an account goes in as its body. Any other length goes through UNCHANGED for the wasm to refuse by its own name — no pre-check
      // here; only the body slice the wasm requires.
      const o = parse('fees_owed', fns.fees_owed(poolKey, pool, position, accountToBody(lowerArray), accountToBody(upperArray)));
      return { fees0: big(o, 'fees0'), fees1: big(o, 'fees1') };
    },
    sqrtPriceAtTick: (tick) => big(parse('sqrt_price_at_tick', fns.sqrt_price_at_tick(tick)), 'sqrt_price_x64'),
    tickAtSqrtPrice: (sqrtPriceX64) => num(parse('tick_at_sqrt_price', fns.tick_at_sqrt_price(toDecimal(sqrtPriceX64))), 'tick'),
    tickAtPriceUi: (priceUi, d0, d1) => num(parse('tick_at_price_ui', fns.tick_at_price_ui(priceUi, d0, d1)), 'tick'),
    priceUiAtTick: (tick, d0, d1) => fns.price_ui_at_tick(tick, d0, d1),
  };
}

/**
 * The wire helpers `createQuoter` decodes with, for a hook's SDK whose own wasm speaks the same envelope, so a refusal
 * and a malformed answer read exactly as the AMM's do.
 */
export const QUOTE_WIRE = { parse, big, num, bool, str, obj, ints, side } as const;
