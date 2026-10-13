// SPDX-License-Identifier: Apache-2.0 AND MIT
// Copyright 2026 FLOOR; the transcribed v4-core constants and formulas are
// Copyright 2023 Universal Navigation Inc. (MIT, licence text in ../NOTICE).
//
// New in FLOOR, 2026. The oracles below re-express Uniswap v4-core @ 46c6834
// `src/libraries/{TickMath,SqrtPriceMath,SwapMath}.sol` (SPDX MIT; licence text in ../NOTICE) at Q64.64 in
// exact 512-bit integers. They are an independent path: they share no function with the crate under test
// except the `U256`/`U512` integer types.
//
// What "parity" means here:
// - amounts and next-price: bit-identical to v4's rounding at the same Q64 inputs;
// - one swap step on the fee-on-input path (the only path the AMM calls): bit-identical to
//   `computeSwapStep`, including the LP-fee split, wherever v4's amounts fit u64;
// - tick → sqrt price: Raydium's 64-bit magic constants against v4's 128-bit ones, a PRECISION class
//   measured and bounded; a v4 price decodes through our inverse to its tick or the one below,
//   and the counts are pinned.

use carpenter_clmm_math::big_num::{U256, U512};
use carpenter_clmm_math::error::MathError;
use carpenter_clmm_math::liquidity_math::{get_delta_amount_0_unsigned, get_delta_amount_1_unsigned};
use carpenter_clmm_math::sqrt_price_math::{
    get_next_sqrt_price_from_input, get_next_sqrt_price_from_output,
};
use carpenter_clmm_math::swap_math::{compute_swap, FEE_RATE_DENOMINATOR_VALUE};
use carpenter_clmm_math::tick_math::{
    get_sqrt_price_at_tick, get_tick_at_sqrt_price, MAX_SQRT_PRICE_X64, MAX_TICK,
    MIN_SQRT_PRICE_X64, MIN_TICK,
};
use proptest::prelude::*;
use proptest::test_runner::RngSeed;

const MAX_FEE: u32 = FEE_RATE_DENOMINATOR_VALUE;

fn u(x: u128) -> U512 {
    U512::from(x)
}
fn ceil_div(n: U512, d: U512) -> U512 {
    let q = n / d;
    if n % d > U512::zero() {
        q + U512::one()
    } else {
        q
    }
}
fn fits_u64(x: U512) -> Option<u64> {
    if x > U512::from(u64::MAX) {
        None
    } else {
        Some(x.low_u64())
    }
}
fn fits_u128(x: U512) -> Option<u128> {
    if x > U512::from(u128::MAX) {
        None
    } else {
        Some(x.low_u128())
    }
}

// ---- SqrtPriceMath.sol, Q64 ----------------------------------------------------------------------

/// `getAmount0Delta`: `ceil|floor( (L << 64)·(b − a) / (a·b) )`. `None` = v4 reverts (`InvalidPrice`).
fn v4_amount0(mut a: u128, mut b: u128, l: u128, up: bool) -> Option<U512> {
    if a > b {
        core::mem::swap(&mut a, &mut b);
    }
    if a == 0 {
        return None;
    }
    let n = (u(l) << 64) * u(b - a);
    let d = u(a) * u(b);
    Some(if up { ceil_div(n, d) } else { n / d })
}

/// `getAmount1Delta`: `ceil|floor( L·(b − a) / 2^64 )`.
fn v4_amount1(a: u128, b: u128, l: u128, up: bool) -> U512 {
    let diff = if a > b { a - b } else { b - a };
    let n = u(l) * u(diff);
    if up {
        ceil_div(n, U512::one() << 64)
    } else {
        n >> 64
    }
}

