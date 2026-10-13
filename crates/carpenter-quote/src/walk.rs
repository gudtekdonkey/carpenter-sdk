//! The swap walk: v4 `Pool.swap` semantics over fixed tick arrays, one step at a time through
//! `carpenter_clmm_math::swap_math::compute_swap(…, is_fee_on_input = true)`.
//!
//! THE STEP-BOUNDARY CONTRACT. Rounding happens once per step, so two walks that split the same
//! price move at different ticks disagree by units. This walk stops ONLY at: an initialized tick
//! (`liquidity_gross != 0`), the price limit, `MIN_TICK` / `MAX_TICK` when no initialized tick remains,
//! or where the amount runs out. It never stops at an uninitialized array edge. The next tick is:
//! - `zero_for_one`: the greatest initialized tick `<= tick_current` (v4's `lte` search);
//! - `!zero_for_one`: the least initialized tick `> tick_current`;
//!
//! searched first in the array holding `tick_current` (only if the bitmap marks it), then in each next
//! array the bitmap marks, strictly past, in the swap direction (`bitmap::next_initialized_array_start_index`).
//! That is Raydium `swap_internal`'s search, with v4's end-of-range behaviour (walk to the bound with
//! zero liquidity) in place of Raydium's `LiquidityInsufficient`. The AMM program's walk makes the
//! same choice, and the program repository's quote-parity test holds the two together.
//!
//! Tick after a step (v4): reaching the step's tick sets `tick = zero_for_one ? next - 1 : next`
//! (crossing it first if initialized); a partial move sets `tick = get_tick_at_sqrt_price(sqrt)`;
//! a zero move leaves it.
//!
//! THE CARPENTER PLATFORM FEE, mirrored from the program's walk through the ONE implementation both
//! run, `carpenter_types::protocol_fee`: every walk — the trader's AND every action's (the in-swap
//! nudge pays too) — charges the pool's `protocol_fee` in its
//! `protocol_fee_currency`. On the input: v4's composed step rate, the protocol's share split off
//! each step fee (`input_share`), so `fee_total` and fee growth are the LP half only. On the OUTPUT
//! (the SOL / quote side, e.g. a sell on a FLOOR pool): the steps run at the LP fee, ONE `output_share`
//! comes off the walk's whole output, and an exact-out walk first grosses its target up
//! (`gross_for_net`). Either way the share is booked into `pool.protocol_fees_owed0/1`, exactly as
//! the chain books it, so `pool_after` stays byte-equal.

use carpenter_clmm_math::{bitmap, liquidity_math, swap_math, tick_math};
use carpenter_types::constants::{MAX_SQRT_PRICE_X64, MAX_TICK, MIN_SQRT_PRICE_X64, MIN_TICK, TICK_ARRAY_SIZE};
use carpenter_types::hook::BalanceDelta;
use carpenter_types::layout::{Pool, TickArray};
use carpenter_types::protocol_fee::{self, WalkFee};
use carpenter_types::U128;

use crate::error::{QuoteError, QuoteResult};
use crate::state::TickArraySet;

/// Which error a missing array is: the trader's walk (`TickArrayMissing`) or a hook action's
/// (`HookActionAccountMissing`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WalkKind {
    Trader,
    Action,
}

