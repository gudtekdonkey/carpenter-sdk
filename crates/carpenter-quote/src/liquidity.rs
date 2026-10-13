//! Liquidity helpers: what a `modify_liquidity` moves, the liquidity an amount buys, and the fees a
//! position has accrued (as `modify_liquidity` computes them).

use carpenter_clmm_math::big_num::U256;
use carpenter_clmm_math::{liquidity_math, tick_math};
use carpenter_types::constants::{MAX_TICK, MIN_TICK};
use carpenter_types::layout::{Pool, Position, Tick};

use crate::error::{QuoteError, QuoteResult};

/// The token amounts a `liquidity_delta` on `[tick_lower, tick_upper)` moves at the pool's price,
/// in v4 `Pool.modifyLiquidity`'s shape: branches on the TICK (`tick < lower`, `tick < upper`),
/// rounds UP when adding (what the owner pays) and DOWN when removing (what the owner receives).
/// Returned as magnitudes; the sign is the delta's.
pub fn amounts_for_liquidity(
    sqrt_price_x64: u128,
    tick_current: i32,
    tick_lower: i32,
    tick_upper: i32,
    liquidity_delta: i128,
) -> QuoteResult<(u64, u64)> {
    check_range(tick_lower, tick_upper)?;
    if liquidity_delta == 0 {
        return Ok((0, 0));
    }
    let round_up = liquidity_delta > 0;
    let l = liquidity_delta.unsigned_abs();
    let sa = tick_math::get_sqrt_price_at_tick(tick_lower)?;
    let sb = tick_math::get_sqrt_price_at_tick(tick_upper)?;
    if tick_current < tick_lower {
        Ok((liquidity_math::get_delta_amount_0_unsigned(sa, sb, l, round_up)?, 0))
    } else if tick_current < tick_upper {
        Ok((
            liquidity_math::get_delta_amount_0_unsigned(sqrt_price_x64, sb, l, round_up)?,
            liquidity_math::get_delta_amount_1_unsigned(sa, sqrt_price_x64, l, round_up)?,
        ))
    } else {
        Ok((0, liquidity_math::get_delta_amount_1_unsigned(sa, sb, l, round_up)?))
    }
}

/// The most liquidity `amount0_max` / `amount1_max` buy on `[tick_lower, tick_upper)` at `sqrt_price_x64`
/// (`get_liquidity_from_amounts`, floor rounding). Feed the result back through
/// [`amounts_for_liquidity`] for what the add will actually charge; it never exceeds either maximum.
pub fn quote_liquidity(sqrt_price_x64: u128, tick_lower: i32, tick_upper: i32, amount0_max: u64, amount1_max: u64) -> QuoteResult<u128> {
    check_range(tick_lower, tick_upper)?;
    let sa = tick_math::get_sqrt_price_at_tick(tick_lower)?;
    let sb = tick_math::get_sqrt_price_at_tick(tick_upper)?;
    Ok(liquidity_math::get_liquidity_from_amounts(sqrt_price_x64, sa, sb, amount0_max, amount1_max)?)
}

fn check_range(lower: i32, upper: i32) -> QuoteResult<()> {
    if lower >= upper || lower < MIN_TICK || upper > MAX_TICK {
        return Err(QuoteError::Math(carpenter_clmm_math::MathError::TickOutOfRange));
    }
    Ok(())
}

/// v4 `Pool.getFeeGrowthInside`, wrapping: `(inside0, inside1)`.
pub fn fee_growth_inside(pool: &Pool, tick_lower: i32, lower: &Tick, tick_upper: i32, upper: &Tick) -> (u128, u128) {
    let t = pool.tick_current;
    let g0 = pool.fee_growth_global0_x64.get();
    let g1 = pool.fee_growth_global1_x64.get();
    let (lo0, lo1) = (lower.fee_growth_outside0_x64.get(), lower.fee_growth_outside1_x64.get());
    let (up0, up1) = (upper.fee_growth_outside0_x64.get(), upper.fee_growth_outside1_x64.get());
    if t < tick_lower {
        (lo0.wrapping_sub(up0), lo1.wrapping_sub(up1))
    } else if t >= tick_upper {
        (up0.wrapping_sub(lo0), up1.wrapping_sub(lo1))
    } else {
        (g0.wrapping_sub(lo0).wrapping_sub(up0), g1.wrapping_sub(lo1).wrapping_sub(up1))
    }
}

/// The fees a position would be paid at its next `modify_liquidity`: `(inside − last) · L >> 64` in
/// U256, per currency (fee growth wraps).
///
/// This is the amount OWED. The chain pays `min(owed, u64::MAX, what the vault holds beyond claims,
/// protocol fees and this operation's principal)` and emits `FeesTruncated` when it clamps; the
/// vault term needs the vault balance, so the clamp to `u64::MAX` is applied here and the vault clamp
/// is the caller's.
pub fn fees_owed(pool: &Pool, position: &Position, lower: &Tick, upper: &Tick) -> (u64, u64) {
    let (in0, in1) = fee_growth_inside(pool, position.tick_lower, lower, position.tick_upper, upper);
    let l = position.liquidity.get();
    let owed = |inside: u128, last: u128| -> u64 {
        let delta = inside.wrapping_sub(last);
        let v: U256 = (U256::from(delta) * U256::from(l)) >> 64;
        if v > U256::from(u64::MAX) {
            u64::MAX
        } else {
            v.as_u64()
        }
    };
    (
        owed(in0, position.fee_growth_inside0_last_x64.get()),
        owed(in1, position.fee_growth_inside1_last_x64.get()),
    )
}