/// `getNextSqrtPriceFromAmount0RoundingUp`. At Q64 in 512 bits the product never overflows, so the
/// first branch is the whole function. `None` = v4 reverts (`PriceOverflow`) or the result overflows
/// the price type (v4's `toUint160`, here u128).
fn v4_next_from_amount0_up(p: u128, l: u128, amount: u64, add: bool) -> Option<u128> {
    if amount == 0 {
        return Some(p);
    }
    let n1 = u(l) << 64;
    let product = U512::from(amount) * u(p);
    if add {
        fits_u128(ceil_div(n1 * u(p), n1 + product))
    } else {
        if n1 <= product {
            return None;
        }
        fits_u128(ceil_div(n1 * u(p), n1 - product))
    }
}

/// `getNextSqrtPriceFromAmount1RoundingDown`. `None` = v4 reverts (`NotEnoughLiquidity`, or overflow).
fn v4_next_from_amount1_down(p: u128, l: u128, amount: u64, add: bool) -> Option<u128> {
    let n = U512::from(amount) << 64;
    if add {
        fits_u128(u(p) + n / u(l))
    } else {
        let q = ceil_div(n, u(l));
        if u(p) <= q {
            return None;
        }
        fits_u128(u(p) - q)
    }
}

fn v4_next_from_input(p: u128, l: u128, amount: u64, zfo: bool) -> Option<u128> {
    if p == 0 || l == 0 {
        return None;
    }
    if zfo {
        v4_next_from_amount0_up(p, l, amount, true)
    } else {
        v4_next_from_amount1_down(p, l, amount, true)
    }
}

fn v4_next_from_output(p: u128, l: u128, amount: u64, zfo: bool) -> Option<u128> {
    if p == 0 || l == 0 {
        return None;
    }
    if zfo {
        v4_next_from_amount1_down(p, l, amount, false)
    } else {
        v4_next_from_amount0_up(p, l, amount, false)
    }
}

// ---- SwapMath.sol computeSwapStep, Q64 -------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum V4Step {
    /// (sqrt_next, amount_in, amount_out, fee), every amount within u64
    Ok(u128, u64, u64, u64),
    /// v4 reverts, or an amount exceeds u64 (a Solana token amount); the crate must return `Err`
    Fails,
}

/// `computeSwapStep` with `amountRemaining < 0` for exact-in. `zeroForOne = current >= target`.
fn v4_compute_swap_step(
    current: u128,
    target: u128,
    l: u128,
    remaining: u64,
    exact_in: bool,
    fee: u32,
) -> V4Step {
    let zfo = current >= target;
    let fee = fee as u128;
    let max = MAX_FEE as u128;
    let delta0 = |a, b, up| v4_amount0(a, b, l, up);
    let step = || -> Option<(u128, U512, U512, U512)> {
        if exact_in {
            let less_fee = U512::from(remaining) * u(max - fee) / u(max);
            let less_fee_u64 = less_fee.low_u64();
            let in_at_target = if zfo {
                delta0(target, current, true)?
            } else {
                v4_amount1(current, target, l, true)
            };
            let (next, amount_in, fee_amount);
            if less_fee >= in_at_target {
                next = target;
                amount_in = in_at_target;
                fee_amount = if fee == max {
                    amount_in
                } else {
                    ceil_div(amount_in * u(fee), u(max - fee))
                };
            } else {
                amount_in = less_fee;
                next = v4_next_from_input(current, l, less_fee_u64, zfo)?;
                fee_amount = U512::from(remaining) - amount_in;
            }
            let out = if zfo {
                v4_amount1(next, current, l, false)
            } else {
                delta0(current, next, false)?
            };
            Some((next, amount_in, out, fee_amount))
        } else {
            let out_at_target = if zfo {
                v4_amount1(target, current, l, false)
            } else {
                delta0(current, target, false)?
            };
            let (next, out);
            if U512::from(remaining) >= out_at_target {
                next = target;
                out = out_at_target;
            } else {
                out = U512::from(remaining);
                next = v4_next_from_output(current, l, remaining, zfo)?;
            }
            let amount_in = if zfo {
                delta0(next, current, true)?
            } else {
                v4_amount1(current, next, l, true)
            };
            // v4: "feePips cannot be MAX_SWAP_FEE for exact out"
            if fee == max {
                return None;
            }
            let fee_amount = ceil_div(amount_in * u(fee), u(max - fee));
            Some((next, amount_in, out, fee_amount))
        }
    };
    match step() {
        None => V4Step::Fails,
        Some((next, i, o, f)) => match (fits_u64(i), fits_u64(o), fits_u64(f)) {
            (Some(i), Some(o), Some(f)) => V4Step::Ok(next, i, o, f),
            _ => V4Step::Fails,
        },
    }
}

