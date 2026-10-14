//! `carpenter-quote-wasm`: the JS surface of `carpenter-quote`'s plain-pool quoter.
//!
//! The contract with the TypeScript wrapper (`@floorlaunchpad/carpenter-sdk`'s `createQuoter`):
//! - Accounts go in as their raw bytes (`Uint8Array`), as `getAccountInfo` returns them. Tick arrays
//!   go in CONCATENATED, each a 3,888-byte account body in a 3,888-byte stride (the wrapper slices a
//!   3,920-byte account to its body). An empty `bitmap` means "not supplied".
//! - Every integer that can exceed 2^53 goes in and out as a DECIMAL STRING; ticks, decimals and
//!   spacings are numbers.
//! - Every function returns one JSON string, `{"ok": …}` or `{"err": "<Name>", …}` (`version` returns
//!   its object bare), and never throws or panics on bad input. An error name is the one the chain
//!   uses (`QuoteError::name`).
//! - No hook quoter is linked: a pool whose hook takes part in swaps is refused with
//!   `QuoteHookNotQuotable`; a hook's SDK ships the wasm that quotes its pools.
//!
//! Nothing here computes; every number comes from `carpenter-quote`.

use carpenter_quote::price;
use carpenter_quote::state::{decode_account, decode_pool, TickArraySet};
use carpenter_quote::types::layout::{Layout, Position, Tick, TickArray};
use carpenter_quote::{quote_plain, PoolAccount, QuoteError, SwapQuote, SwapQuoteParams};
use wasm_bindgen::prelude::*;

// ---- JSON ---------------------------------------------------------------------------------------

struct Obj(String);

impl Obj {
    fn new() -> Self {
        Obj(String::from("{"))
    }
    fn key(&mut self, k: &str) {
        if self.0.len() > 1 {
            self.0.push(',');
        }
        self.0.push('"');
        self.0.push_str(k);
        self.0.push_str("\":");
    }
    fn str(mut self, k: &str, v: impl ToString) -> Self {
        self.key(k);
        self.0.push('"');
        self.0.push_str(&v.to_string());
        self.0.push('"');
        self
    }
    fn num(mut self, k: &str, v: impl ToString) -> Self {
        self.key(k);
        self.0.push_str(&v.to_string());
        self
    }
    fn raw(mut self, k: &str, json: &str) -> Self {
        self.key(k);
        self.0.push_str(json);
        self
    }
    fn done(mut self) -> String {
        self.0.push('}');
        self.0
    }
}

fn ok(body: String) -> String {
    Obj::new().raw("ok", &body).done()
}

fn err(e: QuoteError) -> String {
    let mut o = Obj::new().str("err", e.name());
    match e {
        QuoteError::TickArrayMissing { start_index } | QuoteError::HookActionAccountMissing { start_index } => {
            o = o.num("start_index", start_index);
        }
        QuoteError::QuoteBadAccount(which) => {
            o = o.str("account", which);
        }
        _ => {}
    }
    o.done()
}

fn out(r: Result<String, QuoteError>) -> String {
    match r {
        Ok(body) => ok(body),
        Err(e) => err(e),
    }
}

fn ints(v: &[i32]) -> String {
    let items: Vec<String> = v.iter().map(|x| x.to_string()).collect();
    format!("[{}]", items.join(","))
}

fn parse<T: core::str::FromStr>(s: &str) -> Result<T, QuoteError> {
    s.trim().parse::<T>().map_err(|_| QuoteError::QuoteBadAccount("number"))
}

fn key32(b: &[u8]) -> Result<[u8; 32], QuoteError> {
    b.try_into().map_err(|_| QuoteError::QuoteBadAccount("pool_key"))
}

// ---- swap ---------------------------------------------------------------------------------------

/// Quote a swap on a pool with no swap hook. `amount_specified`: `< 0` exact-in, `> 0` exact-out.
/// `sqrt_price_limit_x64` empty = no limit (the direction's bound).
#[wasm_bindgen]
pub fn quote_swap(
    pool_key: &[u8],
    pool: &[u8],
    bitmap: &[u8],
    tick_arrays: &[u8],
    zero_for_one: bool,
    amount_specified: &str,
    sqrt_price_limit_x64: &str,
) -> String {
    out(quote_swap_inner(pool_key, pool, bitmap, tick_arrays, zero_for_one, amount_specified, sqrt_price_limit_x64))
}

