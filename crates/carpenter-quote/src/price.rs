//! price ↔ tick.
//!
//! The exact functions are integer (`sqrt_price_at_tick`, `tick_at_sqrt_price`, the usable-tick
//! bounds). A hook that snaps ticks to its own grid (FLOOR's `floor_to_spacing` / `ceil_to_spacing`)
//! ships that snap in its own crate: it is the hook's rule, not the AMM's.
//! The `*_ui` functions take and return `f64` human prices for DISPLAY and for turning a typed price
//! into a tick; nothing that settles is ever computed from them (a typed price is snapped to a tick,
//! and the tick is what the transaction carries; a UI shows the tick's price back).
//!
//! Pool price is token1 per token0 in base units; the UI price is token1 per token0 in whole tokens:
//! `ui = raw · 10^(decimals0 − decimals1)`.

use carpenter_clmm_math::tick_math;
use carpenter_types::constants::{MAX_TICK, MIN_TICK};

use crate::error::{QuoteError, QuoteResult};

pub fn sqrt_price_at_tick(tick: i32) -> QuoteResult<u128> {
    Ok(tick_math::get_sqrt_price_at_tick(tick)?)
}

pub fn tick_at_sqrt_price(sqrt_price_x64: u128) -> QuoteResult<i32> {
    Ok(tick_math::get_tick_at_sqrt_price(sqrt_price_x64)?)
}

/// v4 `TickMath.minUsableTick`: the lowest multiple of `spacing` inside the tick range. Display and
/// range UI only; never a hook's snap (a hook's own grid rule lives in that hook's crate).
pub fn min_usable_tick(spacing: u16) -> QuoteResult<i32> {
    let s = spacing_i32(spacing)?;
    Ok(-((-MIN_TICK).div_euclid(s) * s))
}

/// v4 `TickMath.maxUsableTick`. Display and range UI only; never a hook's snap.
pub fn max_usable_tick(spacing: u16) -> QuoteResult<i32> {
    let s = spacing_i32(spacing)?;
    Ok(MAX_TICK.div_euclid(s) * s)
}

fn spacing_i32(spacing: u16) -> QuoteResult<i32> {
    if spacing == 0 {
        return Err(QuoteError::Math(carpenter_clmm_math::MathError::InvalidTickSpacing));
    }
    Ok(i32::from(spacing))
}

/// Human price (token1 per token0, whole tokens) at a sqrt price. Display only.
pub fn price_ui_at_sqrt(sqrt_price_x64: u128, decimals0: u8, decimals1: u8) -> f64 {
    let s = sqrt_price_x64 as f64 / 18_446_744_073_709_551_616.0;
    s * s * 10f64.powi(i32::from(decimals0) - i32::from(decimals1))
}

/// Human price at a tick. Display only.
pub fn price_ui_at_tick(tick: i32, decimals0: u8, decimals1: u8) -> QuoteResult<f64> {
    Ok(price_ui_at_sqrt(sqrt_price_at_tick(tick)?, decimals0, decimals1))
}

/// The tick whose price is at or below a typed human price (then snap it to the pool's grid).
/// Resolved exactly on the integer side: the float only picks a starting tick, and the result is the
/// greatest tick with `price_ui_at_tick(t) <= price`.
pub fn tick_at_price_ui(price: f64, decimals0: u8, decimals1: u8) -> QuoteResult<i32> {
    if !(price.is_finite() && price > 0.0) {
        return Err(QuoteError::Math(carpenter_clmm_math::MathError::SqrtPriceOutOfRange));
    }
    let raw = price / 10f64.powi(i32::from(decimals0) - i32::from(decimals1));
    let guess = (raw.ln() / 1.0001f64.ln()).floor();
    let mut t = if guess.is_finite() { (guess as i64).clamp(MIN_TICK as i64, MAX_TICK as i64) as i32 } else { MIN_TICK };
    // Walk to the exact answer; the float guess is within a few ticks.
    while t < MAX_TICK && price_ui_at_tick(t + 1, decimals0, decimals1)? <= price {
        t += 1;
    }
    while t > MIN_TICK && price_ui_at_tick(t, decimals0, decimals1)? > price {
        t -= 1;
    }
    Ok(t)
}