fn crate_step(
    current: u128,
    target: u128,
    l: u128,
    remaining: u64,
    exact_in: bool,
    fee: u32,
) -> V4Step {
    let zfo = current >= target;
    match compute_swap(current, target, l, remaining, fee, exact_in, zfo, true) {
        Ok(r) => V4Step::Ok(
            r.sqrt_price_next_x64,
            r.amount_in,
            r.amount_out,
            r.fee_amount,
        ),
        Err(_) => V4Step::Fails,
    }
}

/// Named difference ND-EXACT-OUT-DUST (upstream behaviour, kept): on a PARTIAL exact-output step v4 books
/// `amountOut = amountRemaining` without re-deriving it, while upstream re-derives both amounts from the
/// rounded next price and refuses with `MaxTokenOverflow` if the re-derived output leaves u64 (it is then
/// capped to `amount_remaining` only when it fits). That happens only where one Q64 unit of price is
/// worth more than what is left of u64 above the request: token0 out needs `L > sqrt_price_x64²` (below
/// price 1, large liquidity); token1 out needs `amount_remaining + L/2^64 ≥ u64::MAX`. The boundary grid
/// asserts both regions and prints the counts. The crate refuses; it
/// never returns a different successful answer. Returns true when a case is exactly that.
fn is_exact_out_rederived_overflow(
    current: u128,
    target: u128,
    l: u128,
    remaining: u64,
    exact_in: bool,
    fee: u32,
    v4: &V4Step,
) -> bool {
    let V4Step::Ok(next, ..) = *v4 else {
        return false;
    };
    if exact_in || next == target {
        return false;
    }
    let zfo = current >= target;
    let rederived_out = if zfo {
        v4_amount1(next, current, l, false)
    } else {
        v4_amount0(current, next, l, false).unwrap()
    };
    fits_u64(rederived_out).is_none()
        && compute_swap(current, target, l, remaining, fee, false, zfo, true).unwrap_err()
            == MathError::MaxTokenOverflow
}

/// Asserts parity; returns true when the case was accepted as ND-EXACT-OUT-DUST instead.
fn assert_step_parity(
    current: u128,
    target: u128,
    l: u128,
    remaining: u64,
    exact_in: bool,
    fee: u32,
) -> bool {
    let v4 = v4_compute_swap_step(current, target, l, remaining, exact_in, fee);
    let ours = crate_step(current, target, l, remaining, exact_in, fee);
    if ours == V4Step::Fails
        && is_exact_out_rederived_overflow(current, target, l, remaining, exact_in, fee, &v4)
    {
        return true;
    }
    match (&v4, &ours) {
        (V4Step::Ok(..), _) => assert_eq!(
            ours, v4,
            "step parity: current={current} target={target} L={l} remaining={remaining} exact_in={exact_in} fee={fee}"
        ),
        // Where v4's own amounts leave u64 the step cannot settle on Solana; the crate must not
        // invent a different successful answer.
        (V4Step::Fails, V4Step::Fails) => {}
        (V4Step::Fails, V4Step::Ok(..)) => panic!(
            "crate succeeded where v4 fails or leaves u64: current={current} target={target} L={l} \
             remaining={remaining} exact_in={exact_in} fee={fee} ours={ours:?}"
        ),
    }
    false
}