fn quote_swap_inner(
    pool_key: &[u8],
    pool: &[u8],
    bitmap: &[u8],
    tick_arrays: &[u8],
    zero_for_one: bool,
    amount_specified: &str,
    sqrt_price_limit_x64: &str,
) -> Result<String, QuoteError> {
    let key = key32(pool_key)?;
    let p = decode_pool(pool)?;
    if tick_arrays.len() % TickArray::ACCOUNT_LEN != 0 {
        return Err(QuoteError::QuoteBadAccount(TickArray::NAME));
    }
    let arrays: Vec<&[u8]> = tick_arrays.chunks_exact(TickArray::ACCOUNT_LEN).collect();
    let bm = if bitmap.is_empty() { None } else { Some(bitmap) };
    let set = TickArraySet::from_accounts(&key, p.tick_spacing, &arrays, bm)?;
    let limit = if sqrt_price_limit_x64.trim().is_empty() { carpenter_quote::no_limit(zero_for_one) } else { parse::<u128>(sqrt_price_limit_x64)? };
    let params = SwapQuoteParams { zero_for_one, amount_specified: parse::<i128>(amount_specified)?, sqrt_price_limit_x64: limit, supply0: 0, supply1: 0 };
    let q = quote_plain(&PoolAccount { key, pool: p }, &set, &params)?;
    Ok(swap_json(&q))
}

fn swap_json(q: &SwapQuote) -> String {
    let fees = Obj::new().str("lp", q.fees.lp).str("protocol", q.fees.protocol).num("protocol_currency", q.fees.protocol_currency).done();
    Obj::new()
        .str("amount_in", q.amount_in)
        .str("amount_out", q.amount_out)
        .raw("fees", &fees)
        .str("sqrt_after", q.sqrt_after)
        .num("tick_after", q.tick_after)
        .raw("arrays_touched", &ints(&q.arrays_touched))
        .raw("all_arrays", &ints(&q.all_arrays()))
        .done()
}

// ---- liquidity ----------------------------------------------------------------------------------

/// The token amounts of `liquidity_delta` over `[tick_lower, tick_upper]` at the given price (signed
/// like `modify_liquidity`'s delta: an add rounds up, a removal down).
#[wasm_bindgen]
pub fn amounts_for_liquidity(sqrt_price_x64: &str, tick_current: i32, tick_lower: i32, tick_upper: i32, liquidity_delta: &str) -> String {
    out((|| {
        let (a0, a1) = carpenter_quote::amounts_for_liquidity(parse(sqrt_price_x64)?, tick_current, tick_lower, tick_upper, parse(liquidity_delta)?)?;
        Ok(Obj::new().str("amount0", a0).str("amount1", a1).done())
    })())
}

/// The most liquidity the two maximum amounts buy over `[tick_lower, tick_upper]` at the given price.
#[wasm_bindgen]
pub fn quote_liquidity(sqrt_price_x64: &str, tick_lower: i32, tick_upper: i32, amount0_max: &str, amount1_max: &str) -> String {
    out((|| {
        let l = carpenter_quote::quote_liquidity(parse(sqrt_price_x64)?, tick_lower, tick_upper, parse(amount0_max)?, parse(amount1_max)?)?;
        Ok(Obj::new().str("liquidity", l).done())
    })())
}

