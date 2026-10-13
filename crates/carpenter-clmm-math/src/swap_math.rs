// Derived from raydium-io/raydium-clmm @ ed7c84a5, programs/amm/src/libraries/swap_math.rs, Apache-2.0.
// Modified by FLOOR, 2026:
// - Anchor's `Result` / `ErrorCode` replaced by `MathResult` / `MathError`; the bare `require_gte!`
//   price-target checks return `MathError::PriceTargetOvershoot`; import paths;
//   `FEE_RATE_DENOMINATOR_VALUE` is defined here (upstream `states/config.rs:6`, same value).
// - `compute_swap` refuses `fee_rate > 1_000_000` (`FeeRateTooHigh`) and exact output at exactly
//   1_000_000 (`InvalidFeeForExactOut`, v4 `Pool.sol:310-316`) instead of underflowing or
//   dividing by zero.
// - The LP-fee rounding on the fee-on-input exact-input path follows v4-core `SwapMath.computeSwapStep`
//   (MIT, Uniswap/v4-core@46c6834): a partial step books `amount_in` = the fee-less amount and
//   `fee = amount_remaining - amount_in`; a step that reaches its target at a 100 % fee books
//   `fee = amount_in` (0). The two reference oracles in the tests carry the same split, as upstream
//   applied fcad161 to all oracles.
// - `compute_swap` refuses a `zero_for_one` that disagrees with `current > target`
//   (`DirectionMismatch`); v4 derives the flag, so the two cannot disagree there.
// - Tests added below the upstream ones: fee 1e6 on both exact modes, the fee-rate guard, the v4 split,
//   and the fcad161 exact-input dust loop re-expressed as a step loop over `compute_swap` (upstream's
//   copy of that test drives `swap_internal`, which this crate does not have).
// The fee-on-output paths, the fcad161 dust fix itself, and every upstream test are unchanged.

use crate::error::{MathError, MathResult};
use crate::full_math::MulDiv;
use crate::liquidity_math;
use crate::sqrt_price_math;

/// Fee rates are in hundredths of a bip: 1,000,000 = 100 %. Raydium `states/config.rs:6`;
/// equal to v4 `SwapMath.MAX_SWAP_FEE` and `LPFeeLibrary.MAX_LP_FEE`.
pub const FEE_RATE_DENOMINATOR_VALUE: u32 = 1_000_000;

/// Result of a swap computation
/// Contains the computed price, amounts, and fees after executing a swap calculation
#[derive(Default, Debug)]
pub struct SwapComputationResult {
    /// The price after swapping the amount in/out, not to exceed the price target
    pub sqrt_price_next_x64: u128,
    pub amount_in: u64,
    pub amount_out: u64,
    pub fee_amount: u64,
}

impl SwapComputationResult {
    pub fn new(sqrt_price_next_x64: u128) -> Self {
        Self {
            sqrt_price_next_x64,
            amount_in: 0,
            amount_out: 0,
            fee_amount: 0,
        }
    }
}