fn log_u64() -> impl Strategy<Value = u64> {
    (1u32..=64).prop_flat_map(|bits| {
        let lo = 1u64 << (bits - 1);
        let hi = if bits == 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        lo..=hi
    })
}
fn log_u128(max_bits: u32) -> impl Strategy<Value = u128> {
    (1u32..=max_bits).prop_flat_map(|bits| {
        let lo = 1u128 << (bits - 1);
        let hi = if bits == 128 {
            u128::MAX
        } else {
            (1u128 << bits) - 1
        };
        lo..=hi
    })
}
fn fee_strategy() -> impl Strategy<Value = u32> {
    prop_oneof![
        Just(0u32),
        Just(100u32),
        Just(500u32),
        Just(2_500u32),
        Just(3_000u32),
        Just(10_000u32),
        Just(999_999u32),
        Just(MAX_FEE),
        0u32..=MAX_FEE,
    ]
}

/// The parity runs are reproducible: a fixed seed, so a failing case can be
/// re-run by anyone from the same source. `PROPTEST_RNG_SEED=<u64>` still wins when set, which is
/// how a different 20,000 are explored without editing this file; `PROPTEST_CASES` likewise.
const PARITY_SEED: u64 = 0x464c_4f4f_5220_7634; // "FLOOR v4"
fn parity_config() -> ProptestConfig {
    let base = ProptestConfig::default();
    let rng_seed = match base.rng_seed {
        RngSeed::Random => RngSeed::Fixed(PARITY_SEED),
        env_pinned => env_pinned,
    };
    eprintln!("v4_parity proptest rng_seed = {rng_seed}");
    ProptestConfig { cases: 20_000, failure_persistence: None, rng_seed, ..base }
}

proptest! {
    #![proptest_config(parity_config())]

    /// `get_delta_amount_{0,1}_unsigned` == v4 `getAmount{0,1}Delta` at both roundings, bit for bit,
    /// and the crate refuses exactly where v4's amount leaves u64.
    #[test]
    fn amount_deltas_match_v4(
        a in MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
        b in MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
        l in prop_oneof![log_u128(128), 0u128..=u128::MAX],
        up in any::<bool>(),
    ) {
        let v0 = v4_amount0(a, b, l, up).unwrap();
        match get_delta_amount_0_unsigned(a, b, l, up) {
            Ok(x) => prop_assert_eq!(Some(x), fits_u64(v0)),
            Err(MathError::MaxTokenOverflow) => prop_assert!(fits_u64(v0).is_none()),
            Err(e) => prop_assert!(false, "amount0 unexpected {:?}", e),
        }
        let v1 = v4_amount1(a, b, l, up);
        match get_delta_amount_1_unsigned(a, b, l, up) {
            Ok(x) => prop_assert_eq!(Some(x), fits_u64(v1)),
            Err(MathError::MaxTokenOverflow) => prop_assert!(fits_u64(v1).is_none()),
            Err(e) => prop_assert!(false, "amount1 unexpected {:?}", e),
        }
    }

    /// `get_next_sqrt_price_from_{input,output}` == v4 `getNextSqrtPriceFrom{Input,Output}` wherever v4
    /// returns a price; where v4 reverts, the crate returns `Err` or (one named case, below) 0.
    #[test]
    fn next_sqrt_price_matches_v4(
        p in MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
        l in prop_oneof![log_u128(128), 1u128..=u128::MAX],
        amount in log_u64(),
        zfo in any::<bool>(),
    ) {
        let v_in = v4_next_from_input(p, l, amount, zfo);
        let ours = get_next_sqrt_price_from_input(p, l, amount, zfo);
        match v_in {
            Some(x) => prop_assert_eq!(ours, Ok(x)),
            None => prop_assert!(ours.is_err(), "input: v4 fails, ours {:?}", ours),
        }
        let v_out = v4_next_from_output(p, l, amount, zfo);
        let ours = get_next_sqrt_price_from_output(p, l, amount, zfo);
        match v_out {
            Some(x) => prop_assert_eq!(ours, Ok(x)),
            // Named difference: removing token1 down to EXACTLY a zero price. v4 reverts
            // (`sqrtPX96 <= quotient`); upstream's `checked_sub` returns 0. `compute_swap` never reaches
            // it (a partial exact-out step ends strictly between current and a target ≥ MIN).
            None => prop_assert!(ours.is_err() || ours == Ok(0), "output: v4 fails, ours {:?}", ours),
        }
    }

    /// `compute_swap(.., is_fee_on_input = true)` == v4 `computeSwapStep`, exact-in, every fee in
    /// [0, 1e6] (1e6 and its neighbours weighted), ticks and raw prices, small to u128 liquidity.
    #[test]
    fn compute_swap_exact_in_matches_v4_step(
        current in prop_oneof![
            (MIN_TICK..MAX_TICK).prop_map(|t| get_sqrt_price_at_tick(t).unwrap()),
            MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
        ],
        target in prop_oneof![
            (MIN_TICK..=MAX_TICK).prop_map(|t| get_sqrt_price_at_tick(t).unwrap()),
            MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
        ],
        l in prop_oneof![Just(0u128), log_u128(100), log_u128(128)],
        remaining in log_u64(),
        fee in fee_strategy(),
    ) {
        prop_assert!(!assert_step_parity(current, target, l, remaining, true, fee), "exact-in never needs the named difference");
    }

    /// The same for exact output (fees below 1e6; at 1e6 both refuse, asserted separately).
    #[test]
    fn compute_swap_exact_out_matches_v4_step(
        current in (MIN_TICK..MAX_TICK).prop_map(|t| get_sqrt_price_at_tick(t).unwrap()),
        target in prop_oneof![
            (MIN_TICK..=MAX_TICK).prop_map(|t| get_sqrt_price_at_tick(t).unwrap()),
            MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64,
        ],
        l in prop_oneof![Just(0u128), log_u128(100), log_u128(128)],
        remaining in log_u64(),
        fee in (0u32..MAX_FEE),
    ) {
        assert_step_parity(current, target, l, remaining, false, fee);
    }
}