impl WalkKind {
    pub(crate) fn missing(self, start_index: i32) -> QuoteError {
        match self {
            WalkKind::Trader => QuoteError::TickArrayMissing { start_index },
            WalkKind::Action => QuoteError::HookActionAccountMissing { start_index },
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WalkParams {
    pub zero_for_one: bool,
    /// v4 sign: `< 0` exact-in. Never 0 (the caller skips the walk, `Pool.sol:320`).
    pub amount_specified: i128,
    pub sqrt_price_limit_x64: u128,
    /// The resolved LP fee, pips (override, `pool.lp_fee`, or `pool.lp_fee` for an action). The
    /// pool's Carpenter Platform fee is read off the pool itself and composed onto it where it rides
    /// the input ([`fee_for`]).
    pub fee_pips: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WalkOutcome {
    /// Caller perspective (v4): negative = paid into the pool. NET of a protocol fee on the output.
    pub swap_delta: BalanceDelta,
    /// Σ step LP fees, in the input currency — the protocol's share excluded.
    pub fee_total: u64,
    /// The Carpenter Platform fee this walk booked owed, in
    /// `protocol_currency`.
    pub protocol_fee: u64,
    /// The currency (0 / 1) `protocol_fee` is in.
    pub protocol_currency: u8,
    /// Start indices of every array whose ticks this walk read, in first-read order.
    pub arrays_read: Vec<i32>,
    pub steps: u32,
}

/// A walk that has not terminated after this many steps is a quoter bug, not a price: refuse it
/// rather than hang a browser tab. No reachable swap comes near (each step crosses a tick or ends).
const MAX_STEPS: u32 = 1_000_000;

/// v4 `Pool.sol:322-337`, as one check.
pub fn check_price_limit(pool: &Pool, zero_for_one: bool, limit: u128) -> QuoteResult<()> {
    let sqrt = pool.sqrt_price_x64.get();
    let ok = if zero_for_one {
        limit < sqrt && limit > MIN_SQRT_PRICE_X64
    } else {
        limit > sqrt && limit < MAX_SQRT_PRICE_X64
    };
    if ok {
        Ok(())
    } else {
        Err(QuoteError::PriceLimitInvalid)
    }
}

/// The rates a walk in this direction charges on `pool` at LP fee `lp_fee_pips`: the program's
/// `protocol_fee::walk_fee` over the pool's stored `protocol_fee` / `protocol_fee_currency`. The
/// caller's exact-out fee check reads `swap_fee` from here (the composed rate when the fee rides the input).
pub fn fee_for(pool: &Pool, zero_for_one: bool, lp_fee_pips: u32) -> WalkFee {
    protocol_fee::walk_fee(pool.protocol_fee, pool.protocol_fee_currency, zero_for_one, lp_fee_pips)
}

/// Run one walk, mutating `pool` (price, tick, liquidity, the input side's fee growth, the protocol
/// fee owed) and the crossed ticks in `set`. The caller has already run [`check_price_limit`].
pub fn walk(pool: &mut Pool, set: &mut TickArraySet, p: &WalkParams, kind: WalkKind) -> QuoteResult<WalkOutcome> {
    if p.amount_specified == 0 {
        return Err(QuoteError::AmountZero);
    }
    let zfo = p.zero_for_one;
    let exact_in = p.amount_specified < 0;
    let limit = p.sqrt_price_limit_x64;
    let wf = fee_for(pool, zfo, p.fee_pips);

    // An exact-out walk whose protocol fee rides the OUTPUT walks to the gross that nets
    // exactly what was named (the program's `walk` does the same, from the same function).
    let target: i128 = if !exact_in && wf.output_pips > 0 {
        let named = u64::try_from(p.amount_specified).map_err(|_| QuoteError::AmountTooLarge)?;
        i128::from(protocol_fee::gross_for_net(named, wf.output_pips).ok_or(QuoteError::Overflow)?)
    } else {
        p.amount_specified
    };
    let mut remaining: i128 = target;
    let mut calculated: i128 = 0;
    let mut sqrt = pool.sqrt_price_x64.get();
    let mut tick = pool.tick_current;
    let mut liquidity = pool.liquidity.get();
    let mut fg_in = if zfo { pool.fee_growth_global0_x64.get() } else { pool.fee_growth_global1_x64.get() };
    let fg_other = if zfo { pool.fee_growth_global1_x64.get() } else { pool.fee_growth_global0_x64.get() };

    let mut out = WalkOutcome::default();

    while !(remaining == 0 || sqrt == limit) {
        out.steps += 1;
        if out.steps > MAX_STEPS {
            return Err(QuoteError::Overflow);
        }
        let sqrt_start = sqrt;
        let (mut tick_next, initialized) = next_initialized_tick(set, tick, zfo, kind, &mut out.arrays_read)?;
        tick_next = tick_next.clamp(MIN_TICK, MAX_TICK);
        let sqrt_next = tick_math::get_sqrt_price_at_tick(tick_next)?;
        let target = if (zfo && sqrt_next < limit) || (!zfo && sqrt_next > limit) { limit } else { sqrt_next };

        let amount_remaining = u64::try_from(remaining.unsigned_abs()).map_err(|_| QuoteError::AmountTooLarge)?;
        let step = swap_math::compute_swap(sqrt, target, liquidity, amount_remaining, wf.swap_fee, exact_in, zfo, true)?;
        sqrt = step.sqrt_price_next_x64;

        let in_plus_fee = (step.amount_in as i128) + (step.fee_amount as i128);
        if exact_in {
            remaining += in_plus_fee;
            calculated += step.amount_out as i128;
        } else {
            remaining -= step.amount_out as i128;
            calculated -= in_plus_fee;
        }
        // The protocol's share of the step fee when it rides the input (v4's split,
        // `protocol_fee::input_share`, the program's exact call); the LPs keep the rest.
        let gross_in = step.amount_in.checked_add(step.fee_amount).ok_or(QuoteError::Overflow)?;
        let share = protocol_fee::input_share(step.fee_amount, gross_in, wf.swap_fee, wf.input_pips).ok_or(QuoteError::Overflow)?;
        let lp_fee = step.fee_amount - share;
        out.protocol_fee = out.protocol_fee.checked_add(share).ok_or(QuoteError::Overflow)?;
        out.fee_total = out.fee_total.checked_add(lp_fee).ok_or(QuoteError::Overflow)?;

        // Fee growth: LP fee · 2^64 / L, floor, wrapping. fee < 2^64, so the shift fits a u128.
        if liquidity > 0 {
            fg_in = fg_in.wrapping_add(((lp_fee as u128) << 64) / liquidity);
        }

        if sqrt == sqrt_next {
            if initialized {
                let (g0, g1) = if zfo { (fg_in, fg_other) } else { (fg_other, fg_in) };
                let mut net = cross_tick(set, tick_next, g0, g1, kind)?;
                if zfo {
                    net = net.checked_neg().ok_or(QuoteError::Overflow)?;
                }
                liquidity = liquidity_math::add_delta(liquidity, net)?;
            }
            tick = if zfo { tick_next - 1 } else { tick_next };
        } else if sqrt != sqrt_start {
            tick = tick_math::get_tick_at_sqrt_price(sqrt)?;
        }
    }

    pool.sqrt_price_x64 = U128::new(sqrt);
    pool.tick_current = tick;
    pool.liquidity = U128::new(liquidity);
    if zfo {
        pool.fee_growth_global0_x64 = U128::new(fg_in);
    } else {
        pool.fee_growth_global1_x64 = U128::new(fg_in);
    }

    // v4: `zeroForOne != (amountSpecified < 0)` puts the calculated side on currency0.
    let specified_used = target - remaining;
    out.swap_delta = if zfo != exact_in {
        BalanceDelta { amount0: calculated, amount1: specified_used }
    } else {
        BalanceDelta { amount0: specified_used, amount1: calculated }
    };

    // A fee on the OUTPUT is ONE floor off the whole output (the
    // positive leg), which the vault keeps, owed. 0 when the fee rides the input or is 0.
    if wf.output_pips > 0 {
        let leg = if zfo { &mut out.swap_delta.amount1 } else { &mut out.swap_delta.amount0 };
        let gross_out = u64::try_from(*leg).map_err(|_| QuoteError::Overflow)?;
        let share = protocol_fee::output_share(gross_out, wf.output_pips);
        *leg -= i128::from(share);
        out.protocol_fee = out.protocol_fee.checked_add(share).ok_or(QuoteError::Overflow)?;
    }
    out.protocol_currency = wf.currency;
    if wf.currency == 0 {
        pool.protocol_fees_owed0 = pool.protocol_fees_owed0.checked_add(out.protocol_fee).ok_or(QuoteError::Overflow)?;
    } else {
        pool.protocol_fees_owed1 = pool.protocol_fees_owed1.checked_add(out.protocol_fee).ok_or(QuoteError::Overflow)?;
    }
    Ok(out)
}

/// The next tick the walk steps to, and whether it is initialized (see the module contract).
fn next_initialized_tick(
    set: &TickArraySet,
    tick: i32,
    zfo: bool,
    kind: WalkKind,
    read: &mut Vec<i32>,
) -> QuoteResult<(i32, bool)> {
    let bound = if zfo { MIN_TICK } else { MAX_TICK };
    if !(MIN_TICK..=MAX_TICK).contains(&tick) {
        // Only reachable one tick below MIN_TICK after crossing an initialized MIN_TICK, which the
        // price limit (> MIN_SQRT_PRICE) forbids; answer the bound rather than index out of range.
        return Ok((bound, false));
    }
    let spacing = set.tick_spacing;
    let s = i32::from(spacing);
    let start = bitmap::array_start_index(tick, spacing)?;

    if set.is_marked(start)? {
        let array = load(set, start, kind, read)?;
        let offset = (tick - start).div_euclid(s);
        let found = if zfo {
            let top = offset.min(TICK_ARRAY_SIZE - 1);
            (0..=top).rev().find(|&i| is_init(array, i))
        } else {
            ((offset + 1)..TICK_ARRAY_SIZE).find(|&i| is_init(array, i))
        };
        if let Some(i) = found {
            return Ok((start + i * s, true));
        }
    }

    let mut last = start;
    loop {
        match bitmap::next_initialized_array_start_index(&set.words, spacing, last, zfo)? {
            None => return Ok((bound, false)),
            Some(next) => {
                let array = load(set, next, kind, read)?;
                let found = if zfo {
                    (0..TICK_ARRAY_SIZE).rev().find(|&i| is_init(array, i))
                } else {
                    (0..TICK_ARRAY_SIZE).find(|&i| is_init(array, i))
                };
                if let Some(i) = found {
                    return Ok((next + i * s, true));
                }
                // A marked array with no initialized tick is a stale bit; keep searching past it.
                last = next;
            }
        }
    }
}

fn is_init(array: &TickArray, i: i32) -> bool {
    array.ticks[i as usize].liquidity_gross.get() != 0
}

fn load<'a>(set: &'a TickArraySet, start: i32, kind: WalkKind, read: &mut Vec<i32>) -> QuoteResult<&'a TickArray> {
    let array = set.arrays.get(&start).ok_or_else(|| kind.missing(start))?;
    if !read.contains(&start) {
        read.push(start);
    }
    Ok(array)
}

/// v4 `Pool.crossTick`: flip both `fee_growth_outside` against the in-flight globals (wrapping) and
/// return `liquidity_net`.
fn cross_tick(set: &mut TickArraySet, tick: i32, g0: u128, g1: u128, kind: WalkKind) -> QuoteResult<i128> {
    let spacing = set.tick_spacing;
    let start = bitmap::array_start_index(tick, spacing)?;
    let i = ((tick - start) / i32::from(spacing)) as usize;
    let array = set.arrays.get_mut(&start).ok_or_else(|| kind.missing(start))?;
    let t = &mut array.ticks[i];
    t.fee_growth_outside0_x64 = U128::new(g0.wrapping_sub(t.fee_growth_outside0_x64.get()));
    t.fee_growth_outside1_x64 = U128::new(g1.wrapping_sub(t.fee_growth_outside1_x64.get()));
    Ok(t.liquidity_net.get())
}
