//! A settling hook (`SETTLES_FLAG`, Uniswap v4's settle): the AMM-side rules the quoter replays — the
//! `before_swap` price limit that caps (or skips) the trader's walk, the `after_swap` fill of the
//! unfilled exact-in remainder and its bound, the vault mint credited to claims, the payout from
//! claims as a negative delta, and the refusals for each. The hook is a test double that fills the
//! remainder along a fixed rate, minting its output into the output vault.

use carpenter_quote::clmm::tick_math;
use carpenter_quote::synth::{build_pool, PoolKeys, SynthKind, SynthPool, SynthPosition};
use carpenter_quote::types::constants::{
    AFTER_SWAP_FLAG, AFTER_SWAP_RETURNS_DELTA_FLAG, BEFORE_SWAP_FLAG, BEFORE_SWAP_RETURNS_DELTA_FLAG, DYNAMIC_FEE_FLAG,
    SETTLES_FLAG,
};
use carpenter_quote::types::hook::{HookRecordV1, HOOK_INTERFACE_VERSION, PHASE_AFTER_SWAP, PHASE_BEFORE_SWAP};
use carpenter_quote::{no_limit, quote_plain, quote_swap, AfterSwapIn, HookQuoter, HookSwapCtx, QuoteError, QuoteResult, SwapQuoteParams};

const HOOK: [u8; 32] = [0x5A; 32];
const SETTLING: u16 = BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG | AFTER_SWAP_RETURNS_DELTA_FLAG | SETTLES_FLAG;

fn pool(hook: [u8; 32], flags: u16) -> SynthPool {
    let keys = PoolKeys {
        pool: [7; 32],
        mint0: [1; 32],
        mint1: [2; 32],
        vault0: [3; 32],
        vault1: [4; 32],
        token_program0: [5; 32],
        token_program1: [6; 32],
        hook,
        hook_authority: if hook == [0; 32] { [0; 32] } else { [9; 32] },
    };
    let kind = if hook == [0; 32] {
        SynthKind::Plain { fee: 3_000 }
    } else {
        SynthKind::Hooked { fee: DYNAMIC_FEE_FLAG, lp_fee: 3_000, flags, max_rounds: 3, callback_accounts: 1 }
    };
    let positions = [SynthPosition { tick_lower: -6_000, tick_upper: 6_000, liquidity: 10_000_000_000_000 }];
    build_pool(&keys, kind, 60, tick_math::get_sqrt_price_at_tick(0).unwrap(), 9, 6, &positions).unwrap()
}

fn params(zfo: bool, amount: i128) -> SwapQuoteParams {
    SwapQuoteParams { zero_for_one: zfo, amount_specified: amount, sqrt_price_limit_x64: no_limit(zfo), supply0: 0, supply1: 0 }
}

/// Caps the walk at `limit_tick` (0 = no limit), takes the unfilled exact-in remainder (plus
/// `overfill`), mints `out_per_in` × it into the output vault and pays it to the trader.
struct Curve {
    limit_tick: Option<i32>,
    before_specified: i128,
    out_per_in: i128,
    overfill: i128,
    /// Mint this much less than the payout (to run claims short).
    short: u64,
    minted: [u64; 2],
    settled_reads: u32,
}

impl Curve {
    fn new(limit_tick: Option<i32>) -> Self {
        Curve { limit_tick, before_specified: 0, out_per_in: 2, overfill: 0, short: 0, minted: [0, 0], settled_reads: 0 }
    }
}

impl HookQuoter for Curve {
    fn program_id(&self) -> [u8; 32] {
        HOOK
    }
    fn before_swap(&mut self, _ctx: &HookSwapCtx) -> QuoteResult<HookRecordV1> {
        let mut r = HookRecordV1::pass_through(HOOK_INTERFACE_VERSION, PHASE_BEFORE_SWAP, 0);
        r.delta_specified = self.before_specified;
        if let Some(t) = self.limit_tick {
            r.sqrt_price_limit_x64 = tick_math::get_sqrt_price_at_tick(t).unwrap();
        }
        Ok(r)
    }
    fn after_swap(&mut self, ctx: &HookSwapCtx, after: &AfterSwapIn) -> QuoteResult<HookRecordV1> {
        let mut r = HookRecordV1::pass_through(HOOK_INTERFACE_VERSION, PHASE_AFTER_SWAP, 1);
        let to_swap = ctx.amount_specified + after.before_delta_specified;
        let consumed = if ctx.zero_for_one { after.swap_delta.amount0 } else { after.swap_delta.amount1 }.unsigned_abs() as i128;
        let unfilled = if ctx.amount_specified < 0 { to_swap.abs() - consumed } else { 0 };
        let take = unfilled + self.overfill;
        let pay = take * self.out_per_in;
        r.delta_specified = take;
        r.delta_unspecified = -pay;
        let mint = (pay as u64).saturating_sub(self.short);
        if ctx.zero_for_one {
            self.minted[1] = mint;
        } else {
            self.minted[0] = mint;
        }
        Ok(r)
    }
    fn settled(&mut self) -> [u64; 2] {
        self.settled_reads += 1;
        std::mem::take(&mut self.minted)
    }
}