/// A fixed grid on top of the random runs: boundary ticks, the fee edge cases, both modes, and
/// current == target. A CURRENT price of exactly MAX_SQRT_PRICE_X64 is left out: no pool can be there
/// (v4 and this AMM both require a price limit below it), and the crate refuses it where v4's step would
/// run (`current_price_at_max_is_refused`).
#[test]
fn compute_swap_matches_v4_step_on_a_boundary_grid() {
    let ticks = [
        MIN_TICK,
        MIN_TICK + 1,
        -200_000,
        -6_932,
        -60,
        -1,
        0,
        1,
        60,
        6_932,
        200_000,
        MAX_TICK - 1,
        MAX_TICK,
    ];
    let ls = [0u128, 1, 1_000_000, 1u128 << 64, 1u128 << 96, u128::MAX];
    let amounts = [
        1u64,
        2,
        999_999,
        1_000_000,
        1_000_001,
        1u64 << 40,
        u64::MAX - 1,
        u64::MAX,
    ];
    let fees = [0u32, 1, 2_500, 500_000, 999_999, MAX_FEE];
    let mut cases = 0u64;
    let mut accepted = 0u64;
    let mut accepted_token1_out = 0u64;
    let mut min_remaining_token1_out = u64::MAX;
    let mut min_l_token1_out = u128::MAX;
    let mut max_price_token0_out = 0u128;
    let mut min_l_token0_out = u128::MAX;
    for &tc in ticks.iter().filter(|&&t| t != MAX_TICK) {
        for &tt in &ticks {
            let (c, t) = (
                get_sqrt_price_at_tick(tc).unwrap(),
                get_sqrt_price_at_tick(tt).unwrap(),
            );
            for &l in &ls {
                for &a in &amounts {
                    for &f in &fees {
                        assert!(!assert_step_parity(c, t, l, a, true, f));
                        if f < MAX_FEE && assert_step_parity(c, t, l, a, false, f) {
                            accepted += 1;
                            if c > t {
                                // token1 out: the re-derived floor is < a + L/2^64 + 1, so it can
                                // only leave u64 when the request is within L/2^64 of u64::MAX
                                assert!((a as u128) + (l >> 64) + 1 > u64::MAX as u128);
                                accepted_token1_out += 1;
                                min_remaining_token1_out = min_remaining_token1_out.min(a);
                                min_l_token1_out = min_l_token1_out.min(l);
                            } else {
                                // token0 out: one Q64 price unit is worth L·2^64/(c·next) > 2^64
                                // tokens only when L > c·next ≥ c², so only below price 1
                                assert!(c < (1u128 << 64));
                                max_price_token0_out = max_price_token0_out.max(c);
                                min_l_token0_out = min_l_token0_out.min(l);
                            }
                        }
                        cases += 1;
                    }
                }
            }
        }
    }
    assert_eq!(cases, 12 * 13 * 6 * 8 * 6);
    assert!(accepted > 0, "the grid exercises ND-EXACT-OUT-DUST");
    eprintln!(
        "boundary grid: {cases} fee/amount/L/tick cases; exact-out refusals accepted as ND-EXACT-OUT-DUST: \
         {accepted} (token1 out: {accepted_token1_out}, min remaining {min_remaining_token1_out}, min L \
         {min_l_token1_out}; token0 out: max current sqrt price {max_price_token0_out}, min L {min_l_token0_out})"
    );
}