/// Fees owed to a position: the pool's key and bytes, the position, and the arrays holding its two
/// ticks (the same account twice when both ticks share an array). Every account must name the pool:
/// a position of another pool is `QuoteBadAccount("Position")`, an array of another pool
/// `QuoteBadTickArray`.
#[wasm_bindgen]
pub fn fees_owed(pool_key: &[u8], pool: &[u8], position: &[u8], lower_array: &[u8], upper_array: &[u8]) -> String {
    out((|| {
        let key = key32(pool_key)?;
        let p = decode_pool(pool)?;
        let pos: Position = decode_account(position)?;
        if pos.pool != key {
            return Err(QuoteError::QuoteBadAccount(Position::NAME));
        }
        let lo: TickArray = decode_account(lower_array)?;
        let up: TickArray = decode_account(upper_array)?;
        if lo.pool != key || up.pool != key {
            return Err(QuoteError::QuoteBadTickArray);
        }
        let tick_of = |a: &TickArray, t: i32| -> Result<Tick, QuoteError> {
            let s = i32::from(p.tick_spacing);
            let i = t.checked_sub(a.start_index).filter(|d| *d >= 0 && d % s == 0).map(|d| d / s).filter(|i| *i < 60);
            i.map(|i| a.ticks[i as usize]).ok_or(QuoteError::QuoteBadTickArray)
        };
        let (f0, f1) = carpenter_quote::fees_owed(&p, &pos, &tick_of(&lo, pos.tick_lower)?, &tick_of(&up, pos.tick_upper)?);
        Ok(Obj::new().str("fees0", f0).str("fees1", f1).done())
    })())
}

// ---- price ↔ tick -------------------------------------------------------------------------------

#[wasm_bindgen]
pub fn sqrt_price_at_tick(tick: i32) -> String {
    out(price::sqrt_price_at_tick(tick).map(|s| Obj::new().str("sqrt_price_x64", s).done()))
}

#[wasm_bindgen]
pub fn tick_at_sqrt_price(sqrt_price_x64: &str) -> String {
    out((|| Ok(Obj::new().num("tick", price::tick_at_sqrt_price(parse(sqrt_price_x64)?)?).done()))())
}

#[wasm_bindgen]
pub fn tick_at_price_ui(price_ui: f64, decimals0: u8, decimals1: u8) -> String {
    out(price::tick_at_price_ui(price_ui, decimals0, decimals1).map(|t| Obj::new().num("tick", t).done()))
}

/// Display only. `NaN` for a tick out of range.
#[wasm_bindgen]
pub fn price_ui_at_tick(tick: i32, decimals0: u8, decimals1: u8) -> f64 {
    price::price_ui_at_tick(tick, decimals0, decimals1).unwrap_or(f64::NAN)
}

// ---- identity -----------------------------------------------------------------------------------

/// The crate version and the hook-interface version the quoter replays.
#[wasm_bindgen]
pub fn version() -> String {
    Obj::new()
        .str("carpenter_quote_wasm", env!("CARGO_PKG_VERSION"))
        .num("hook_interface", carpenter_quote::types::hook::HOOK_INTERFACE_VERSION)
        .done()
}

#[cfg(test)]
mod tests {
    use super::*;
    use carpenter_quote::state::{encode_account, encode_bitmap};
    use carpenter_quote::synth::{build_pool, PoolKeys, SynthKind, SynthPosition};
    use carpenter_quote::types::constants::{AFTER_SWAP_FLAG, PROTOCOL_FEE_DEFAULT_PACKED, PROTOCOL_FEE_ON_CURRENCY1};
    use carpenter_quote::types::U128;

    fn keys() -> PoolKeys {
        PoolKeys { pool: [1; 32], mint0: [2; 32], mint1: [3; 32], ..Default::default() }
    }

