//! `cargo test -p carpenter-quote`: the plain-pool tier.
//!
//! What a chain-free test can prove: the walk against closed forms from `carpenter-clmm-math`, the
//! walk's own bookkeeping (fee growth, crossings, the arrays it names), the refusals by name, and
//! determinism. Hooked pools are tested in the hook's crate (FLOOR's: `floor-quote`); the hook
//! dispatch itself is `tests/hook_dispatch.rs`.

use carpenter_quote::clmm::{liquidity_math, tick_math};
use carpenter_quote::synth::{build_pool, PoolKeys, SynthKind, SynthPool, SynthPosition};
use carpenter_quote::types::constants::{MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64};
use carpenter_quote::{no_limit, quote_swap, QuoteError, SwapQuoteParams, TickArraySet};

fn keys(hook: [u8; 32]) -> PoolKeys {
    PoolKeys {
        pool: [7; 32],
        mint0: [1; 32],
        mint1: [2; 32],
        vault0: [3; 32],
        vault1: [4; 32],
        token_program0: [5; 32],
        token_program1: [6; 32],
        hook,
        hook_authority: if hook == [0; 32] { [0; 32] } else { [9; 32] },
    }
}

fn plain(fee: u32, spacing: u16, tick: i32, positions: &[SynthPosition]) -> SynthPool {
    let sqrt = tick_math::get_sqrt_price_at_tick(tick).unwrap();
    build_pool(&keys([0; 32]), SynthKind::Plain { fee }, spacing, sqrt, 9, 6, positions).unwrap()
}

fn params(zfo: bool, amount: i128) -> SwapQuoteParams {
    SwapQuoteParams { zero_for_one: zfo, amount_specified: amount, sqrt_price_limit_x64: no_limit(zfo), supply0: 0, supply1: 1_000_000_000_000_000 }
}

fn pos(lo: i32, up: i32, l: u128) -> SynthPosition {
    SynthPosition { tick_lower: lo, tick_upper: up, liquidity: l }
}

#[test]
fn exact_in_inside_one_range_matches_the_closed_form() {
    let sp = plain(3_000, 60, 0, &[pos(-6_000, 6_000, 10_000_000_000_000)]);
    let set = sp.tick_array_set().unwrap();
    let q = quote_swap(&sp.account, &set, None, &params(true, -1_000_000)).unwrap();
    assert_eq!(q.amount_in, 1_000_000);
    // v4 split: the fee-less amount moves the price, the rest is the fee.
    let less_fee = 1_000_000u64 * (1_000_000 - 3_000) / 1_000_000;
    assert_eq!(q.fees.lp, 1_000_000 - less_fee);
    let start = tick_math::get_sqrt_price_at_tick(0).unwrap();
    let out = liquidity_math::get_delta_amount_1_unsigned(q.sqrt_after, start, 10_000_000_000_000, false).unwrap();
    assert_eq!(q.amount_out, out);
    assert!(q.sqrt_after < start);
    // Array 0 holds no initialized tick (unmarked, never read); the next marked array down holds -6,000.
    assert_eq!(q.arrays_touched, vec![-7_200]);
    assert_eq!(q.pool_after.swap_count, 1);
    assert_eq!(q.pool_after.volume0.get(), 1_000_000);
    assert_eq!(q.pool_after.lp_fees0.get(), q.fees.lp as u128);
    let growth = ((q.fees.lp as u128) << 64) / 10_000_000_000_000u128;
    assert_eq!(q.pool_after.fee_growth_global0_x64.get(), growth);
}