/// A step that starts AT `MAX_SQRT_PRICE_X64` is refused (`SqrtPriceOutOfRange`), where v4's
/// `computeSwapStep` alone would compute one. Unreachable for a pool; recorded so it is not a surprise.
#[test]
fn current_price_at_max_is_refused() {
    let r = compute_swap(
        MAX_SQRT_PRICE_X64,
        MIN_SQRT_PRICE_X64,
        1,
        1,
        0,
        true,
        true,
        true,
    );
    assert_eq!(r.unwrap_err(), MathError::SqrtPriceOutOfRange);
    assert!(matches!(
        v4_compute_swap_step(MAX_SQRT_PRICE_X64, MIN_SQRT_PRICE_X64, 1, 1, true, 0),
        V4Step::Ok(..)
    ));
}

/// v4 refuses exact output at a 100 % fee; so does the crate, by name.
#[test]
fn exact_out_at_fee_1e6_is_refused_like_v4() {
    let c = get_sqrt_price_at_tick(0).unwrap();
    let t = get_sqrt_price_at_tick(-60).unwrap();
    assert_eq!(
        v4_compute_swap_step(c, t, 1u128 << 64, 1_000, false, MAX_FEE),
        V4Step::Fails
    );
    assert_eq!(
        compute_swap(c, t, 1u128 << 64, 1_000, MAX_FEE, false, true, true).unwrap_err(),
        MathError::InvalidFeeForExactOut
    );
}

// ---- TickMath.sol ---------------------------------------------------------------------------------