    #[test]
    fn json_shapes() {
        assert_eq!(err(QuoteError::TickArrayMissing { start_index: -3600 }), r#"{"err":"TickArrayMissing","start_index":-3600}"#);
        assert_eq!(ok(Obj::new().str("a", 1u64).num("b", -2).done()), r#"{"ok":{"a":"1","b":-2}}"#);
        assert!(sqrt_price_at_tick(0).starts_with(r#"{"ok":{"sqrt_price_x64":"18446744073709551616"}"#));
        assert_eq!(sqrt_price_at_tick(1_000_000), r#"{"err":"TickOutOfRange"}"#);
        assert_eq!(quote_swap(&[0; 3], &[], &[], &[], true, "-1", ""), r#"{"err":"QuoteBadAccount","account":"pool_key"}"#);
        assert!(version().starts_with(r#"{"carpenter_quote_wasm":"0.1.0","hook_interface":"#));
    }

    #[test]
    fn fees_owed_refuses_accounts_of_another_pool() {
        let k = keys();
        let sqrt = carpenter_quote::clmm::tick_math::get_sqrt_price_at_tick(0).unwrap();
        let sp = build_pool(&k, SynthKind::Plain { fee: 3_000 }, 60, sqrt, 9, 6, &[SynthPosition { tick_lower: -6_000, tick_upper: 6_000, liquidity: 1_000_000 }]).unwrap();
        let pool = encode_account(&sp.account.pool);
        let mut pos: Position = bytemuck::Zeroable::zeroed();
        pos.pool = [1; 32];
        pos.tick_lower = -6_000;
        pos.tick_upper = 6_000;
        pos.liquidity = U128::new(1_000_000);
        let lo = encode_account(&sp.arrays[0]);
        let up = encode_account(&sp.arrays[1]);
        assert_eq!(fees_owed(&[1; 32], &pool, &encode_account(&pos), &lo, &up), r#"{"ok":{"fees0":"0","fees1":"0"}}"#);
        let mut foreign = pos;
        foreign.pool = [9; 32];
        assert_eq!(fees_owed(&[1; 32], &pool, &encode_account(&foreign), &lo, &up), r#"{"err":"QuoteBadAccount","account":"Position"}"#);
        let mut stray = sp.arrays[1];
        stray.pool = [9; 32];
        assert_eq!(fees_owed(&[1; 32], &pool, &encode_account(&pos), &lo, &encode_account(&stray)), r#"{"err":"QuoteBadTickArray"}"#);
    }

    /// The swap JSON is `carpenter_quote::quote_plain`'s answer, field for field, the protocol fee included.
    #[test]
    fn the_swap_json_is_the_quoters_answer() {
        let k = keys();
        let sqrt = carpenter_quote::clmm::tick_math::get_sqrt_price_at_tick(0).unwrap();
        let positions = [SynthPosition { tick_lower: -6_000, tick_upper: 6_000, liquidity: 1_000_000_000_000 }];
        for (packed, code) in [(PROTOCOL_FEE_DEFAULT_PACKED, PROTOCOL_FEE_ON_CURRENCY1), (0, 0)] {
            let sp = build_pool(&k, SynthKind::Plain { fee: 3_000 }, 60, sqrt, 9, 6, &positions).unwrap().with_protocol_fee(packed, code);
            let pool = encode_account(&sp.account.pool);
            let arrays: Vec<u8> = sp.arrays.iter().flat_map(encode_account).collect();
            let bitmap = encode_bitmap(&k.pool, 60, &sp.words);
            let json = quote_swap(&k.pool, &pool, &bitmap, &arrays, true, "-1000000000", "");
            let want = quote_plain(
                &PoolAccount { key: k.pool, pool: sp.account.pool },
                &sp.tick_array_set().unwrap(),
                &SwapQuoteParams { zero_for_one: true, amount_specified: -1_000_000_000, sqrt_price_limit_x64: carpenter_quote::no_limit(true), supply0: 0, supply1: 0 },
            )
            .unwrap();
            assert_eq!(json, ok(swap_json(&want)));
            assert_eq!(want.fees.protocol > 0, packed != 0);
        }
    }

    /// A pool whose hook takes part in swaps is refused, never quoted as plain.
    #[test]
    fn a_swap_hooked_pool_is_refused() {
        let k = keys();
        let sqrt = carpenter_quote::clmm::tick_math::get_sqrt_price_at_tick(0).unwrap();
        let mut sp = build_pool(&k, SynthKind::Plain { fee: 3_000 }, 60, sqrt, 9, 6, &[SynthPosition { tick_lower: -600, tick_upper: 600, liquidity: 1_000_000 }]).unwrap();
        sp.account.pool.hook = [8; 32];
        sp.account.pool.hook_flags = AFTER_SWAP_FLAG;
        let arrays: Vec<u8> = sp.arrays.iter().flat_map(encode_account).collect();
        let json = quote_swap(&k.pool, &encode_account(&sp.account.pool), &encode_bitmap(&k.pool, 60, &sp.words), &arrays, true, "-1000", "");
        assert_eq!(json, r#"{"err":"QuoteHookNotQuotable"}"#);
    }
}