#[test]
fn crossing_a_tick_flips_outside_growth_and_changes_liquidity() {
    // Two ranges: [-600, 600) deep, [-60000, 60000) wide. A sell of ~1.5e11 empties the inner range to
    // its lower tick; 2e11 crosses it and stops well inside the wide one.
    let sp = plain(500, 60, 0, &[pos(-600, 600, 5_000_000_000_000), pos(-60_000, 60_000, 1_000_000_000_000)]);
    let set = sp.tick_array_set().unwrap();
    let q = quote_swap(&sp.account, &set, None, &params(true, -200_000_000_000)).unwrap();
    assert!(q.tick_after < -600 && q.tick_after > -60_000, "crossed -600 only: {}", q.tick_after);
    assert_eq!(q.pool_after.liquidity.get(), 1_000_000_000_000);
    let arr = q.arrays_after.iter().find(|a| a.start_index == -3_600).expect("array holding -600");
    let t = &arr.ticks[((-600 - -3_600) / 60) as usize];
    assert_ne!(t.fee_growth_outside0_x64.get(), 0, "outside growth flipped to the in-flight global");
}

#[test]
fn exact_out_never_charges_more_than_the_exact_in_that_produced_it() {
    let sp = plain(3_000, 10, 1_234, &[pos(-10_000, 10_000, 77_000_000_000), pos(1_000, 2_000, 9_000_000_000_000)]);
    let set = sp.tick_array_set().unwrap();
    for amount in [1_000u64, 123_456, 9_999_999, 5_000_000_000] {
        for zfo in [true, false] {
            let a = quote_swap(&sp.account, &set, None, &params(zfo, -(amount as i128))).unwrap();
            if a.amount_out == 0 {
                continue;
            }
            let b = quote_swap(&sp.account, &set, None, &params(zfo, a.amount_out as i128)).unwrap();
            assert_eq!(b.amount_out, a.amount_out);
            // Each step rounds against the trader on both routes, so the inverse may differ by a few
            // units per step; it must never be materially dearer.
            assert!(b.amount_in <= a.amount_in + 4, "zfo {zfo} amount {amount}: {} > {}", b.amount_in, a.amount_in);
        }
    }
}

#[test]
fn zero_liquidity_walks_to_the_limit_and_moves_nothing() {
    let sp = plain(3_000, 60, 0, &[]);
    let set = sp.tick_array_set().unwrap();
    let limit = tick_math::get_sqrt_price_at_tick(-120).unwrap();
    let p = SwapQuoteParams { sqrt_price_limit_x64: limit, ..params(true, -1_000) };
    let q = quote_swap(&sp.account, &set, None, &p).unwrap();
    assert_eq!((q.amount_in, q.amount_out), (0, 0));
    assert_eq!(q.sqrt_after, limit);
}

#[test]
fn r12_a_full_fee_refuses_exact_out_and_eats_exact_in() {
    let sp = plain(1_000_000, 60, 0, &[pos(-600, 600, 1_000_000_000_000)]);
    let set = sp.tick_array_set().unwrap();
    assert_eq!(quote_swap(&sp.account, &set, None, &params(true, 1_000)).unwrap_err(), QuoteError::InvalidFeeForExactOut);
    let q = quote_swap(&sp.account, &set, None, &params(true, -1_000)).unwrap();
    assert_eq!((q.amount_in, q.amount_out, q.fees.lp), (1_000, 0, 1_000));
}

#[test]
fn refusals_carry_the_chain_names() {
    let sp = plain(3_000, 60, 0, &[pos(-600, 600, 1_000_000_000_000)]);
    let set = sp.tick_array_set().unwrap();
    assert_eq!(quote_swap(&sp.account, &set, None, &params(true, 0)).unwrap_err().name(), "AmountZero");
    let wrong_side = SwapQuoteParams { sqrt_price_limit_x64: MAX_SQRT_PRICE_X64 - 1, ..params(true, -5) };
    assert_eq!(quote_swap(&sp.account, &set, None, &wrong_side).unwrap_err().name(), "PriceLimitInvalid");
    let at_bound = SwapQuoteParams { sqrt_price_limit_x64: MIN_SQRT_PRICE_X64, ..params(true, -5) };
    assert_eq!(quote_swap(&sp.account, &set, None, &at_bound).unwrap_err().name(), "PriceLimitInvalid");
    let too_big = params(false, -(u64::MAX as i128) - 1);
    let e = quote_swap(&sp.account, &set, None, &too_big).unwrap_err();
    assert_eq!(e, QuoteError::AmountTooLarge);
    // The prelude bound names no chain error and the AMM has no `AmountTooLarge`, so it reports under
    // the name the chain gives a checked u64 narrowing (see error.rs).
    assert_eq!(e.name(), "MathOverflow");
}