/// Computes the result of swapping some amount in, or amount out, given the parameters of the swap
pub fn compute_swap(
    sqrt_price_current_x64: u128,
    sqrt_price_target_x64: u128,
    liquidity: u128,
    amount_remaining: u64,
    fee_rate: u32,
    is_base_input: bool,
    zero_for_one: bool,
    is_fee_on_input: bool,
) -> MathResult<SwapComputationResult> {
    // FLOOR: upstream underflows `FEE_RATE_DENOMINATOR_VALUE - fee_rate` above 100 % and divides by
    // zero at exactly 100 % on exact output. v4 refuses the latter as `InvalidFeeForExactOut`
    // (`Pool.sol:310-316`); the AMM checks it first, and this is the math's own backstop.
    if fee_rate > FEE_RATE_DENOMINATOR_VALUE {
        return Err(MathError::FeeRateTooHigh);
    }
    if !is_base_input && fee_rate == FEE_RATE_DENOMINATOR_VALUE {
        return Err(MathError::InvalidFeeForExactOut);
    }
    // FLOOR: v4 `computeSwapStep` DERIVES the direction (`zeroForOne = current >= target`), so it can
    // never disagree with the prices. Upstream takes it as a parameter and never checks it: an inverted
    // flag with `target` on the wrong side takes the reached-target branch, passes the overshoot check,
    // and returns a step that moves the price UP while charging token0 in and paying token1 out.
    // `current == target` is a zero-width step and accepts either flag.
    if sqrt_price_current_x64 != sqrt_price_target_x64
        && zero_for_one != (sqrt_price_current_x64 > sqrt_price_target_x64)
    {
        return Err(MathError::DirectionMismatch);
    }

    let mut result = SwapComputationResult::default();

    // Gross amount that drives the price math: deduct fee for exact-input
    // fee-on-input; scale up for exact-output fee-on-output.
    let amount_for_price_calc = if is_base_input {
        if is_fee_on_input {
            amount_remaining
                .mul_div_floor(
                    (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                    u64::from(FEE_RATE_DENOMINATOR_VALUE),
                )
                .ok_or(MathError::CalculateOverflow)?
        } else {
            amount_remaining
        }
    } else {
        if is_fee_on_input {
            amount_remaining
        } else {
            amount_remaining
                .mul_div_ceil(
                    u64::from(FEE_RATE_DENOMINATOR_VALUE).into(),
                    (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                )
                .ok_or(MathError::CalculateOverflow)?
        }
    };

    // Both amounts at the target price. `MaxTokenOverflow` ⇒ target unreachable
    // at u64 precision → fall through to the not-reached branch.
    let max_reachable = match liquidity_math::get_delta_amounts_for_swap(
        sqrt_price_target_x64,
        sqrt_price_current_x64,
        liquidity,
        zero_for_one,
    ) {
        Ok((amount_in_at_target, amount_out_at_target)) => {
            let user_limit = if is_base_input {
                amount_in_at_target
            } else {
                amount_out_at_target
            };
            if amount_for_price_calc >= user_limit {
                Some((amount_in_at_target, amount_out_at_target))
            } else {
                None
            }
        }
        Err(MathError::MaxTokenOverflow) => None,
        Err(e) => return Err(e),
    };

    if let Some((amount_in_at_target, amount_out_at_target)) = max_reachable {
        result.sqrt_price_next_x64 = sqrt_price_target_x64;
        result.amount_in = amount_in_at_target;
        result.amount_out = amount_out_at_target;
    } else {
        // Solve for the actual sqrt_next reachable with `amount_for_price_calc`.
        // Since sqrt_next is closer to current than target, both amounts at
        // sqrt_next fit u64 even when the target-side amounts didn't.
        let sqrt_next = if is_base_input {
            sqrt_price_math::get_next_sqrt_price_from_input(
                sqrt_price_current_x64,
                liquidity,
                amount_for_price_calc,
                zero_for_one,
            )?
        } else {
            sqrt_price_math::get_next_sqrt_price_from_output(
                sqrt_price_current_x64,
                liquidity,
                amount_for_price_calc,
                zero_for_one,
            )?
        };
        result.sqrt_price_next_x64 = sqrt_next;
        let (amount_in, amount_out) = liquidity_math::get_delta_amounts_for_swap(
            sqrt_next,
            sqrt_price_current_x64,
            liquidity,
            zero_for_one,
        )?;
        result.amount_in = amount_in;
        result.amount_out = amount_out;
    }

    if zero_for_one {
        if !(result.sqrt_price_next_x64 >= sqrt_price_target_x64) {
            return Err(MathError::PriceTargetOvershoot);
        }
    } else {
        if !(sqrt_price_target_x64 >= result.sqrt_price_next_x64) {
            return Err(MathError::PriceTargetOvershoot);
        }
    }

    if is_base_input {
        if is_fee_on_input {
            if result.sqrt_price_next_x64 != sqrt_price_target_x64 {
                // FLOOR, the v4 LP-fee split (`SwapMath.sol` computeSwapStep, exact-in, "exhaust the
                // remaining amount"): the input is the fee-less amount the price was solved from, and
                // the fee is exactly what is left. Upstream instead books the input re-derived from the
                // rounded price (≤ that amount) and folds the difference into the fee, which credits
                // the LPs' fee growth with rounding dust v4 leaves as principal. `amount_in + fee ==
                // amount_remaining` holds either way, so the step still consumes everything (fcad161).
                result.amount_in = amount_for_price_calc;
                result.fee_amount = amount_remaining
                    .checked_sub(result.amount_in)
                    .ok_or(MathError::CalculateOverflow)?;
            } else if fee_rate == FEE_RATE_DENOMINATOR_VALUE {
                // FLOOR, v4 parity: at a 100 % fee the fee-less amount is 0, so reaching the target
                // means `amount_in == 0` (current == target, or zero liquidity), and the fee is that 0.
                // Upstream divides by `FEE_RATE_DENOMINATOR_VALUE - fee_rate == 0` here.
                result.fee_amount = result.amount_in;
            } else {
                result.fee_amount = result
                    .amount_in
                    .mul_div_ceil(
                        fee_rate.into(),
                        (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                    )
                    .ok_or(MathError::CalculateOverflow)?;
            }
        } else {
            // Fee from output: result.amount_out is gross output, fee is calculated from gross output
            // fee = gross_output * fee_rate / FEE_RATE_DENOMINATOR
            result.fee_amount = result
                .amount_out
                .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                .ok_or(MathError::CalculateOverflow)?;
            // Deduct fee from output: user receives net output
            result.amount_out = result
                .amount_out
                .checked_sub(result.fee_amount)
                .ok_or(MathError::CalculateOverflow)?;

            // Partial step: the price moved less than the exact input warrants (rounded toward the
            // pool — down for one_for_zero, up for zero_for_one), so amount_in recomputed from that
            // move can be below the available input, leaving an un-tradeable dust (< liquidity/Q64).
            // Fee-on-input folds it into the fee, fee-on-output cannot, so it would stall the loop.
            // Charge the full input (== amount_remaining here); the sub-unit excess goes to the pool.
            if result.sqrt_price_next_x64 != sqrt_price_target_x64 {
                result.amount_in = amount_remaining;
            }
        }
    } else {
        if is_fee_on_input {
            // Fee from input: amount_remaining is the desired gross output
            // Cap the gross output amount to the remaining amount
            result.amount_out = result.amount_out.min(amount_remaining);
            result.fee_amount = result
                .amount_in
                .mul_div_ceil(
                    fee_rate.into(),
                    (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                )
                .ok_or(MathError::CalculateOverflow)?;
        } else {
            result.fee_amount = result
                .amount_out
                .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                .ok_or(MathError::CalculateOverflow)?;

            // Calculate net output
            let net_output = result
                .amount_out
                .checked_sub(result.fee_amount)
                .ok_or(MathError::CalculateOverflow)?;

            // Cap net output to amount_remaining (user's desired net output)
            // If net output exceeds amount_remaining, adjust fee to cap it
            if net_output > amount_remaining {
                // Adjust fee so that net output = amount_remaining
                result.fee_amount = result
                    .amount_out
                    .checked_sub(amount_remaining)
                    .ok_or(MathError::CalculateOverflow)?;
                result.amount_out = amount_remaining;
            } else {
                // Deduct fee from output: user receives net output
                result.amount_out = net_output;
            }
        }
    }

    Ok(result)
}

#[cfg(test)]
mod swap_math_test {
    use crate::tick_math;

    use super::*;
    use proptest::prelude::*;

    /// Log-uniform `u64` sampler: every bit-width octave equally likely,
    /// unlike the default linear-uniform strategy.
    fn log_uniform_u64(min_bits: u32, max_bits: u32) -> impl Strategy<Value = u64> {
        (min_bits..=max_bits).prop_flat_map(|bits| {
            let lo = if bits == 0 { 0u64 } else { 1u64 << (bits - 1) };
            let hi = if bits >= 64 {
                u64::MAX
            } else {
                (1u64 << bits) - 1
            };
            lo..=hi
        })
    }

    /// Log-uniform `u128` sampler — same shape as [`log_uniform_u64`].
    fn log_uniform_u128(min_bits: u32, max_bits: u32) -> impl Strategy<Value = u128> {
        (min_bits..=max_bits).prop_flat_map(|bits| {
            let lo = if bits == 0 {
                0u128
            } else {
                1u128 << (bits - 1)
            };
            let hi = if bits >= 128 {
                u128::MAX
            } else {
                (1u128 << bits) - 1
            };
            lo..=hi
        })
    }

    /// Pre-refactor `compute_swap` (oracle for the equivalence proptest).
    /// Inherits the optimised math helpers — only the data-flow restructure
    /// is being verified here.
    fn compute_swap_reference(
        sqrt_price_current_x64: u128,
        sqrt_price_target_x64: u128,
        liquidity: u128,
        amount_remaining: u64,
        fee_rate: u32,
        is_base_input: bool,
        zero_for_one: bool,
        is_fee_on_input: bool,
    ) -> MathResult<SwapComputationResult> {
        let mut result = SwapComputationResult::default();
        if is_base_input {
            let amount_for_price_calc = if is_fee_on_input {
                // Fee from input: amount_remaining includes fee, so we need to deduct fee first
                amount_remaining
                    .mul_div_floor(
                        (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                        u64::from(FEE_RATE_DENOMINATOR_VALUE),
                    )
                    .ok_or(MathError::CalculateOverflow)?
            } else {
                amount_remaining
            };

            let amount_in = calculate_amount_in_range_reference(
                sqrt_price_current_x64,
                sqrt_price_target_x64,
                liquidity,
                zero_for_one,
                is_base_input,
            )?;
            if let Some(v) = amount_in {
                result.amount_in = v;
            }

            result.sqrt_price_next_x64 =
                if amount_in.is_some() && amount_for_price_calc >= result.amount_in {
                    sqrt_price_target_x64
                } else {
                    sqrt_price_math::get_next_sqrt_price_from_input(
                        sqrt_price_current_x64,
                        liquidity,
                        amount_for_price_calc,
                        zero_for_one,
                    )?
                };
        } else {
            // amount_remaining is the net output the user wants to receive (after fee deduction if fee is from output)
            let amount_for_price_calc = if is_fee_on_input {
                amount_remaining
            } else {
                // Fee from output: amount_remaining is net output, we need gross output for price calculation
                // gross_output = net_output / (1 - fee_rate / FEE_RATE_DENOMINATOR)
                amount_remaining
                    .mul_div_ceil(
                        u64::from(FEE_RATE_DENOMINATOR_VALUE).into(),
                        (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                    )
                    .ok_or(MathError::CalculateOverflow)?
            };

            let amount_out = calculate_amount_in_range_reference(
                sqrt_price_current_x64,
                sqrt_price_target_x64,
                liquidity,
                zero_for_one,
                is_base_input,
            )?;
            if let Some(v) = amount_out {
                result.amount_out = v;
            }
            result.sqrt_price_next_x64 =
                if amount_out.is_some() && amount_for_price_calc >= result.amount_out {
                    sqrt_price_target_x64
                } else {
                    sqrt_price_math::get_next_sqrt_price_from_output(
                        sqrt_price_current_x64,
                        liquidity,
                        amount_for_price_calc,
                        zero_for_one,
                    )?
                }
        }

        if zero_for_one {
            if !(result.sqrt_price_next_x64 >= sqrt_price_target_x64) {
                return Err(MathError::PriceTargetOvershoot);
            }
        } else {
            if !(sqrt_price_target_x64 >= result.sqrt_price_next_x64) {
                return Err(MathError::PriceTargetOvershoot);
            }
        }

        // whether we reached the max possible price for the given ticks
        let max = sqrt_price_target_x64 == result.sqrt_price_next_x64;
        // get the input / output amounts when target price is not reached
        if zero_for_one {
            // if max is reached for exact input case, entire amount_in is needed
            if !(max && is_base_input) {
                result.amount_in = liquidity_math::get_delta_amount_0_unsigned(
                    result.sqrt_price_next_x64,
                    sqrt_price_current_x64,
                    liquidity,
                    true,
                )?
            };
            // if max is reached for exact output case, entire amount_out is needed
            if !(max && !is_base_input) {
                result.amount_out = liquidity_math::get_delta_amount_1_unsigned(
                    result.sqrt_price_next_x64,
                    sqrt_price_current_x64,
                    liquidity,
                    false,
                )?;
            };
        } else {
            if !(max && is_base_input) {
                result.amount_in = liquidity_math::get_delta_amount_1_unsigned(
                    sqrt_price_current_x64,
                    result.sqrt_price_next_x64,
                    liquidity,
                    true,
                )?
            };
            if !(max && !is_base_input) {
                result.amount_out = liquidity_math::get_delta_amount_0_unsigned(
                    sqrt_price_current_x64,
                    result.sqrt_price_next_x64,
                    liquidity,
                    false,
                )?
            };
        }

        if is_base_input {
            if is_fee_on_input {
                if result.sqrt_price_next_x64 != sqrt_price_target_x64 {
                    // FLOOR: the v4 LP-fee split, applied to this oracle as in production.
                    result.amount_in = amount_remaining
                        .mul_div_floor(
                            (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                            u64::from(FEE_RATE_DENOMINATOR_VALUE),
                        )
                        .ok_or(MathError::CalculateOverflow)?;
                    result.fee_amount = amount_remaining
                        .checked_sub(result.amount_in)
                        .ok_or(MathError::CalculateOverflow)?;
                } else {
                    result.fee_amount = result
                        .amount_in
                        .mul_div_ceil(
                            fee_rate.into(),
                            (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                        )
                        .ok_or(MathError::CalculateOverflow)?;
                }
            } else {
                // Fee from output: result.amount_out is gross output, fee is calculated from gross output
                // fee = gross_output * fee_rate / FEE_RATE_DENOMINATOR
                result.fee_amount = result
                    .amount_out
                    .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                    .ok_or(MathError::CalculateOverflow)?;
                // Deduct fee from output: user receives net output
                result.amount_out = result
                    .amount_out
                    .checked_sub(result.fee_amount)
                    .ok_or(MathError::CalculateOverflow)?;

                // Dust fix: see production compute_swap.
                if !max {
                    result.amount_in = amount_remaining;
                }
            }
        } else {
            if is_fee_on_input {
                // Fee from input: amount_remaining is the desired gross output
                // Cap the gross output amount to the remaining amount
                result.amount_out = result.amount_out.min(amount_remaining);
                result.fee_amount = result
                    .amount_in
                    .mul_div_ceil(
                        fee_rate.into(),
                        (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                    )
                    .ok_or(MathError::CalculateOverflow)?;
            } else {
                result.fee_amount = result
                    .amount_out
                    .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                    .ok_or(MathError::CalculateOverflow)?;

                // Calculate net output
                let net_output = result
                    .amount_out
                    .checked_sub(result.fee_amount)
                    .ok_or(MathError::CalculateOverflow)?;

                // Cap net output to amount_remaining (user's desired net output)
                // If net output exceeds amount_remaining, adjust fee to cap it
                if net_output > amount_remaining {
                    // Adjust fee so that net output = amount_remaining
                    result.fee_amount = result
                        .amount_out
                        .checked_sub(amount_remaining)
                        .ok_or(MathError::CalculateOverflow)?;
                    result.amount_out = amount_remaining;
                } else {
                    // Deduct fee from output: user receives net output
                    result.amount_out = net_output;
                }
            }
        }

        Ok(result)
    }

    /// Pre-refactor `calculate_amount_in_range`, kept here so the reference
    /// `compute_swap` is stand-alone.
    fn calculate_amount_in_range_reference(
        sqrt_price_current_x64: u128,
        sqrt_price_target_x64: u128,
        liquidity: u128,
        zero_for_one: bool,
        is_base_input: bool,
    ) -> MathResult<Option<u64>> {
        let result = if is_base_input {
            if zero_for_one {
                liquidity_math::get_delta_amount_0_unsigned(
                    sqrt_price_target_x64,
                    sqrt_price_current_x64,
                    liquidity,
                    true,
                )
            } else {
                liquidity_math::get_delta_amount_1_unsigned(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    true,
                )
            }
        } else {
            if zero_for_one {
                liquidity_math::get_delta_amount_1_unsigned(
                    sqrt_price_target_x64,
                    sqrt_price_current_x64,
                    liquidity,
                    false,
                )
            } else {
                liquidity_math::get_delta_amount_0_unsigned(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    false,
                )
            }
        };

        match result {
            Ok(v) => Ok(Some(v)),
            Err(MathError::MaxTokenOverflow) => Ok(None),
            Err(_) => Err(MathError::CalculateOverflow),
        }
    }

    proptest! {
        // Sampled from ticks (not raw u128) so sqrt_price spans the full range
        // uniformly. Reject budget covers the exact_output + fee_on_output filters.
        #![proptest_config(ProptestConfig {
            cases: 65_536,
            max_global_rejects: 2_097_152,
            ..ProptestConfig::default()
        })]

        /// `compute_swap` must match `compute_swap_reference` byte-for-byte.
        /// `liquidity`/`amount_remaining` mix linear-uniform (high-magnitude
        /// U256 stress) with log-uniform (low-magnitude real-world swaps).
        #[test]
        fn compute_swap_matches_reference(
            tick_current in tick_math::MIN_TICK..tick_math::MAX_TICK,
            tick_target in tick_math::MIN_TICK..tick_math::MAX_TICK,
            liquidity in prop_oneof![
                (1u128..(1u128 << 80)).boxed(),
                log_uniform_u128(1, 80).boxed(),
            ],
            amount_remaining in prop_oneof![
                (1u64..u64::MAX).boxed(),
                log_uniform_u64(1, 64).boxed(),
            ],
            fee_rate in 1u32..(FEE_RATE_DENOMINATOR_VALUE - 1000),
            is_base_input in proptest::bool::ANY,
            is_fee_on_input in proptest::bool::ANY,
        ) {
            assert_equivalent_at_ticks(
                tick_current, tick_target, liquidity, amount_remaining, fee_rate,
                is_base_input, is_fee_on_input,
            );
        }

        #[test]
        fn compute_swap_step_test(
            sqrt_price_current_x64 in tick_math::MIN_SQRT_PRICE_X64..tick_math::MAX_SQRT_PRICE_X64,
            sqrt_price_target_x64 in tick_math::MIN_SQRT_PRICE_X64..tick_math::MAX_SQRT_PRICE_X64,
            liquidity in 1..u32::MAX as u128,
            amount_remaining in 1..u64::MAX,
            fee_rate in 1..(FEE_RATE_DENOMINATOR_VALUE - 1000), // Avoid fee_rate too close to FEE_RATE_DENOMINATOR_VALUE
            is_base_input in proptest::bool::ANY,
            is_fee_on_input in proptest::bool::ANY,
        ) {
            prop_assume!(sqrt_price_current_x64 != sqrt_price_target_x64);

            // Avoid extreme price differences that could cause overflow
            let price_diff = if sqrt_price_current_x64 > sqrt_price_target_x64 {
                sqrt_price_current_x64 - sqrt_price_target_x64
            } else {
                sqrt_price_target_x64 - sqrt_price_current_x64
            };
            prop_assume!(price_diff < u128::MAX / 1000);

            // Exact-output + fee-on-output: keep amount_remaining * DENOM within u64.
            if !is_base_input && !is_fee_on_input {
                prop_assume!(amount_remaining <= u64::MAX / u64::from(FEE_RATE_DENOMINATOR_VALUE));
            }

            // Exact-output + fee-on-input: bound amount and fee_rate so the
            // input-side fee math cannot overflow.
            if !is_base_input && is_fee_on_input {
                prop_assume!(amount_remaining <= 1_000_000_000u64);
                prop_assume!(fee_rate <= FEE_RATE_DENOMINATOR_VALUE - 100);
            }

            let zero_for_one = sqrt_price_current_x64 > sqrt_price_target_x64;
            let swap_step = compute_swap(
                sqrt_price_current_x64,
                sqrt_price_target_x64,
                liquidity,
                amount_remaining,
                fee_rate,
                is_base_input,
                zero_for_one,
                is_fee_on_input,
            ).unwrap();

            let amount_in = swap_step.amount_in;
            let amount_out = swap_step.amount_out;
            let sqrt_price_next_x64 = swap_step.sqrt_price_next_x64;
            let fee_amount = swap_step.fee_amount;

            // amount_used represents the amount_remaining that was actually consumed
            let amount_used = if is_base_input {
                if is_fee_on_input {
                    // amount_remaining is gross input
                    amount_in + fee_amount
                } else {
                    // amount_remaining is net input
                    amount_in
                }
            } else {
                if is_fee_on_input {
                    // amount_remaining is gross output
                    amount_out
                } else {
                    // amount_remaining is net output; gross = amount_out + fee
                    amount_out + fee_amount
                }
            };

            if sqrt_price_next_x64 != sqrt_price_target_x64 {
                assert!(amount_used == amount_remaining,
                    "amount_used ({}) should equal amount_remaining ({}) when target not reached",
                    amount_used, amount_remaining);
            } else {
                assert!(amount_used <= amount_remaining,
                    "amount_used ({}) should be <= amount_remaining ({}) when target reached",
                    amount_used, amount_remaining);
            }
            let price_lower = sqrt_price_current_x64.min(sqrt_price_target_x64);
            let price_upper = sqrt_price_current_x64.max(sqrt_price_target_x64);
            assert!(sqrt_price_next_x64 >= price_lower);
            assert!(sqrt_price_next_x64 <= price_upper);
        }
    }

    /// `compute_swap` from `f16c59a` with the two-step division helpers,
    /// kept as an oracle independent of the optimised math library.
    mod pre_series {
        use super::{SwapComputationResult, FEE_RATE_DENOMINATOR_VALUE};
        use crate::big_num::U256;
        use crate::error::{MathError, MathResult};
        use crate::fixed_point_64;
        use crate::full_math::MulDiv;
        use crate::sqrt_price_math;
        use crate::unsafe_math::UnsafeMathTrait;

        /// Two-step `get_delta_amount_0_unsigned` (separate divisions by
        /// `sqrt_b` then `sqrt_a`), independent path for the merged form.
        pub fn get_delta_amount_0_unsigned(
            mut sqrt_ratio_a_x64: u128,
            mut sqrt_ratio_b_x64: u128,
            liquidity: u128,
            round_up: bool,
        ) -> MathResult<u64> {
            if sqrt_ratio_a_x64 > sqrt_ratio_b_x64 {
                std::mem::swap(&mut sqrt_ratio_a_x64, &mut sqrt_ratio_b_x64);
            };
            if !(sqrt_ratio_a_x64 > 0) {
                return Err(MathError::ZeroSqrtPrice);
            }
            let numerator_1 = U256::from(liquidity) << fixed_point_64::RESOLUTION;
            let numerator_2 = U256::from(sqrt_ratio_b_x64 - sqrt_ratio_a_x64);
            let result = if round_up {
                U256::div_rounding_up(
                    numerator_1
                        .mul_div_ceil(numerator_2, U256::from(sqrt_ratio_b_x64))
                        .ok_or(MathError::CalculateOverflow)?,
                    U256::from(sqrt_ratio_a_x64),
                )
            } else {
                numerator_1
                    .mul_div_floor(numerator_2, U256::from(sqrt_ratio_b_x64))
                    .ok_or(MathError::CalculateOverflow)?
                    / U256::from(sqrt_ratio_a_x64)
            };
            if result > U256::from(u64::MAX) {
                return Err(MathError::MaxTokenOverflow);
            }
            Ok(result.as_u64())
        }

        // amount_1 is unchanged between f16c59a and HEAD; reuse production helper.
        use crate::liquidity_math::get_delta_amount_1_unsigned;

        fn calculate_amount_in_range(
            sqrt_price_current_x64: u128,
            sqrt_price_target_x64: u128,
            liquidity: u128,
            zero_for_one: bool,
            is_base_input: bool,
        ) -> MathResult<Option<u64>> {
            let result = if is_base_input {
                if zero_for_one {
                    get_delta_amount_0_unsigned(
                        sqrt_price_target_x64,
                        sqrt_price_current_x64,
                        liquidity,
                        true,
                    )
                } else {
                    get_delta_amount_1_unsigned(
                        sqrt_price_current_x64,
                        sqrt_price_target_x64,
                        liquidity,
                        true,
                    )
                }
            } else if zero_for_one {
                get_delta_amount_1_unsigned(
                    sqrt_price_target_x64,
                    sqrt_price_current_x64,
                    liquidity,
                    false,
                )
            } else {
                get_delta_amount_0_unsigned(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    false,
                )
            };
            match result {
                Ok(v) => Ok(Some(v)),
                Err(MathError::MaxTokenOverflow) => Ok(None),
                Err(_) => Err(MathError::CalculateOverflow),
            }
        }

        /// `compute_swap` from f16c59a, using only the pre-series helpers above.
        pub fn compute_swap_pre_series_reference(
            sqrt_price_current_x64: u128,
            sqrt_price_target_x64: u128,
            liquidity: u128,
            amount_remaining: u64,
            fee_rate: u32,
            is_base_input: bool,
            zero_for_one: bool,
            is_fee_on_input: bool,
        ) -> MathResult<SwapComputationResult> {
            let mut result = SwapComputationResult::default();
            if is_base_input {
                let amount_for_price_calc = if is_fee_on_input {
                    amount_remaining
                        .mul_div_floor(
                            (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                            u64::from(FEE_RATE_DENOMINATOR_VALUE),
                        )
                        .ok_or(MathError::CalculateOverflow)?
                } else {
                    amount_remaining
                };
                let amount_in = calculate_amount_in_range(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    zero_for_one,
                    is_base_input,
                )?;
                if let Some(v) = amount_in {
                    result.amount_in = v;
                }
                result.sqrt_price_next_x64 =
                    if amount_in.is_some() && amount_for_price_calc >= result.amount_in {
                        sqrt_price_target_x64
                    } else {
                        sqrt_price_math::get_next_sqrt_price_from_input(
                            sqrt_price_current_x64,
                            liquidity,
                            amount_for_price_calc,
                            zero_for_one,
                        )?
                    };
            } else {
                let amount_for_price_calc = if is_fee_on_input {
                    amount_remaining
                } else {
                    amount_remaining
                        .mul_div_ceil(
                            u64::from(FEE_RATE_DENOMINATOR_VALUE).into(),
                            (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                        )
                        .ok_or(MathError::CalculateOverflow)?
                };
                let amount_out = calculate_amount_in_range(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    zero_for_one,
                    is_base_input,
                )?;
                if let Some(v) = amount_out {
                    result.amount_out = v;
                }
                result.sqrt_price_next_x64 =
                    if amount_out.is_some() && amount_for_price_calc >= result.amount_out {
                        sqrt_price_target_x64
                    } else {
                        sqrt_price_math::get_next_sqrt_price_from_output(
                            sqrt_price_current_x64,
                            liquidity,
                            amount_for_price_calc,
                            zero_for_one,
                        )?
                    }
            }
            if zero_for_one {
                if !(result.sqrt_price_next_x64 >= sqrt_price_target_x64) {
                    return Err(MathError::PriceTargetOvershoot);
                }
            } else {
                if !(sqrt_price_target_x64 >= result.sqrt_price_next_x64) {
                    return Err(MathError::PriceTargetOvershoot);
                }
            }
            let max = sqrt_price_target_x64 == result.sqrt_price_next_x64;
            if zero_for_one {
                if !(max && is_base_input) {
                    result.amount_in = get_delta_amount_0_unsigned(
                        result.sqrt_price_next_x64,
                        sqrt_price_current_x64,
                        liquidity,
                        true,
                    )?
                };
                if !(max && !is_base_input) {
                    result.amount_out = get_delta_amount_1_unsigned(
                        result.sqrt_price_next_x64,
                        sqrt_price_current_x64,
                        liquidity,
                        false,
                    )?;
                };
            } else {
                if !(max && is_base_input) {
                    result.amount_in = get_delta_amount_1_unsigned(
                        sqrt_price_current_x64,
                        result.sqrt_price_next_x64,
                        liquidity,
                        true,
                    )?
                };
                if !(max && !is_base_input) {
                    result.amount_out = get_delta_amount_0_unsigned(
                        sqrt_price_current_x64,
                        result.sqrt_price_next_x64,
                        liquidity,
                        false,
                    )?
                };
            }
            if is_base_input {
                if is_fee_on_input {
                    if result.sqrt_price_next_x64 != sqrt_price_target_x64 {
                        // FLOOR: the v4 LP-fee split, applied to this oracle as in production.
                        result.amount_in = amount_remaining
                            .mul_div_floor(
                                (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                                u64::from(FEE_RATE_DENOMINATOR_VALUE),
                            )
                            .ok_or(MathError::CalculateOverflow)?;
                        result.fee_amount = amount_remaining
                            .checked_sub(result.amount_in)
                            .ok_or(MathError::CalculateOverflow)?;
                    } else {
                        result.fee_amount = result
                            .amount_in
                            .mul_div_ceil(
                                fee_rate.into(),
                                (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                            )
                            .ok_or(MathError::CalculateOverflow)?;
                    }
                } else {
                    result.fee_amount = result
                        .amount_out
                        .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                        .ok_or(MathError::CalculateOverflow)?;
                    result.amount_out = result
                        .amount_out
                        .checked_sub(result.fee_amount)
                        .ok_or(MathError::CalculateOverflow)?;
                    // Dust fix (fcad161), intentionally applied to all oracles.
                    if !max {
                        result.amount_in = amount_remaining;
                    }
                }
            } else if is_fee_on_input {
                result.amount_out = result.amount_out.min(amount_remaining);
                result.fee_amount = result
                    .amount_in
                    .mul_div_ceil(
                        fee_rate.into(),
                        (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                    )
                    .ok_or(MathError::CalculateOverflow)?;
            } else {
                result.fee_amount = result
                    .amount_out
                    .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                    .ok_or(MathError::CalculateOverflow)?;
                let net_output = result
                    .amount_out
                    .checked_sub(result.fee_amount)
                    .ok_or(MathError::CalculateOverflow)?;
                if net_output > amount_remaining {
                    result.fee_amount = result
                        .amount_out
                        .checked_sub(amount_remaining)
                        .ok_or(MathError::CalculateOverflow)?;
                    result.amount_out = amount_remaining;
                } else {
                    result.amount_out = net_output;
                }
            }
            Ok(result)
        }
    }

    /// Compares production `compute_swap` against both oracles over the full
    /// input domain. Also pins one absolute property the shared-source fee code
    /// makes equivalence blind to: pure-fee modes
    /// (`is_base_input != is_fee_on_input`) must emit zero fee at zero rate;
    /// the other two modes repurpose `fee_amount` (cap excess / partial dust).
    fn assert_equivalent_at_ticks(
        tick_current: i32,
        tick_target: i32,
        liquidity: u128,
        amount_remaining: u64,
        fee_rate: u32,
        is_base_input: bool,
        is_fee_on_input: bool,
    ) {
        if tick_current == tick_target {
            return;
        }

        let sqrt_price_current_x64 = tick_math::get_sqrt_price_at_tick(tick_current).unwrap();
        let sqrt_price_target_x64 = tick_math::get_sqrt_price_at_tick(tick_target).unwrap();
        let zero_for_one = sqrt_price_current_x64 > sqrt_price_target_x64;

        let new_result = compute_swap(
            sqrt_price_current_x64,
            sqrt_price_target_x64,
            liquidity,
            amount_remaining,
            fee_rate,
            is_base_input,
            zero_for_one,
            is_fee_on_input,
        );

        if fee_rate == 0 && (is_base_input != is_fee_on_input) {
            if let Ok(r) = &new_result {
                assert_eq!(
                    r.fee_amount, 0,
                    "fee_amount must be 0 when fee_rate==0 in a pure-fee mode at ticks=({tick_current},{tick_target}), L={liquidity}, amount={amount_remaining}, is_base_input={is_base_input}, is_fee_on_input={is_fee_on_input}",
                );
            }
        }

        let ref_result = compute_swap_reference(
            sqrt_price_current_x64,
            sqrt_price_target_x64,
            liquidity,
            amount_remaining,
            fee_rate,
            is_base_input,
            zero_for_one,
            is_fee_on_input,
        );

        let pre_series_result = pre_series::compute_swap_pre_series_reference(
            sqrt_price_current_x64,
            sqrt_price_target_x64,
            liquidity,
            amount_remaining,
            fee_rate,
            is_base_input,
            zero_for_one,
            is_fee_on_input,
        );
        match (&new_result, &pre_series_result) {
            (Ok(new), Ok(p)) => assert_eq!(
                (
                    new.sqrt_price_next_x64,
                    new.amount_in,
                    new.amount_out,
                    new.fee_amount
                ),
                (
                    p.sqrt_price_next_x64,
                    p.amount_in,
                    p.amount_out,
                    p.fee_amount
                ),
                "pre-series oracle mismatch at ticks=({},{}), L={}, amount={}, fee_rate={}, is_base_input={}, is_fee_on_input={}",
                tick_current,
                tick_target,
                liquidity,
                amount_remaining,
                fee_rate,
                is_base_input,
                is_fee_on_input
            ),
            (Err(_), Err(_)) => {}
            (Ok(_), Err(e)) => panic!(
                "new succeeded where pre-series errored at ticks=({},{}), L={}, amount={}: pre_err={:?}",
                tick_current, tick_target, liquidity, amount_remaining, e
            ),
            (Err(e), Ok(_)) => panic!(
                "new errored where pre-series succeeded at ticks=({},{}), L={}, amount={}: new_err={:?}",
                tick_current, tick_target, liquidity, amount_remaining, e
            ),
        }

        match (new_result, ref_result) {
            (Ok(new), Ok(reference)) => {
                assert_eq!(
                    (
                        new.sqrt_price_next_x64,
                        new.amount_in,
                        new.amount_out,
                        new.fee_amount
                    ),
                    (
                        reference.sqrt_price_next_x64,
                        reference.amount_in,
                        reference.amount_out,
                        reference.fee_amount
                    ),
                    "SwapComputationResult mismatch at ticks=({},{}), L={}, amount={}, fee_rate={}, is_base_input={}, is_fee_on_input={}",
                    tick_current,
                    tick_target,
                    liquidity,
                    amount_remaining,
                    fee_rate,
                    is_base_input,
                    is_fee_on_input
                );
            }
            (Err(_), Err(_)) => {}
            (Ok(new), Err(e)) => panic!(
                "new succeeded where ref errored at ticks=({},{}), L={}, amount={}: new={:?}, ref_err={:?}",
                tick_current, tick_target, liquidity, amount_remaining, new, e
            ),
            (Err(e), Ok(reference)) => panic!(
                "new errored where ref succeeded at ticks=({},{}), L={}, amount={}: new_err={:?}, ref={:?}",
                tick_current, tick_target, liquidity, amount_remaining, e, reference
            ),
        }
    }

    /// Boundary-tick / liquidity / amount / fee combinations run on every
    /// `cargo test`. ~1 s.
    #[test]
    fn compute_swap_matches_reference_boundary_samples() {
        let boundary_ticks: [i32; 19] = [
            tick_math::MIN_TICK,
            tick_math::MIN_TICK + 1,
            -400_000,
            -100_000,
            -10_000,
            -1_000,
            -100,
            -10,
            -1,
            0,
            1,
            10,
            100,
            1_000,
            10_000,
            100_000,
            400_000,
            tick_math::MAX_TICK - 1,
            tick_math::MAX_TICK,
        ];
        // L=0 is production-reachable: the swap loop only guards on is_price_change.
        let liquidities: [u128; 6] = [0, 1, 1_000_000, 1u128 << 32, 1u128 << 60, (1u128 << 80) - 1];
        // Pins both u64 extremes.
        let amounts: [u64; 8] = [
            1,
            2,
            1_000,
            1_000_000_000,
            1_000_000_000_000,
            u64::MAX / 2,
            u64::MAX - 1,
            u64::MAX,
        ];
        // `fee_rate = 0` covers zero-fee pools; the proptests start at 1.
        let fee_rates: [u32; 5] = [0, 1, 2_500, 30_000, FEE_RATE_DENOMINATOR_VALUE - 2_000];
        let flags: [(bool, bool); 4] = [(true, true), (true, false), (false, true), (false, false)];

        for &tc in &boundary_ticks {
            for &tt in &boundary_ticks {
                for &l in &liquidities {
                    for &a in &amounts {
                        for &fr in &fee_rates {
                            for &(is_base, is_fee_in) in &flags {
                                assert_equivalent_at_ticks(tc, tt, l, a, fr, is_base, is_fee_in);
                            }
                        }
                    }
                }
            }
        }
    }

    /// The four MaxTokenOverflow-at-target corners (overflowing side ×
    /// exact-in/out × direction): HEAD and baseline must both fail. Error
    /// codes may differ — either way the transaction reverts.
    #[test]
    fn outcome_equivalent_under_max_token_overflow_scenarios() {
        // (description, tick_a, tick_b, L, amount, fee_rate, is_base_input, is_fee_on_input)
        let cases: &[(&str, i32, i32, u128, u64, u32, bool, bool)] = &[
            (
                "amount_0 overflow at MIN_TICK, exact-output zfo (un-spec amount_in overflows)",
                -443_635,
                -443_636,
                1u128 << 60,
                1_000_000_000,
                1,
                false,
                true,
            ),
            (
                "amount_1 overflow at MAX_TICK, exact-output !zfo (un-spec amount_in overflows)",
                443_635,
                443_636,
                1u128 << 60,
                1_000_000_000,
                1,
                false,
                true,
            ),
            (
                "amount_1 overflow at high prices, exact-input zfo (un-spec amount_out overflows)",
                100_001,
                100_000,
                1u128 << 80,
                u64::MAX,
                1,
                true,
                true,
            ),
            (
                "amount_0 overflow at low prices, exact-input !zfo (un-spec amount_out overflows)",
                -100_001,
                -100_000,
                1u128 << 80,
                u64::MAX,
                1,
                true,
                true,
            ),
        ];

        for &(
            desc,
            tick_current,
            tick_target,
            liquidity,
            amount,
            fee,
            is_base_input,
            is_fee_on_input,
        ) in cases
        {
            let sqrt_current = tick_math::get_sqrt_price_at_tick(tick_current).unwrap();
            let sqrt_target = tick_math::get_sqrt_price_at_tick(tick_target).unwrap();
            let zfo = sqrt_current > sqrt_target;

            let head = compute_swap(
                sqrt_current,
                sqrt_target,
                liquidity,
                amount,
                fee,
                is_base_input,
                zfo,
                is_fee_on_input,
            );
            let baseline = pre_series::compute_swap_pre_series_reference(
                sqrt_current,
                sqrt_target,
                liquidity,
                amount,
                fee,
                is_base_input,
                zfo,
                is_fee_on_input,
            );

            assert!(head.is_err(), "[{}] HEAD must fail; got {:?}", desc, head);
            assert!(
                baseline.is_err(),
                "[{}] baseline must fail; got {:?}",
                desc,
                baseline
            );
        }
    }

    /// Success path of the `MaxTokenOverflow → None → !max` route: amounts
    /// overflow u64 at target but the swap stops short of it, so it must
    /// succeed byte-identically in all implementations. Each case is
    /// precondition-checked to actually overflow at target.
    #[test]
    fn outcome_equivalent_when_target_overflows_but_swap_succeeds() {
        // (description, tick_a, tick_b, L, amount, fee_rate, is_base_input, is_fee_on_input)
        let cases: &[(&str, i32, i32, u128, u64, u32, bool, bool)] = &[
            (
                "exact-output zfo low prices: un-spec amount_in (=amount_0) overflows at target",
                -200_000,
                -443_636,
                1u128 << 60,
                1,
                1,
                false,
                true,
            ),
            (
                "exact-output !zfo high prices: un-spec amount_in (=amount_1) overflows at target",
                200_000,
                443_636,
                1u128 << 60,
                1,
                1,
                false,
                true,
            ),
            (
                "exact-input !zfo low prices: un-spec amount_out (=amount_0) overflows at target",
                -443_636,
                -200_000,
                1u128 << 60,
                1,
                1,
                true,
                true,
            ),
            (
                "exact-input zfo high prices: un-spec amount_out (=amount_1) overflows at target",
                443_635,
                200_000,
                1u128 << 60,
                1,
                1,
                true,
                true,
            ),
        ];

        for &(desc, tc, tt, liquidity, amount, fee, is_base_input, is_fee_on_input) in cases {
            let sqrt_current = tick_math::get_sqrt_price_at_tick(tc).unwrap();
            let sqrt_target = tick_math::get_sqrt_price_at_tick(tt).unwrap();
            let zfo = sqrt_current > sqrt_target;

            // Precondition: the target-side delta must overflow u64.
            let target_probe = liquidity_math::get_delta_amounts_for_swap(
                sqrt_target,
                sqrt_current,
                liquidity,
                zfo,
            );
            let probe_dbg = format!("{:?}", target_probe);
            assert!(
                probe_dbg.contains("MaxTokenOverflow"),
                "[{}] precondition: target-side amount should overflow u64 to exercise the \
                 MaxTokenOverflow path; got {}",
                desc,
                probe_dbg
            );

            assert_equivalent_at_ticks(
                tc,
                tt,
                liquidity,
                amount,
                fee,
                is_base_input,
                is_fee_on_input,
            );
        }
    }

    /// Partial-step fee contract for the two exact-input modes, asserted
    /// independently of the (shared-source) oracles: fee-on-input conserves
    /// the gross amount; fee-on-output charges the full input (dust fix) and
    /// takes the fee from the gross output.
    #[test]
    fn partial_step_exact_input_fee_invariants() {
        const FEE_RATE: u32 = 2_500;
        // (tick_current, tick_target, liquidity) — targets far enough that every
        // amount below leaves the step partial.
        let scenarios: &[(i32, i32, u128)] = &[
            (1_000, tick_math::MIN_TICK, 1u128 << 40),
            (1_000, tick_math::MIN_TICK, 1u128 << 60),
            (-1_000, tick_math::MAX_TICK, 1u128 << 40),
            (-1_000, tick_math::MAX_TICK, 1u128 << 60),
            (0, -100_000, 1_000_000_000_000),
            (0, 100_000, 1_000_000_000_000),
        ];
        let amounts: [u64; 3] = [1_000_000, 1_000_000_000, 1_000_000_000_000];

        for &(tc, tt, liquidity) in scenarios {
            let sqrt_current = tick_math::get_sqrt_price_at_tick(tc).unwrap();
            let sqrt_target = tick_math::get_sqrt_price_at_tick(tt).unwrap();
            let zero_for_one = sqrt_current > sqrt_target;
            let price_lo = sqrt_current.min(sqrt_target);
            let price_hi = sqrt_current.max(sqrt_target);

            for &amount in &amounts {
                // ---- fee-on-input (exact-input) ----
                let r = compute_swap(
                    sqrt_current,
                    sqrt_target,
                    liquidity,
                    amount,
                    FEE_RATE,
                    true,
                    zero_for_one,
                    true,
                )
                .unwrap();
                assert_ne!(
                    r.sqrt_price_next_x64, sqrt_target,
                    "fee-on-input precondition: expected partial step at ticks=({tc},{tt}), L={liquidity}, amount={amount}",
                );
                assert!(
                    r.sqrt_price_next_x64 > price_lo && r.sqrt_price_next_x64 < price_hi,
                    "sqrt_next must move strictly inside (current,target)",
                );
                assert_eq!(
                    r.amount_in.checked_add(r.fee_amount).unwrap(),
                    amount,
                    "fee-on-input partial step: amount_in + fee must equal gross amount_remaining",
                );
                assert!(
                    r.amount_in < amount,
                    "fee-on-input: a positive fee must be carved out"
                );
                assert!(
                    r.amount_out > 0,
                    "fee-on-input: partial step must produce output"
                );

                // ---- fee-on-output (exact-input): the dust fix ----
                let r = compute_swap(
                    sqrt_current,
                    sqrt_target,
                    liquidity,
                    amount,
                    FEE_RATE,
                    true,
                    zero_for_one,
                    false,
                )
                .unwrap();
                assert_ne!(
                    r.sqrt_price_next_x64, sqrt_target,
                    "fee-on-output precondition: expected partial step at ticks=({tc},{tt}), L={liquidity}, amount={amount}",
                );
                assert_eq!(
                    r.amount_in, amount,
                    "dust fix: a partial step must charge the full input so the loop cannot stall",
                );
                let gross_output = r.amount_out.checked_add(r.fee_amount).unwrap();
                assert!(
                    r.amount_out > 0,
                    "fee-on-output: partial step must produce net output"
                );
                assert!(
                    r.fee_amount > 0,
                    "fee-on-output: a positive fee must be taken"
                );
                assert_eq!(
                    r.fee_amount,
                    gross_output
                        .mul_div_ceil(FEE_RATE.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                        .unwrap(),
                    "fee-on-output: fee must be taken from the gross output",
                );
            }
        }
    }

    /// Exact-output swap-loop convergence: exact-input has a dust fix that forces
    /// progress on a partial step, exact-output does not, so verify the loop still
    /// terminates. Models swap.rs: each step subtracts amount_out from remaining
    /// and advances the price, exiting on remaining==0 or price==target. A step
    /// that neither moves the price nor consumes output would stall forever.
    #[test]
    fn exact_output_swap_loop_converges() {
        const ITER_LIMIT: usize = 100_000;
        let tick_pairs: [(i32, i32); 6] = [
            (0, -100),
            (0, 100),
            (0, -100_000),
            (0, 100_000),
            (100_000, tick_math::MIN_TICK),
            (-100_000, tick_math::MAX_TICK),
        ];
        // Large L makes the price move tiny per output unit — the regime where a
        // floor-to-zero amount_out is most likely.
        let liquidities: [u128; 4] = [1_000_000, 1u128 << 40, 1u128 << 70, 1u128 << 100];
        let amounts: [u64; 4] = [1, 2, 1_000, 1_000_000];
        let fee_rate = 2_500u32;

        for &(tc, tt) in &tick_pairs {
            let sqrt_current = tick_math::get_sqrt_price_at_tick(tc).unwrap();
            let sqrt_target = tick_math::get_sqrt_price_at_tick(tt).unwrap();
            let zero_for_one = sqrt_current > sqrt_target;
            for &l in &liquidities {
                for &amount in &amounts {
                    for &is_fee_on_input in &[true, false] {
                        let mut price = sqrt_current;
                        let mut remaining = amount;
                        let mut converged = false;
                        for iter in 0..ITER_LIMIT {
                            if remaining == 0 || price == sqrt_target {
                                converged = true;
                                break;
                            }
                            let r = match compute_swap(
                                price,
                                sqrt_target,
                                l,
                                remaining,
                                fee_rate,
                                false, // exact-output
                                zero_for_one,
                                is_fee_on_input,
                            ) {
                                Ok(r) => r,
                                // A revert is a clean exit, not a stall.
                                Err(_) => {
                                    converged = true;
                                    break;
                                }
                            };
                            let moved = r.sqrt_price_next_x64 != price;
                            assert!(
                                r.amount_out > 0 || moved,
                                "STALL at iter {iter}: no progress. ticks=({tc},{tt}), L={l}, remaining={remaining}, is_fee_on_input={is_fee_on_input}",
                            );
                            remaining = remaining.checked_sub(r.amount_out).expect(
                                "exact-output amount_out exceeded remaining (over-delivery)",
                            );
                            price = r.sqrt_price_next_x64;
                        }
                        assert!(
                            converged,
                            "did not converge within {ITER_LIMIT} iters: ticks=({tc},{tt}), L={l}, amount={amount}, is_fee_on_input={is_fee_on_input}, remaining={remaining}",
                        );
                    }
                }
            }
        }
    }

    /// Full tick range at stride 200, mid-magnitude inner params (~710M cases,
    /// ~8 min in release). Extremes live in `boundary_samples` — mixing them in
    /// here would multiply per-case cost. Run with `--ignored` before a release.
    #[test]
    #[ignore]
    fn compute_swap_matches_reference_exhaustive_sweep() {
        const STRIDE: i32 = 200;
        let liquidities: [u128; 3] = [1_000_000, 1u128 << 32, 1u128 << 60];
        let amounts: [u64; 3] = [1_000, 1_000_000_000, 1_000_000_000_000];
        let fee_rates: [u32; 1] = [2_500]; // 0.25% — representative mainstream rate
        let flags: [(bool, bool); 4] = [(true, true), (true, false), (false, true), (false, false)];

        let mut ticks = Vec::new();
        let mut t = tick_math::MIN_TICK;
        while t <= tick_math::MAX_TICK {
            ticks.push(t);
            t += STRIDE;
        }
        if *ticks.last().unwrap() != tick_math::MAX_TICK {
            ticks.push(tick_math::MAX_TICK);
        }
        let total = ticks.len()
            * ticks.len()
            * liquidities.len()
            * amounts.len()
            * fee_rates.len()
            * flags.len();
        eprintln!(
            "exhaustive sweep: {} ticks × {} ticks × {} L × {} amt × {} fee × {} flags = {} cases",
            ticks.len(),
            ticks.len(),
            liquidities.len(),
            amounts.len(),
            fee_rates.len(),
            flags.len(),
            total
        );

        for &tc in &ticks {
            for &tt in &ticks {
                for &l in &liquidities {
                    for &a in &amounts {
                        for &fr in &fee_rates {
                            for &(is_base, is_fee_in) in &flags {
                                assert_equivalent_at_ticks(tc, tt, l, a, fr, is_base, is_fee_in);
                            }
                        }
                    }
                }
            }
        }
    }

    mod fee_calculate_mode_test {
        use super::*;

        // Test all combinations of is_base_input and is_fee_on_input
        #[test]
        fn basic_fee_mode_tests() {
            let sqrt_price_current_x64 = tick_math::get_sqrt_price_at_tick(0).unwrap();
            let liquidity = 1_000_000_000_000u128;
            let fee_rate = 2500u32;

            // Test cases: (is_base_input, is_fee_on_input, zero_for_one, amount_remaining, description)
            let test_cases = vec![
                (
                    true,
                    true,
                    true,
                    1_000_000u64,
                    "exact_input_fee_from_input_zero_for_one",
                ),
                (
                    true,
                    true,
                    false,
                    1_000_000u64,
                    "exact_input_fee_from_input_one_for_zero",
                ),
                (
                    true,
                    false,
                    true,
                    1_000_000u64,
                    "exact_input_fee_from_output_zero_for_one",
                ),
                (
                    true,
                    false,
                    false,
                    1_000_000u64,
                    "exact_input_fee_from_output_one_for_zero",
                ),
                (
                    false,
                    true,
                    true,
                    500_000u64,
                    "exact_output_fee_from_input_zero_for_one",
                ),
                (
                    false,
                    true,
                    false,
                    500_000u64,
                    "exact_output_fee_from_input_one_for_zero",
                ),
                (
                    false,
                    false,
                    true,
                    500_000u64,
                    "exact_output_fee_from_output_zero_for_one",
                ),
                (
                    false,
                    false,
                    false,
                    500_000u64,
                    "exact_output_fee_from_output_one_for_zero",
                ),
            ];

            for (is_base_input, is_fee_on_input, zero_for_one, amount_remaining, _desc) in
                test_cases
            {
                // Use appropriate target price based on direction
                let sqrt_price_target_x64 = if zero_for_one {
                    tick_math::get_sqrt_price_at_tick(-100).unwrap()
                } else {
                    tick_math::get_sqrt_price_at_tick(100).unwrap()
                };

                let result = compute_swap(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    amount_remaining,
                    fee_rate,
                    is_base_input,
                    zero_for_one,
                    is_fee_on_input,
                )
                .unwrap();

                if is_base_input {
                    if is_fee_on_input {
                        // Exact Input + fee from input: amount_remaining is gross input
                        let total_input = result.amount_in.checked_add(result.fee_amount).unwrap();
                        assert_eq!(
                            total_input, amount_remaining,
                            "gross input should equal amount_remaining"
                        );
                        let expected_fee = result
                            .amount_in
                            .mul_div_ceil(
                                fee_rate.into(),
                                (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                            )
                            .unwrap();
                        assert_eq!(
                            result.fee_amount, expected_fee,
                            "fee should be calculated from amount_in"
                        );
                    } else {
                        // Exact Input + fee from output: amount_remaining is net input
                        assert_eq!(
                            result.amount_in, amount_remaining,
                            "net input should equal amount_remaining"
                        );
                        let gross_output =
                            result.amount_out.checked_add(result.fee_amount).unwrap();
                        let expected_fee = gross_output
                            .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                            .unwrap();
                        assert_eq!(
                            result.fee_amount, expected_fee,
                            "fee should be calculated from gross output"
                        );
                    }
                } else {
                    if is_fee_on_input {
                        // Exact Output + fee from input: amount_remaining is gross output
                        assert_eq!(
                            result.amount_out, amount_remaining,
                            "gross output should equal amount_remaining"
                        );
                        let expected_fee = result
                            .amount_in
                            .mul_div_ceil(
                                fee_rate.into(),
                                (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                            )
                            .unwrap();
                        assert_eq!(
                            result.fee_amount, expected_fee,
                            "fee should be calculated from amount_in"
                        );
                    } else {
                        // Exact Output + fee from output: amount_remaining is net output
                        assert_eq!(
                            result.amount_out, amount_remaining,
                            "net output should equal amount_remaining"
                        );
                        let gross_output =
                            result.amount_out.checked_add(result.fee_amount).unwrap();
                        let expected_fee = gross_output
                            .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                            .unwrap();
                        assert_eq!(
                            result.fee_amount, expected_fee,
                            "fee should be calculated from gross output"
                        );
                    }
                }

                // Common assertions
                assert!(result.amount_in > 0, "amount_in should be positive");
                assert!(result.amount_out > 0, "amount_out should be positive");
                assert!(result.fee_amount > 0, "fee_amount should be positive");
            }
        }

        #[test]
        fn compare_fee_from_input_vs_output_exact_input() {
            // Compare fee_from_input vs fee_from_output for exact input swap
            // Test both zero_for_one directions
            let test_cases = vec![
                (
                    true,
                    tick_math::get_sqrt_price_at_tick(0).unwrap(),
                    tick_math::get_sqrt_price_at_tick(-100).unwrap(),
                ),
                (
                    false,
                    tick_math::get_sqrt_price_at_tick(0).unwrap(),
                    tick_math::get_sqrt_price_at_tick(100).unwrap(),
                ),
            ];

            for (zero_for_one, sqrt_price_current_x64, sqrt_price_target_x64) in test_cases {
                let liquidity = 1_000_000_000_000u128;
                let amount_remaining = 1_000_000u64;
                let fee_rate = 2500u32;

                // Test 1: fee from input
                let result1 = compute_swap(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    amount_remaining,
                    fee_rate,
                    true,
                    zero_for_one,
                    true,
                )
                .unwrap();

                // Test 2: fee from output (use net input = gross input - fee from test 1)
                // For fair comparison, use the same net input amount
                let net_input = result1.amount_in;
                let result2 = compute_swap(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    net_input,
                    fee_rate,
                    true,
                    zero_for_one,
                    false,
                )
                .unwrap();

                // Verify: same net input should produce same amount_in
                assert_eq!(
                    result1.amount_in, result2.amount_in,
                    "same net input should produce same amount_in"
                );

                // Verify: output with fee from output should be less (fee deducted from output)
                assert_eq!(result2.amount_out + result2.fee_amount, result1.amount_out,);

                // Verify: fees are calculated correctly
                assert!(result1.fee_amount > 0, "fee from input should be positive");
                assert!(result2.fee_amount > 0, "fee from output should be positive");
            }
        }

        #[test]
        fn compare_fee_from_input_vs_output_exact_output() {
            // Compare fee_from_input vs fee_from_output for exact output swap
            // Test both zero_for_one directions
            let test_cases = vec![
                (
                    true,
                    tick_math::get_sqrt_price_at_tick(0).unwrap(),
                    tick_math::get_sqrt_price_at_tick(-100).unwrap(),
                ),
                (
                    false,
                    tick_math::get_sqrt_price_at_tick(0).unwrap(),
                    tick_math::get_sqrt_price_at_tick(100).unwrap(),
                ),
            ];

            for (zero_for_one, sqrt_price_current_x64, sqrt_price_target_x64) in test_cases {
                let liquidity = 1_000_000_000_000u128;
                let amount_remaining = 500_000u64; // same amount_remaining for both tests
                let fee_rate = 2500u32;

                // Test 1: fee from output (amount_remaining is net output)
                let result1 = compute_swap(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    amount_remaining,
                    fee_rate,
                    false,
                    zero_for_one,
                    false,
                )
                .unwrap();

                // Test 2: fee from input (amount_remaining is gross output)
                let result2 = compute_swap(
                    sqrt_price_current_x64,
                    sqrt_price_target_x64,
                    liquidity,
                    amount_remaining,
                    fee_rate,
                    false,
                    zero_for_one,
                    true,
                )
                .unwrap();

                // Verify: with fee from output, user gets amount_remaining as net output
                assert_eq!(
                    result1.amount_out, amount_remaining,
                    "net output should equal amount_remaining"
                );

                // Verify: with fee from input, user gets amount_remaining as gross output
                assert_eq!(
                    result2.amount_out, amount_remaining,
                    "gross output should equal amount_remaining"
                );

                // Verify: gross output in result1 should include the fee
                let result1_gross_output = result1.amount_out + result1.fee_amount;
                assert!(result1_gross_output >= result2.amount_out,);
                // Verify: gross input in result2 should include the fee
                let result2_gross_input = result2.amount_in + result2.fee_amount;
                assert!(result2_gross_input >= result1.amount_in,);

                // Verify: fee calculation methods
                // result1 fee is calculated from gross output
                let expected_fee1 = result1_gross_output
                    .mul_div_ceil(fee_rate.into(), FEE_RATE_DENOMINATOR_VALUE.into())
                    .unwrap();
                assert_eq!(
                    result1.fee_amount, expected_fee1,
                    "Fee from output should be calculated from gross output"
                );

                // result2 fee is calculated from input
                let expected_fee2 = result2
                    .amount_in
                    .mul_div_ceil(
                        fee_rate.into(),
                        (FEE_RATE_DENOMINATOR_VALUE - fee_rate).into(),
                    )
                    .unwrap();
                assert_eq!(
                    result2.fee_amount, expected_fee2,
                    "Fee from input should be calculated from input"
                );

                // Verify: fees are calculated correctly
                assert!(result1.fee_amount > 0, "fee from output should be positive");
                assert!(result2.fee_amount > 0, "fee from input should be positive");
            }
        }
    }

    /// FLOOR tests (not in upstream). See the file header.
    mod floor_additions {
        use super::*;
        use crate::fixed_point_64;

        const MAX_FEE: u32 = FEE_RATE_DENOMINATOR_VALUE;

        /// `compute_swap` at fee 1e6, exact-in. v4 lets a 100 % fee
        /// execute exact-in: the whole input is fee, nothing moves, nothing comes out.
        #[test]
        fn fee_1e6_exact_in_takes_everything_as_fee_and_moves_nothing() {
            let current = tick_math::get_sqrt_price_at_tick(0).unwrap();
            for &(target_tick, zero_for_one) in &[(-100, true), (100, false)] {
                let target = tick_math::get_sqrt_price_at_tick(target_tick).unwrap();
                for &liquidity in &[1u128, 1_000_000, 1u128 << 64, 1u128 << 100] {
                    for &amount in &[1u64, 1_000, 1_000_000_000, u64::MAX] {
                        let r = compute_swap(
                            current,
                            target,
                            liquidity,
                            amount,
                            MAX_FEE,
                            true,
                            zero_for_one,
                            true,
                        )
                        .unwrap();
                        assert_eq!(
                            r.sqrt_price_next_x64, current,
                            "L={liquidity} amount={amount}"
                        );
                        assert_eq!(r.amount_in, 0);
                        assert_eq!(r.amount_out, 0);
                        assert_eq!(r.fee_amount, amount, "the fee is the whole input");
                    }
                }
            }
        }

        /// A 100 % fee across a zero-liquidity gap (the walk crosses those) and at current == target:
        /// both reach the target with `amount_in == 0`, where upstream divided by zero.
        #[test]
        fn fee_1e6_exact_in_reaching_target_charges_zero_not_a_division_by_zero() {
            let current = tick_math::get_sqrt_price_at_tick(0).unwrap();
            let target = tick_math::get_sqrt_price_at_tick(-60).unwrap();
            let r = compute_swap(current, target, 0, 1_000, MAX_FEE, true, true, true).unwrap();
            assert_eq!(
                (
                    r.sqrt_price_next_x64,
                    r.amount_in,
                    r.amount_out,
                    r.fee_amount
                ),
                (target, 0, 0, 0)
            );
            let r = compute_swap(
                current,
                current,
                1u128 << 64,
                1_000,
                MAX_FEE,
                true,
                true,
                true,
            )
            .unwrap();
            assert_eq!(
                (
                    r.sqrt_price_next_x64,
                    r.amount_in,
                    r.amount_out,
                    r.fee_amount
                ),
                (current, 0, 0, 0)
            );
        }

        /// Exact-in with the fee on OUTPUT at 1e6 also executes: the whole gross output is fee.
        #[test]
        fn fee_1e6_exact_in_fee_on_output_executes() {
            let current = tick_math::get_sqrt_price_at_tick(0).unwrap();
            let target = tick_math::get_sqrt_price_at_tick(-100).unwrap();
            let r = compute_swap(
                current,
                target,
                1u128 << 64,
                1_000,
                MAX_FEE,
                true,
                true,
                false,
            )
            .unwrap();
            assert_eq!(r.amount_out, 0, "net output is zero");
            assert!(r.fee_amount > 0, "the fee is the gross output");
            assert_eq!(r.amount_in, 1_000, "fcad161: the full input is consumed");
        }

        /// The math backstop: exact output at a 100 % fee is a named error on both fee sides.
        #[test]
        fn fee_1e6_exact_out_is_invalid_fee_for_exact_out() {
            let current = tick_math::get_sqrt_price_at_tick(0).unwrap();
            let target = tick_math::get_sqrt_price_at_tick(-100).unwrap();
            for &fee_on_input in &[true, false] {
                assert_eq!(
                    compute_swap(
                        current,
                        target,
                        1u128 << 64,
                        1_000,
                        MAX_FEE,
                        false,
                        true,
                        fee_on_input
                    )
                    .unwrap_err(),
                    MathError::InvalidFeeForExactOut
                );
            }
            // one pip under is still fine on the v4 (fee-on-input) path
            compute_swap(
                current,
                target,
                1u128 << 64,
                1_000,
                MAX_FEE - 1,
                false,
                true,
                true,
            )
            .unwrap();
        }

        #[test]
        fn fee_rate_above_1e6_is_refused_before_any_arithmetic() {
            let current = tick_math::get_sqrt_price_at_tick(0).unwrap();
            let target = tick_math::get_sqrt_price_at_tick(-100).unwrap();
            for &(base, on_input) in &[(true, true), (true, false), (false, true), (false, false)] {
                assert_eq!(
                    compute_swap(
                        current,
                        target,
                        1u128 << 64,
                        1_000,
                        MAX_FEE + 1,
                        base,
                        true,
                        on_input
                    )
                    .unwrap_err(),
                    MathError::FeeRateTooHigh
                );
            }
        }

        /// An inverted `zero_for_one` is refused before any arithmetic, on every fee side and amount
        /// mode, where upstream returned a value-extracting step (price up, token0 in, token1 out).
        #[test]
        fn inverted_direction_is_refused() {
            let at0 = tick_math::get_sqrt_price_at_tick(0).unwrap();
            let up = tick_math::get_sqrt_price_at_tick(100).unwrap();
            let down = tick_math::get_sqrt_price_at_tick(-100).unwrap();
            let liquidity = 1u128 << 70;
            for (current, target, zfo) in [(at0, up, true), (at0, down, false)] {
                for base in [true, false] {
                    for on_input in [true, false] {
                        for amount in [1u64, 1_000_000, u64::MAX] {
                            assert_eq!(
                                compute_swap(
                                    current, target, liquidity, amount, 2_500, base, zfo, on_input
                                )
                                .unwrap_err(),
                                MathError::DirectionMismatch,
                                "zfo={zfo} base={base} on_input={on_input} amount={amount}"
                            );
                        }
                    }
                }
            }
            // the same steps with the right flag succeed, and a zero-width step takes either flag
            assert!(compute_swap(at0, up, liquidity, 1_000_000, 2_500, true, false, true).is_ok());
            assert!(compute_swap(at0, down, liquidity, 1_000_000, 2_500, true, true, true).is_ok());
            for zfo in [true, false] {
                let r =
                    compute_swap(at0, at0, liquidity, 1_000_000, 2_500, true, zfo, true).unwrap();
                assert_eq!(
                    (r.sqrt_price_next_x64, r.amount_in, r.amount_out),
                    (at0, 0, 0)
                );
            }
        }

        /// The v4 LP-fee split on a partial exact-in step: `amount_in` is exactly the fee-less
        /// amount, never the (smaller) price-derived one, and the price-derived one IS smaller in at
        /// least one case, so the test is not vacuous.
        #[test]
        fn partial_exact_in_step_books_the_fee_less_amount_as_input() {
            let current = tick_math::get_sqrt_price_at_tick(0).unwrap();
            let target = tick_math::get_sqrt_price_at_tick(-10_000).unwrap();
            let liquidity = 1u128 << 70;
            let mut saw_rounding_dust = false;
            for amount in [1_000_001u64, 3_333_333, 987_654_321, 1_000_000_007] {
                for fee in [0u32, 1, 2_500, 30_000, 999_999] {
                    let r = compute_swap(current, target, liquidity, amount, fee, true, true, true)
                        .unwrap();
                    assert_ne!(r.sqrt_price_next_x64, target, "precondition: partial step");
                    let less_fee = amount
                        .mul_div_floor((MAX_FEE - fee).into(), MAX_FEE.into())
                        .unwrap();
                    assert_eq!(r.amount_in, less_fee);
                    assert_eq!(r.amount_in + r.fee_amount, amount);
                    let price_derived = liquidity_math::get_delta_amount_0_unsigned(
                        r.sqrt_price_next_x64,
                        current,
                        liquidity,
                        true,
                    )
                    .unwrap();
                    assert!(price_derived <= less_fee);
                    saw_rounding_dust |= price_derived < less_fee;
                }
            }
            assert!(
                saw_rounding_dust,
                "no case exercised the upstream/v4 difference"
            );
        }

        /// Raydium fcad161, `exact_input_fee_from_output_dust_is_consumed_{one_for_zero,zero_for_one}`
        /// (upstream swap.rs tests), re-expressed over `compute_swap`: liquidity 100·Q64, fee 0, the
        /// fee on OUTPUT, 150 units in, one initialized boundary at tick 59 (tick 0 for zero_for_one)
        /// whose crossing would add 1 to liquidity, limit at the far price bound. 150 units at that
        /// liquidity never reach the boundary (tick 59 needs ~0.295·2^64 units), so the whole swap is one
        /// partial step. Upstream's comment: "amount_in=100, dust=50". Before the fix that step booked
        /// 100, left 50 of dust that could never move the price, and the loop spun until CU exhaustion.
        /// The loop must end after that one step and consume all 150.
        #[test]
        fn fcad161_exact_input_fee_from_output_dust_is_consumed() {
            // FLOOR: parameterised over the fee side and the fee. Upstream ran fee 0 on output only; the
            // AMM's swap walk always calls `is_fee_on_input = true`, so that path carries the regression too,
            // including fee 0 (dust that is neither fee nor principal) and 999,999 pips (all fee).
            let liquidity = 100u128 * fixed_point_64::Q64;
            for is_fee_on_input in [false, true] {
                for fee in [0u32, 2_500, 999_999] {
                    for &(zero_for_one, start_tick, boundary_tick, limit) in &[
                        (false, 0, 59, tick_math::MAX_SQRT_PRICE_X64 - 1),
                        (true, 59, 0, tick_math::MIN_SQRT_PRICE_X64 + 1),
                    ] {
                        let case = format!(
                            "is_fee_on_input={is_fee_on_input} fee={fee} zero_for_one={zero_for_one}"
                        );
                        let boundary = tick_math::get_sqrt_price_at_tick(boundary_tick).unwrap();
                        let mut price = tick_math::get_sqrt_price_at_tick(start_tick).unwrap();
                        let mut l = liquidity;
                        let mut remaining = 150u64;
                        let mut consumed = 0u64;
                        let mut crossed = false;
                        let mut saw_dust = false;
                        let mut iterations = 0;
                        while remaining != 0 && price != limit {
                            iterations += 1;
                            assert!(iterations < 1_000, "no-progress loop ({case})");
                            let target = if crossed { limit } else { boundary };
                            let r = compute_swap(
                                price,
                                target,
                                l,
                                remaining,
                                fee,
                                true,
                                zero_for_one,
                                is_fee_on_input,
                            )
                            .unwrap();
                            if r.sqrt_price_next_x64 != target {
                                // the dust precondition upstream's comment describes
                                let from_price = if zero_for_one {
                                    liquidity_math::get_delta_amount_0_unsigned(
                                        r.sqrt_price_next_x64,
                                        price,
                                        l,
                                        true,
                                    )
                                } else {
                                    liquidity_math::get_delta_amount_1_unsigned(
                                        price,
                                        r.sqrt_price_next_x64,
                                        l,
                                        true,
                                    )
                                }
                                .unwrap();
                                if !zero_for_one && !is_fee_on_input && fee == 0 {
                                    assert_eq!(
                                        (from_price, remaining),
                                        (100, 150),
                                        "upstream's dust=50 case"
                                    );
                                }
                                saw_dust |= from_price < remaining;
                            }
                            // the walk's bookkeeping: a fee on input is paid out of the exact input
                            let spent = if is_fee_on_input {
                                assert_eq!(
                                    r.amount_in + r.fee_amount,
                                    remaining,
                                    "a fee-on-input partial step books the whole input ({case})"
                                );
                                r.amount_in + r.fee_amount
                            } else {
                                r.amount_in
                            };
                            remaining -= spent;
                            consumed += spent;
                            price = r.sqrt_price_next_x64;
                            if !crossed && price == boundary {
                                crossed = true;
                                l += 1;
                            }
                        }
                        assert!(
                            !crossed,
                            "the upstream scenario never reaches its boundary ({case})"
                        );
                        assert_eq!(
                            iterations, 1,
                            "one partial step consumes everything ({case})"
                        );
                        assert!(
                            saw_dust,
                            "precondition: the final partial step left price-derived dust ({case})"
                        );
                        assert_eq!(
                            consumed, 150,
                            "the full exact input must be consumed (dust accrues to the pool) ({case})"
                        );
                    }
                }
            }
        }
    }
}