/// v4 `getSqrtPriceAtTick` before its final Q128.128 → Q128.96 conversion: `sqrt(1.0001^tick)·2^128`.
fn v4_sqrt_price_q128(tick: i32) -> U256 {
    const K: [&str; 20] = [
        "fffcb933bd6fad37aa2d162d1a594001",
        "fff97272373d413259a46990580e213a",
        "fff2e50f5f656932ef12357cf3c7fdcc",
        "ffe5caca7e10e4e61c3624eaa0941cd0",
        "ffcb9843d60f6159c9db58835c926644",
        "ff973b41fa98c081472e6896dfb254c0",
        "ff2ea16466c96a3843ec78b326b52861",
        "fe5dee046a99a2a811c461f1969c3053",
        "fcbe86c7900a88aedcffc83b479aa3a4",
        "f987a7253ac413176f2b074cf7815e54",
        "f3392b0822b70005940c7a398e4b70f3",
        "e7159475a2c29b7443b29c7fa6e889d9",
        "d097f3bdfd2022b8845ad8f792aa5825",
        "a9f746462d870fdf8a65dc1f90e061e5",
        "70d869a156d2a1b890bb3df62baf32f7",
        "31be135f97d08fd981231505542fcfa6",
        "9aa508b5b7a84e1c677de54f3e99bc9",
        "5d6af8dedb81196699c329225ee604",
        "2216e584f5fa1ea926041bedfe98",
        "48a170391f7dc42444e8fa2",
    ];
    let abs = tick.unsigned_abs();
    let mut price = if abs & 1 != 0 {
        U256::from_str_radix(K[0], 16).unwrap()
    } else {
        U256::one() << 128
    };
    for (i, k) in K.iter().enumerate().skip(1) {
        if abs & (1 << i) != 0 {
            price = (price * U256::from_str_radix(k, 16).unwrap()) >> 128;
        }
    }
    if tick > 0 {
        price = U256::MAX / price;
    }
    price
}