#[test]
fn a_marked_array_that_was_not_supplied_is_named() {
    let sp = plain(3_000, 60, 0, &[pos(-7_200, 7_200, 1_000_000_000_000), pos(-600, 600, 1_000_000_000_000)]);
    // Drop the array holding -7,200 but keep the bitmap.
    let arrays: Vec<_> = sp.arrays.iter().copied().filter(|a| a.start_index != -7_200).collect();
    let set = TickArraySet::new(&sp.account.key, 60, arrays, Some(sp.words.clone())).unwrap();
    let err = quote_swap(&sp.account, &set, None, &params(true, -(u64::MAX as i128))).unwrap_err();
    assert_eq!(err, QuoteError::TickArrayMissing { start_index: -7_200 });
}

#[test]
fn quoting_twice_is_identical_and_sequential_quotes_compose() {
    let sp = plain(3_000, 60, 60, &[pos(-6_000, 6_000, 10_000_000_000_000), pos(-120, 240, 3_000_000_000_000)]);
    let set = sp.tick_array_set().unwrap();
    let a = quote_swap(&sp.account, &set, None, &params(false, -777_777)).unwrap();
    let b = quote_swap(&sp.account, &set, None, &params(false, -777_777)).unwrap();
    assert_eq!(bytemuck::bytes_of(&a.pool_after), bytemuck::bytes_of(&b.pool_after));
    assert_eq!((a.amount_in, a.amount_out), (b.amount_in, b.amount_out));

    // Feed the post-state back in: the second quote starts where the first ended.
    let mut next = sp.account;
    next.pool = a.pool_after;
    let mut arrays = set.arrays.clone();
    for arr in &a.arrays_after {
        arrays.insert(arr.start_index, *arr);
    }
    let set2 = TickArraySet::new(&next.key, 60, arrays.into_values().collect(), Some(set.words.clone())).unwrap();
    let c = quote_swap(&next, &set2, None, &params(true, -1_000)).unwrap();
    assert_eq!(c.pool_after.swap_count, 2);
}

// ---- the Carpenter Platform fee ----------------------------------------------------------------
//
// A small fee on top of the LP fee, on every pool, on the SOL / quote side: the pool's `protocol_fee`
// (copied at `initialize_pool`, 500 | 500 << 12 by default) is charged on every walk, in the pool's
// `protocol_fee_currency`. These pin the quoter's half.

mod carpenter_platform_fee {
    use super::*;
    use carpenter_quote::types::constants::{
        PROTOCOL_FEE_DEFAULT_PACKED, PROTOCOL_FEE_ON_CURRENCY0, PROTOCOL_FEE_ON_CURRENCY1, PROTOCOL_FEE_ON_INPUT,
    };
    use carpenter_quote::types::protocol_fee::{compose_swap_fee, gross_for_net, output_share};

    const L: u128 = 10_000_000_000_000;

    fn pool_with(fee: u32, packed: u32, code: u8) -> SynthPool {
        plain(fee, 60, 0, &[pos(-6_000, 6_000, L)]).with_protocol_fee(packed, code)
    }