#[test]
fn the_hooks_limit_caps_the_walk_and_the_hook_fills_the_rest_out_of_its_mint() {
    let sp = pool(HOOK, SETTLING);
    let set = sp.tick_array_set().unwrap();
    let limit = tick_math::get_sqrt_price_at_tick(-60).unwrap();
    let mut h = Curve::new(Some(-60));
    let amount = -100_000_000_000i128; // far more than the walk to tick -60 takes
    let q = quote_swap(&sp.account, &set, Some(&mut h), &params(true, amount)).unwrap();
    // the walk stopped at the hook's limit, exactly as a plain walk to that limit does
    let plain = pool([0; 32], 0);
    let p = quote_plain(&plain.account, &plain.tick_array_set().unwrap(), &SwapQuoteParams { sqrt_price_limit_x64: limit, ..params(true, amount) })
        .unwrap();
    assert_eq!(q.sqrt_after_trade, limit);
    assert_eq!(q.swap_delta, p.swap_delta);
    let consumed = (-q.swap_delta.amount0) as u64;
    let unfilled = amount.unsigned_abs() as u64 - consumed;
    // the trader pays the whole exact-in and receives the walk's output plus the hook's fill
    assert_eq!(q.amount_in, amount.unsigned_abs() as u64);
    assert_eq!(q.amount_out, p.amount_out + 2 * unfilled);
    assert_eq!((q.hook_delta.amount0, q.hook_delta.amount1), (unfilled as i128, -2 * unfilled as i128));
    assert_eq!(q.settled, [0, 2 * unfilled]);
    // claims: the taken input stays; the mint is paid straight out; only what the hook took is a fee
    assert_eq!((q.pool_after.hook_claims0, q.pool_after.hook_claims1), (unfilled, 0));
    assert_eq!((q.pool_after.hook_fees0.get(), q.pool_after.hook_fees1.get()), (unfilled as u128, 0));
    assert_eq!(h.settled_reads, 2, "read after before_swap and after_swap");
}

#[test]
fn a_limit_the_price_already_sits_at_or_past_skips_the_walk() {
    let sp = pool(HOOK, SETTLING);
    let set = sp.tick_array_set().unwrap();
    for (zfo, tick) in [(true, 0), (true, 60), (false, 0), (false, -60)] {
        let mut h = Curve::new(Some(tick));
        let q = quote_swap(&sp.account, &set, Some(&mut h), &params(zfo, -1_000_000)).unwrap();
        assert_eq!((q.swap_delta.amount0, q.swap_delta.amount1), (0, 0), "zfo {zfo} tick {tick}");
        assert!(q.arrays_touched.is_empty());
        assert_eq!((q.amount_in, q.amount_out), (1_000_000, 2_000_000));
        assert_eq!(q.sqrt_after, sp.account.pool.sqrt_price_x64.get());
    }
}

#[test]
fn the_after_swap_fill_is_bounded_by_the_unfilled_exact_in() {
    let sp = pool(HOOK, SETTLING);
    let set = sp.tick_array_set().unwrap();
    let mut h = Curve::new(Some(-60));
    h.overfill = 1;
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, -100_000_000_000)).unwrap_err(), QuoteError::HookSettleOverfill);
    assert_eq!(QuoteError::HookSettleOverfill.name(), "HookSettleOverfill");
    // never on an exact-out
    let mut h = Curve::new(None);
    h.overfill = 1;
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, 1_000)).unwrap_err(), QuoteError::HookSettleOverfill);
}

#[test]
fn a_payout_beyond_the_hooks_claims_is_refused() {
    let sp = pool(HOOK, SETTLING);
    let set = sp.tick_array_set().unwrap();
    let mut h = Curve::new(Some(0));
    h.short = 1;
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, -1_000)).unwrap_err(), QuoteError::HookClaimsInsufficient);
}

#[test]
fn a_negative_specified_delta_may_not_enlarge_the_walk() {
    let flags = SETTLING | BEFORE_SWAP_RETURNS_DELTA_FLAG;
    let sp = pool(HOOK, flags);
    let set = sp.tick_array_set().unwrap();
    let mut h = Curve::new(None);
    h.before_specified = -1;
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, -1_000)).unwrap_err(), QuoteError::HookDeltaExceedsSwapAmount);
}

#[test]
fn without_the_flag_a_limit_and_an_after_swap_take_are_refused() {
    let flags = SETTLING & !SETTLES_FLAG;
    let sp = pool(HOOK, flags);
    let set = sp.tick_array_set().unwrap();
    // a price limit is a field the AMM refuses without SETTLES
    let mut h = Curve::new(Some(-60));
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, -1_000)).unwrap_err(), QuoteError::HookFieldNotAllowed);
    // an after_swap specified take is refused too
    let mut h = Curve::new(None);
    h.overfill = 1;
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, -1_000)).unwrap_err(), QuoteError::HookFieldNotAllowed);
    assert_eq!(h.settled_reads, 0, "settled() is read only for a settling hook");
}