/// Tick math parity with v4, over EVERY tick in ±443,636.
///
/// Raydium's constants are 64-bit truncations of v4's 128-bit ones, so the two Q64 prices differ by a
/// precision error, asserted as `floor(log2(v4 / |ours − v4|)) ≥ 32` (the measured widest). What is
/// asserted per tick: `get_tick_at_sqrt_price` of v4's price, rounded to Q64 either way, decodes to the tick
/// whenever that value lies inside our bucket, and otherwise to an adjacent tick; the totals are pinned.
#[test]
fn tick_math_matches_v4_for_every_tick() {
    // below price 1 (tick < 0): the worst absolute error in Q64 units
    let mut worst_abs_below_1: u128 = 0;
    // at or above price 1: the smallest floor(log2(v4 / |diff|)), i.e. the widest relative error
    let mut worst_rel_shift_above_1 = u32::MAX;
    let mut worst_rel_shift_below_1 = u32::MAX;
    let mut decoded_off_by_one = 0u64;
    let mut ours_above_v4 = 0u64;
    let mut decoded_below = 0u64;
    let mut above_at_positive_tick = 0u64;
    let mut decoded_below_at_nonpositive_tick = 0u64;
    let mut worst_abs: u128 = 0;
    let mut bucket_misses = 0u64;
    for tick in MIN_TICK..=MAX_TICK {
        let ours = get_sqrt_price_at_tick(tick).unwrap();
        let q128 = v4_sqrt_price_q128(tick);
        let v4_floor = (q128 >> 64).low_u128();
        let v4_ceil = ((q128 + (U256::one() << 64) - U256::one()) >> 64).low_u128();
        let diff = if ours > v4_floor {
            ours - v4_floor
        } else {
            v4_floor - ours
        };
        worst_abs = worst_abs.max(diff);
        if v4_floor < (1u128 << 64) {
            worst_abs_below_1 = worst_abs_below_1.max(diff);
            if diff > 0 {
                worst_rel_shift_below_1 =
                    worst_rel_shift_below_1.min(127 - (v4_floor / diff).leading_zeros());
            }
        } else if diff > 0 {
            worst_rel_shift_above_1 =
                worst_rel_shift_above_1.min(127 - (v4_floor / diff).leading_zeros());
        }
        // the tick each v4 price decodes to, through OUR inverse
        for v in [v4_floor, v4_ceil] {
            if (MIN_SQRT_PRICE_X64..MAX_SQRT_PRICE_X64).contains(&v) {
                let decoded = get_tick_at_sqrt_price(v).unwrap();
                let next = if tick < MAX_TICK {
                    get_sqrt_price_at_tick(tick + 1).unwrap()
                } else {
                    u128::MAX
                };
                if v >= ours && v < next {
                    assert_eq!(
                        decoded, tick,
                        "v4 price {v} inside tick {tick}'s bucket decoded to {decoded}"
                    );
                } else {
                    bucket_misses += 1;
                    assert!(
                        (decoded - tick).abs() <= 1,
                        "tick {tick}: v4 price {v} decoded to {decoded}"
                    );
                    if decoded != tick {
                        decoded_off_by_one += 1;
                    }
                    if decoded == tick - 1 {
                        decoded_below += 1;
                        if tick <= 0 {
                            decoded_below_at_nonpositive_tick += 1;
                        }
                    }
                }
            }
        }
        if ours > v4_floor {
            ours_above_v4 += 1;
        }
        if ours > v4_floor && tick > 0 {
            above_at_positive_tick += 1;
        }
    }
    // Measured by this test (release and debug agree). Pinned so a change to either table fails.
    // Raydium's Q64 ratio loses precision as it shrinks toward 2^32 at |tick| = 443,636 (and `U128::MAX /
    // ratio` inverts that loss for tick > 0), so the widest relative error is 2^-32 on both sides — far
    // above a naive estimate of `ceil(v·2^-63) + 2` units, but 2^-32 is still ~2^18 below one tick
    // (1.0001^½ − 1 ≈ 2^-14.3), so a price never lands a whole tick away. But the two grids are offset:
    // 849,446 of v4's prices (floor and ceil, per tick) fall below our bucket for that tick and decode to
    // the tick BELOW through our inverse, on both sides of price 1, and none decodes above.
    assert!(worst_rel_shift_below_1 >= 32 && worst_rel_shift_above_1 >= 32);
    assert_eq!(ours_above_v4, 443_636);
    assert_eq!(above_at_positive_tick, 81_250);
    assert_eq!(bucket_misses, 849_446);
    assert_eq!(decoded_off_by_one, bucket_misses);
    assert_eq!(decoded_below, bucket_misses);
    assert_eq!(decoded_below_at_nonpositive_tick, 686_946);
    eprintln!(
        "tick parity: worst |ours - v4| below price 1 = {worst_abs_below_1} Q64 units (relative \
         2^-{worst_rel_shift_below_1}); at/above 1 the widest relative error is 2^-{worst_rel_shift_above_1}; \
         worst absolute = {worst_abs}; v4 Q64 prices outside our bucket: {bucket_misses}, decoded to tick-1: \
         {decoded_below} (at tick <= 0: {decoded_below_at_nonpositive_tick}); ticks where ours > v4 floor: \
         {ours_above_v4} (at tick > 0: {above_at_positive_tick})"
    );
}

/// Pinned values (regression anchors, cross-checked against the v4 oracle above in the same test).
#[test]
fn pinned_tick_vectors() {
    assert_eq!(get_sqrt_price_at_tick(0).unwrap(), 1u128 << 64);
    assert_eq!(
        get_sqrt_price_at_tick(MIN_TICK).unwrap(),
        MIN_SQRT_PRICE_X64
    );
    assert_eq!(
        get_sqrt_price_at_tick(MAX_TICK).unwrap(),
        MAX_SQRT_PRICE_X64
    );
    assert_eq!((v4_sqrt_price_q128(0) >> 64).low_u128(), 1u128 << 64);
    // v4's own constants, as a check on the oracle's transcription: getSqrtPriceAtTick(MIN_TICK = -887272)
    // is 4295128739 and (MAX_TICK) is 1461446703485210103287273052203988822378723970342, in Q96.
    let q96 = |t: i32| (v4_sqrt_price_q128(t) + (U256::one() << 32) - U256::one()) >> 32;
    assert_eq!(q96(-887_272), U256::from(4_295_128_739u64));
    assert_eq!(
        q96(887_272),
        U256::from_dec_str("1461446703485210103287273052203988822378723970342").unwrap()
    );
}