    /// v4 exactly on the input: one step inside one range, so the gross input IS the amount and the
    /// protocol's share is `floor(amount · 500 / 1e6)`; `fees.lp` and fee growth are the LPs' half;
    /// the total is what a protocol-free pool at the COMPOSED rate charges; owed lands on the input.
    #[test]
    fn on_the_input_it_is_v4s_composed_split() {
        let amount = 1_000_000_000i128;
        for (zfo, code) in [
            (true, PROTOCOL_FEE_ON_INPUT),
            (false, PROTOCOL_FEE_ON_INPUT),
            (true, PROTOCOL_FEE_ON_CURRENCY0),
            (false, PROTOCOL_FEE_ON_CURRENCY1),
        ] {
            let sp = pool_with(3_000, PROTOCOL_FEE_DEFAULT_PACKED, code);
            let q = quote_swap(&sp.account, &sp.tick_array_set().unwrap(), None, &params(zfo, -amount)).unwrap();
            assert_eq!(q.fees.protocol, 500_000, "zfo {zfo} code {code}: floor(1e9 · 500 / 1e6)");
            assert_eq!(q.fees.protocol_currency, if zfo { 0 } else { 1 }, "the input currency");
            let composed = compose_swap_fee(500, 3_000);
            assert_eq!(composed, 3_499);
            let flat = plain(composed, 60, 0, &[pos(-6_000, 6_000, L)]);
            let f = quote_swap(&flat.account, &flat.tick_array_set().unwrap(), None, &params(zfo, -amount)).unwrap();
            assert_eq!(q.amount_out, f.amount_out, "the composed rate moves the price");
            assert_eq!(q.fees.lp + q.fees.protocol, f.fees.lp, "lp + protocol == the step fee");
            let (owed_in, owed_out) = if zfo {
                (q.pool_after.protocol_fees_owed0, q.pool_after.protocol_fees_owed1)
            } else {
                (q.pool_after.protocol_fees_owed1, q.pool_after.protocol_fees_owed0)
            };
            assert_eq!((owed_in, owed_out), (q.fees.protocol, 0));
            let growth = if zfo { q.pool_after.fee_growth_global0_x64.get() } else { q.pool_after.fee_growth_global1_x64.get() };
            assert_eq!(growth, ((q.fees.lp as u128) << 64) / L, "fee growth is the LP half only");
            let lp_counter = if zfo { q.pool_after.lp_fees0.get() } else { q.pool_after.lp_fees1.get() };
            assert_eq!(lp_counter, q.fees.lp as u128);
        }
    }

    /// "SOL / quote side": when the listed quote is what the swap RECEIVES, the steps run at the LP fee
    /// alone and `floor(gross_out · 500 / 1e6)` comes off the output, owed in the quote.
    #[test]
    fn on_the_output_it_comes_off_what_the_seller_receives() {
        for (zfo, code) in [(false, PROTOCOL_FEE_ON_CURRENCY0), (true, PROTOCOL_FEE_ON_CURRENCY1)] {
            let sp = pool_with(3_000, PROTOCOL_FEE_DEFAULT_PACKED, code);
            let q = quote_swap(&sp.account, &sp.tick_array_set().unwrap(), None, &params(zfo, -1_000_000_000)).unwrap();
            let bare = pool_with(3_000, 0, code);
            let b = quote_swap(&bare.account, &bare.tick_array_set().unwrap(), None, &params(zfo, -1_000_000_000)).unwrap();
            let want = output_share(b.amount_out, 500);
            assert!(want > 0);
            assert_eq!(q.fees.protocol, want, "zfo {zfo}: floor(gross_out · 500 / 1e6)");
            assert_eq!(q.amount_out, b.amount_out - want, "zfo {zfo}: net of it");
            assert_eq!(q.amount_in, b.amount_in);
            assert_eq!(q.fees.lp, b.fees.lp, "the LPs are paid exactly what a fee-free walk pays them");
            assert_eq!(q.sqrt_after, b.sqrt_after, "the fee never enters the curve");
            let quote_currency = if code == PROTOCOL_FEE_ON_CURRENCY0 { 0u8 } else { 1u8 };
            assert_eq!(q.fees.protocol_currency, quote_currency);
            let owed = [q.pool_after.protocol_fees_owed0, q.pool_after.protocol_fees_owed1];
            assert_eq!(owed[quote_currency as usize], want);
            assert_eq!(owed[1 - quote_currency as usize], 0);
        }
    }

    /// An exact-out through the output fee delivers exactly what it named, and costs what a fee-free
    /// exact-out of the grossed-up amount costs.
    #[test]
    fn an_exact_out_through_the_output_fee_nets_exactly() {
        for named in [1i128, 1_999, 2_000, 1_000_000, 123_456_789] {
            let sp = pool_with(3_000, PROTOCOL_FEE_DEFAULT_PACKED, PROTOCOL_FEE_ON_CURRENCY0);
            let q = quote_swap(&sp.account, &sp.tick_array_set().unwrap(), None, &params(false, named)).unwrap();
            assert_eq!(q.amount_out as i128, named);
            let gross = gross_for_net(named as u64, 500).unwrap();
            assert_eq!(q.fees.protocol, gross - named as u64);
            let bare = pool_with(3_000, 0, PROTOCOL_FEE_ON_CURRENCY0);
            let b = quote_swap(&bare.account, &bare.tick_array_set().unwrap(), None, &params(false, gross as i128)).unwrap();
            assert_eq!(q.amount_in, b.amount_in, "named {named}");
        }
    }

    /// A pool whose protocol fee is 0 quotes byte for byte as a pool with no protocol fee does,
    /// whatever its currency code.
    #[test]
    fn a_zero_rate_quotes_exactly_as_before() {
        for code in [PROTOCOL_FEE_ON_INPUT, PROTOCOL_FEE_ON_CURRENCY0, PROTOCOL_FEE_ON_CURRENCY1] {
            for zfo in [true, false] {
                let sp = pool_with(3_000, 0, code);
                let q = quote_swap(&sp.account, &sp.tick_array_set().unwrap(), None, &params(zfo, -1_000_000_000)).unwrap();
                let old = plain(3_000, 60, 0, &[pos(-6_000, 6_000, L)]);
                let o = quote_swap(&old.account, &old.tick_array_set().unwrap(), None, &params(zfo, -1_000_000_000)).unwrap();
                assert_eq!(q.fees.protocol, 0);
                assert_eq!((q.amount_in, q.amount_out, q.fees.lp), (o.amount_in, o.amount_out, o.fees.lp));
                let mut p = q.pool_after;
                p.protocol_fee_currency = 0;
                assert_eq!(bytemuck::bytes_of(&p), bytemuck::bytes_of(&o.pool_after), "code {code} zfo {zfo}");
            }
        }
    }
}

/// The program's swap refuses a paused pool with `PoolPaused` before the amount checks: a paused pool
/// is never quoted, and the refusal outranks `AmountZero`. The flag alone decides; a guardian window
/// (`guardian_paused_until`) needs a clock and is not judged by the quoter.
#[test]
fn a_paused_pool_is_refused_pool_paused_ahead_of_the_amount_checks() {
    let mut sp = plain(3_000, 60, 0, &[pos(-600, 600, 1_000_000_000_000)]);
    let set = sp.tick_array_set().unwrap();
    assert!(quote_swap(&sp.account, &set, None, &params(true, -1_000)).is_ok());
    sp.account.pool.paused = 1;
    for amount in [-1_000, 1_000, 0] {
        let e = quote_swap(&sp.account, &set, None, &params(true, amount)).unwrap_err();
        assert_eq!((e, e.name()), (QuoteError::PoolPaused, "PoolPaused"), "amount {amount}");
    }
    sp.account.pool.paused = 0;
    sp.account.pool.guardian_paused_until = i64::MAX;
    assert!(quote_swap(&sp.account, &set, None, &params(true, -1_000)).is_ok(), "the guardian window is not the quoter's to judge");
}
