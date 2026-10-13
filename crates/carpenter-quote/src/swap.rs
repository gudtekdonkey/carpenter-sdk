//! `quote_swap`: the AMM's swap round protocol replayed off-chain, for plain and hooked pools.
//!
//! The order below is the AMM's, line for line, so a refusal surfaces with the name and at the moment
//! the chain would raise it:
//!
//! ```text
//! paused → prelude (amount ≠ 0, ≤ u64::MAX) → R0 before_swap (if BEFORE_SWAP) → amount_to_swap (flip check)
//! → fee = override ?? lp_fee → exact-out fee check → [skip if amount_to_swap == 0] price-limit check → walk
//! → R1 after_swap (if AFTER_SWAP) → hook_delta (the ONE credit) → trader_delta → claims += hook_delta
//! → actions (hooks skipped, lp_fee = pool.lp_fee) → after_actions rounds (capped by hook_max_rounds)
//! → trader in ≤ 0 ∧ out ≥ 0 → counters
//! ```
//!
//! The hook callbacks are a [`HookQuoter`]'s (`crate::hook`); this module knows no hook by name.
//!
//! A pool whose hook carries `SETTLES_FLAG` adds, in the program's order: after every callback, the
//! hook's [`HookQuoter::settled`] mint credited to its claims; a negative `before_swap`
//! `delta_specified` may not enlarge the walk (`HookDeltaExceedsSwapAmount`); `before_swap`'s
//! `sqrt_price_limit_x64` caps the walk at the tighter of it and the trader's limit, and skips the
//! walk when the price already sits at or past it; `after_swap`'s `delta_specified` takes at most the
//! exact-in remainder the walk left unfilled (`HookSettleOverfill`); and a negative hook delta pays
//! the trader out of the hook's claims (`HookClaimsInsufficient` when they do not cover it), counted
//! in no `hook_fees` counter.
//!
//! The Carpenter Platform fee: the exact-out fee check reads the rate the steps are charged (`walk::fee_for` — composed
//! with the pool's `protocol_fee` where it rides the input), and every walk — the trader's and each
//! action's — charges the fee as the chain does (`walk.rs`'s header). `fees.protocol` /
//! `fees.protocol_currency` are the trader walk's (`Swapped.protocol_fee_total` /
//! `protocol_fee_currency`); `pool_after.protocol_fees_owed0/1` carry every walk's.
//!
//! Deterministic and lamport-exact by construction: integers only, the programs' own math crates,
//! and no clock, randomness, float or RPC. The one input a pool account does not carry is the mints'
//! supplies (`AfterSwapArgs.supply0/1`), which a hook's callbacks receive; they are parameters.
//!

use carpenter_types::constants::{AFTER_SWAP_FLAG, BEFORE_SWAP_FLAG, DYNAMIC_FEE_FLAG, MAX_SWAP_FEE, SETTLES_FLAG};
use carpenter_types::hook::{
    self as hk, ActionResult, BalanceDelta, FeeSplit, HookAction, HookDeltaAssembly, HookRecordV1, RecordExpect,
    HOOK_INTERFACE_VERSION, PHASE_AFTER_ACTIONS, PHASE_AFTER_SWAP, PHASE_BEFORE_SWAP,
};
use carpenter_types::layout::{Pool, TickArray};
use carpenter_types::U128;

use crate::error::{QuoteError, QuoteResult};
use crate::hook::{AfterSwapIn, HookQuoter, HookSwapCtx};
use crate::modify;
use crate::state::TickArraySet;
use crate::walk::{self, WalkKind, WalkParams};

/// A pool account and the key it lives at (the key is not stored in the account, and a hook's
/// callbacks bind their state to it).
#[derive(Clone, Copy, Debug)]
pub struct PoolAccount {
    pub key: [u8; 32],
    pub pool: Pool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapQuoteParams {
    pub zero_for_one: bool,
    /// v4 sign: `< 0` exact-in, `> 0` exact-out.
    pub amount_specified: i128,
    pub sqrt_price_limit_x64: u128,
    /// `mint0.supply`, as the AMM reads it into the hook callbacks' args. Unused on a plain pool.
    pub supply0: u64,
    /// `mint1.supply`, as the AMM reads it into the hook callbacks' args. Unused on a plain pool.
    pub supply1: u64,
}

/// The fees the AMM itself charges on one swap. A hook's own fees are the records' fee splits
/// ([`SwapQuote::fee_split0`] / [`SwapQuote::fee_split1`]) and, in the hook's own terms, its quoter's.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct QuoteFees {
    /// LP fee, Σ of the trader walk's step fees, in the INPUT currency — the LPs' part only; the
    /// Carpenter Platform fee is [`Self::protocol`].
    pub lp: u64,
    /// The Carpenter Platform fee the TRADER's walk pays (on top of the LP fee), in [`Self::protocol_currency`] — off the input when it rides the input, off
    /// the output otherwise (so `amount_out` is already net of it). `Swapped.protocol_fee_total` on
    /// chain; 0 on a pool whose `protocol_fee` is 0. Like `lp`, the trader's walk only: a hook
    /// action's own walk pays too, visible in `pool_after.protocol_fees_owed0/1`, not here.
    pub protocol: u64,
    /// The currency (0 / 1) `protocol` is in (`Swapped.protocol_fee_currency`).
    pub protocol_currency: u8,
}

#[derive(Clone, Debug)]
pub struct SwapQuote {
    /// What the trader pays (`-in`).
    pub amount_in: u64,
    /// What the trader receives.
    pub amount_out: u64,
    pub fees: QuoteFees,
    /// The pool price after the whole swap, actions included.
    pub sqrt_after: u128,
    pub tick_after: i32,
    /// The price after the trader's walk, before any action.
    pub sqrt_after_trade: u128,
    pub tick_after_trade: i32,
    /// Arrays the trader's walk read (start indices).
    pub arrays_touched: Vec<i32>,
    /// The actions the hook requested and the AMM executed, every round in order.
    pub hook_actions: Vec<HookAction>,
    /// Arrays the actions read: each `ModifyPosition`'s ticks, each `Swap`'s walk.
    pub action_arrays: Vec<i32>,
    /// `after_swap`'s record's `signals` (0 when it did not run).
    pub signals: u8,
    /// Caller perspective, the walk's raw delta.
    pub swap_delta: BalanceDelta,
    /// The one hook delta credited to claims.
    pub hook_delta: BalanceDelta,
    /// `Swapped.fee_split0 / fee_split1`: the records' `fees` per currency, `[skim, creator, holder,
    /// platform, team]` (`FEE_*`), mapped exactly as `hook_delta` is (`HookDeltaAssembly::assemble_fees`
    /// — `before_swap`'s split on the specified currency, `after_swap`'s on the other). `[0; 5]` twice
    /// on a plain pool.
    pub fee_split0: FeeSplit,
    pub fee_split1: FeeSplit,
    /// Caller perspective, what settles.
    pub trader_delta: BalanceDelta,
    pub pool_after: Pool,
    /// Post-state of every array a walk crossed a tick in, or read.
    pub arrays_after: Vec<TickArray>,
    /// `SETTLES_FLAG` only: what the hook minted into `[vault0, vault1]` during the swap (credited to
    /// its claims). `[0, 0]` on every other pool.
    pub settled: [u64; 2],
}

impl SwapQuote {
    /// Every array the transaction must carry: the trader's walk and every action's, deduplicated.
    pub fn all_arrays(&self) -> Vec<i32> {
        let mut v = self.arrays_touched.clone();
        for s in &self.action_arrays {
            if !v.contains(s) {
                v.push(*s);
            }
        }
        v
    }
}

fn snapshot_tick_sqrt(pool: &Pool) -> (u128, i32) {
    (pool.sqrt_price_x64.get(), pool.tick_current)
}

fn expect(pool: &Pool, phase: u8, round: u8) -> RecordExpect {
    RecordExpect { version: HOOK_INTERFACE_VERSION, phase, round, flags: pool.hook_flags, max_rounds: pool.hook_max_rounds }
}

/// The one hook delta into claims. Non-negative components only (`HookDeltaNegative`) — unless the hook
/// settles, when a negative component is a payout from claims (`HookClaimsInsufficient` below zero).
/// `hook_fees` count only what the hook took.
fn credit_claims(pool: &mut Pool, delta: BalanceDelta, settles: bool) -> QuoteResult<()> {
    if !settles && (delta.amount0 < 0 || delta.amount1 < 0) {
        return Err(QuoteError::HookDeltaNegative);
    }
    pool.hook_claims0 = apply_signed(pool.hook_claims0, delta.amount0)?;
    pool.hook_claims1 = apply_signed(pool.hook_claims1, delta.amount1)?;
    pool.hook_fees0 = U128::new(pool.hook_fees0.get().checked_add(taken(delta.amount0)).ok_or(QuoteError::Overflow)?);
    pool.hook_fees1 = U128::new(pool.hook_fees1.get().checked_add(taken(delta.amount1)).ok_or(QuoteError::Overflow)?);
    Ok(())
}

/// `claim + delta`, refused below zero (`HookClaimsInsufficient`) or above `u64::MAX`.
fn apply_signed(claim: u64, delta: i128) -> QuoteResult<u64> {
    let next = (claim as i128).checked_add(delta).ok_or(QuoteError::Overflow)?;
    if next < 0 {
        return Err(QuoteError::HookClaimsInsufficient);
    }
    u64::try_from(next).map_err(|_| QuoteError::Overflow)
}

/// The positive part of a signed component (a payout is not a fee).
fn taken(component: i128) -> u128 {
    if component > 0 {
        component as u128
    } else {
        0
    }
}

/// `SETTLES_FLAG`: after a settling hook's callback, credit what it minted into the vaults to its
/// claims, and add it to the swap's running total.
fn credit_settled<H: HookQuoter + ?Sized>(h: &mut H, pool: &mut Pool, settled: &mut [u64; 2]) -> QuoteResult<()> {
    let minted = h.settled();
    pool.hook_claims0 = pool.hook_claims0.checked_add(minted[0]).ok_or(QuoteError::Overflow)?;
    pool.hook_claims1 = pool.hook_claims1.checked_add(minted[1]).ok_or(QuoteError::Overflow)?;
    settled[0] = settled[0].checked_add(minted[0]).ok_or(QuoteError::Overflow)?;
    settled[1] = settled[1].checked_add(minted[1]).ok_or(QuoteError::Overflow)?;
    Ok(())
}

/// `SETTLES_FLAG`: `after_swap`'s `delta_specified`, the part of an EXACT-IN the walk left unfilled.
/// Refused (`HookSettleOverfill`) when negative, when non-zero on an exact-out, or when larger than
/// `|amount_to_swap| − (input the walk consumed)`.
fn settle_specified(amount_specified: i128, to_swap: i128, zero_for_one: bool, swap_delta: &BalanceDelta, after_specified: i128) -> QuoteResult<i128> {
    if after_specified == 0 {
        return Ok(0);
    }
    if !(after_specified > 0 && amount_specified < 0) {
        return Err(QuoteError::HookSettleOverfill);
    }
    let consumed = if zero_for_one { swap_delta.amount0 } else { swap_delta.amount1 }.unsigned_abs();
    let unfilled = to_swap.unsigned_abs().checked_sub(consumed).ok_or(QuoteError::Overflow)?;
    if (after_specified as u128) > unfilled {
        return Err(QuoteError::HookSettleOverfill);
    }
    Ok(after_specified)
}

/// One round's executed actions, and whether every `ModifyPosition` result carries the position's
/// accrued fees (`ModifyOutcome::fees_replayed`; vacuously true with no `ModifyPosition`). The bool is
/// handed to `HookQuoter::after_actions` as `results_carry_fees`.
struct Executed {
    results: Vec<ActionResult>,
    results_carry_fees: bool,
}

/// Execute one record's actions in order, hooks skipped, each debit checked against claims at
/// the moment it happens. A `ModifyPosition` is replayed through `modify.rs` (an add debits claims,
/// a remove credits them; the position's accrued fees are not known to a quote, see there).
fn execute_actions(
    pool: &mut Pool,
    set: &mut TickArraySet,
    actions: &[HookAction],
    action_arrays: &mut Vec<i32>,
) -> QuoteResult<Executed> {
    let mut results = Vec::with_capacity(actions.len());
    let mut results_carry_fees = true;
    for (index, action) in actions.iter().enumerate() {
        match *action {
            HookAction::ModifyPosition { salt: _, tick_lower, tick_upper, liquidity_delta } => {
                let mut result = ActionResult { index: index as u8, kind: action.kind(), ..Default::default() };
                let m = modify::modify_position(pool, set, tick_lower, tick_upper, liquidity_delta, WalkKind::Action, action_arrays)?;
                results_carry_fees &= m.fees_replayed;
                let (paid0, paid1) = ((-m.delta.amount0).max(0), (-m.delta.amount1).max(0));
                let (got0, got1) = (m.delta.amount0.max(0), m.delta.amount1.max(0));
                let c0 = (pool.hook_claims0 as i128).checked_sub(paid0).ok_or(QuoteError::Overflow)?;
                let c1 = (pool.hook_claims1 as i128).checked_sub(paid1).ok_or(QuoteError::Overflow)?;
                if c0 < 0 || c1 < 0 {
                    return Err(QuoteError::HookClaimsInsufficient);
                }
                pool.hook_claims0 = u64::try_from(c0 + got0).map_err(|_| QuoteError::Overflow)?;
                pool.hook_claims1 = u64::try_from(c1 + got1).map_err(|_| QuoteError::Overflow)?;
                result.delta = m.delta;
                result.post_sqrt_price_x64 = pool.sqrt_price_x64.get();
                result.post_tick = pool.tick_current;
                results.push(result);
            }
            HookAction::Swap { zero_for_one, amount_specified, sqrt_price_limit_x64 } => {
                let mut result = ActionResult { index: index as u8, kind: action.kind(), ..Default::default() };
                if amount_specified != 0 {
                    if amount_specified.unsigned_abs() > u64::MAX as u128 {
                        return Err(QuoteError::AmountTooLarge);
                    }
                    walk::check_price_limit(pool, zero_for_one, sqrt_price_limit_x64)?;
                    let p = WalkParams { zero_for_one, amount_specified, sqrt_price_limit_x64, fee_pips: pool.lp_fee };
                    let out = walk::walk(pool, set, &p, WalkKind::Action)?;
                    for s in out.arrays_read {
                        if !action_arrays.contains(&s) {
                            action_arrays.push(s);
                        }
                    }
                    // Hook-claims perspective: the input is debited, the output credited.
                    let d = out.swap_delta;
                    let (paid0, paid1) = ((-d.amount0).max(0), (-d.amount1).max(0));
                    let (got0, got1) = (d.amount0.max(0), d.amount1.max(0));
                    let c0 = (pool.hook_claims0 as i128).checked_sub(paid0).ok_or(QuoteError::Overflow)?;
                    let c1 = (pool.hook_claims1 as i128).checked_sub(paid1).ok_or(QuoteError::Overflow)?;
                    if c0 < 0 || c1 < 0 {
                        return Err(QuoteError::HookClaimsInsufficient);
                    }
                    pool.hook_claims0 = u64::try_from(c0 + got0).map_err(|_| QuoteError::Overflow)?;
                    pool.hook_claims1 = u64::try_from(c1 + got1).map_err(|_| QuoteError::Overflow)?;
                    result.delta = d;
                    if zero_for_one {
                        result.fees0 = out.fee_total;
                    } else {
                        result.fees1 = out.fee_total;
                    }
                }
                result.post_sqrt_price_x64 = pool.sqrt_price_x64.get();
                result.post_tick = pool.tick_current;
                results.push(result);
            }
        }
    }
    Ok(Executed { results, results_carry_fees })
}

/// Whether a pool's hook takes part in a swap: a hook is set and it sets a swap flag. A hook with
/// neither `BEFORE_SWAP_FLAG` nor `AFTER_SWAP_FLAG` changes nothing a swap does.
pub fn is_swap_hooked(pool: &Pool) -> bool {
    pool.hook != [0u8; 32] && pool.hook_flags & (BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG) != 0
}

/// Quote a swap on a pool with no swap hook. A hooked pool is refused
/// ([`QuoteError::QuoteHookNotQuotable`]).
pub fn quote_plain(pool: &PoolAccount, arrays: &TickArraySet, params: &SwapQuoteParams) -> QuoteResult<SwapQuote> {
    quote_swap(pool, arrays, None, params)
}

/// Quote a swap, picking the quoter that [accepts](HookQuoter::accepts) the pool's hook out of
/// `quoters` (the first that matches). A pool with no swap hook ignores them; a hooked pool whose hook
/// has no quoter here is refused ([`QuoteError::QuoteHookNotQuotable`]).
pub fn quote_swap_with(
    pool: &PoolAccount,
    arrays: &TickArraySet,
    quoters: &mut [&mut dyn HookQuoter],
    params: &SwapQuoteParams,
) -> QuoteResult<SwapQuote> {
    let id = pool.pool.hook;
    let hook = quoters.iter_mut().find(|q| q.accepts(&id)).map(|q| &mut **q);
    quote_with_hook(pool, arrays, hook, params)
}

/// Quote one swap against a pool's account state. See the module docs for the order of effects.
///
/// `hook` is consulted exactly when the pool's hook sets a swap flag ([`is_swap_hooked`]): it must
/// then be the quoter for that hook program ([`HookQuoter::accepts`] true for `pool.hook`), or the
/// quote is refused with [`QuoteError::QuoteHookNotQuotable`] — an unknown hook is never quoted as a
/// plain pool. On a pool with no swap hook, `hook` is not called.
pub fn quote_swap(
    pool: &PoolAccount,
    arrays: &TickArraySet,
    hook: Option<&mut dyn HookQuoter>,
    params: &SwapQuoteParams,
) -> QuoteResult<SwapQuote> {
    quote_with_hook(pool, arrays, hook, params)
}

fn quote_with_hook<H: HookQuoter + ?Sized>(
    pool: &PoolAccount,
    arrays: &TickArraySet,
    mut hook: Option<&mut H>,
    params: &SwapQuoteParams,
) -> QuoteResult<SwapQuote> {
    let mut p = pool.pool;
    let mut set = arrays.clone();
    if set.tick_spacing != p.tick_spacing {
        return Err(QuoteError::QuoteBadTickArray);
    }
    // The program's swap refuses a paused pool right after its hook-data length check, ahead of every
    // callback and of the amount checks: `PoolPaused` outranks a hook's state and `AmountZero`. Only the
    // authority's flag is judged; a guardian pause is a clock window (`QuoteError::PoolPaused`).
    if p.paused != 0 {
        return Err(QuoteError::PoolPaused);
    }
    let flags = p.hook_flags;
    let mut hq: Option<&mut H> = if is_swap_hooked(&p) {
        match hook.take() {
            Some(h) if h.accepts(&p.hook) => {
                h.prepare(pool)?;
                Some(h)
            }
            _ => return Err(QuoteError::QuoteHookNotQuotable),
        }
    } else {
        None
    };
    let settles = hq.is_some() && flags & SETTLES_FLAG != 0;
    let mut settled: [u64; 2] = [0, 0];

    // ---- prelude --------------------------------------------------------------------------------
    let zfo = params.zero_for_one;
    let amount_specified = params.amount_specified;
    if amount_specified == 0 {
        return Err(QuoteError::AmountZero);
    }
    if amount_specified.unsigned_abs() > u64::MAX as u128 {
        return Err(QuoteError::AmountTooLarge);
    }
    let (_, pre_tick) = snapshot_tick_sqrt(&p);
    let ctx = HookSwapCtx {
        pool_key: pool.key,
        zero_for_one: zfo,
        amount_specified,
        pre_tick,
        supply0: params.supply0,
        supply1: params.supply1,
        claims0: p.hook_claims0,
        claims1: p.hook_claims1,
        mint0: p.mint0,
        mint1: p.mint1,
        tick_spacing: p.tick_spacing,
        fee: p.fee,
    };

    // ---- R0 -------------------------------------------------------------------------------------
    let mut spec: i128 = 0;
    let mut unspec: i128 = 0;
    let mut fee_override: Option<u32> = None;
    let mut fees_before: FeeSplit = [0; 5];
    let mut hook_limit: u128 = 0;
    if let Some(h) = hq.as_deref_mut() {
        if flags & BEFORE_SWAP_FLAG != 0 {
            let rec0 = h.before_swap(&ctx)?;
            hk::validate_record(&rec0, &expect(&p, PHASE_BEFORE_SWAP, 0))?;
            if settles {
                credit_settled(h, &mut p, &mut settled)?;
            }
            hook_limit = rec0.sqrt_price_limit_x64;
            spec = rec0.delta_specified;
            unspec = rec0.delta_unspecified;
            fee_override = hk::effective_fee_override(&rec0, flags, p.fee == DYNAMIC_FEE_FLAG);
            fees_before = rec0.fees();
        }
    }

    let amount_to_swap = hk::apply_specified_delta(amount_specified, spec)?;
    // A settling hook's negative specified delta may not enlarge the walk past what the trader
    // authorised; a non-settling one is refused later (`HookDeltaNegative`).
    if settles && spec < 0 && amount_to_swap.unsigned_abs() > amount_specified.unsigned_abs() {
        return Err(QuoteError::HookDeltaExceedsSwapAmount);
    }
    let fee = fee_override.unwrap_or(p.lp_fee);
    // the exact-out fee check on the rate the steps are charged — the COMPOSED rate when the Carpenter Platform fee
    // rides the input (the program's walk checks `walk_fee(..).swap_fee` the same way).
    let walk_fee = walk::fee_for(&p, zfo, fee);
    if walk_fee.swap_fee >= MAX_SWAP_FEE && amount_to_swap > 0 {
        return Err(QuoteError::InvalidFeeForExactOut);
    }

    // ---- the trader's walk ----------------------------------------------------------------------
    // A settling hook's `before_swap` limit: the walk runs to the tighter of it and the trader's own; a
    // hook limit the price already sits at (or past) skips the walk, and the hook fills the swap.
    let mut walk_limit = params.sqrt_price_limit_x64;
    let mut skip_walk = false;
    if hook_limit != 0 {
        let price = p.sqrt_price_x64.get();
        if zfo {
            if hook_limit >= price {
                skip_walk = true;
            } else {
                walk_limit = walk_limit.max(hook_limit);
            }
        } else if hook_limit <= price {
            skip_walk = true;
        } else {
            walk_limit = walk_limit.min(hook_limit);
        }
    }
    let (swap_delta, lp_fee_total, protocol_fee_total, arrays_touched) = if amount_to_swap == 0 || skip_walk {
        (BalanceDelta::default(), 0u64, 0u64, Vec::new()) // Pool.sol:320, before the limit checks
    } else {
        walk::check_price_limit(&p, zfo, walk_limit)?;
        let wp = WalkParams { zero_for_one: zfo, amount_specified: amount_to_swap, sqrt_price_limit_x64: walk_limit, fee_pips: fee };
        let out = walk::walk(&mut p, &mut set, &wp, WalkKind::Trader)?;
        (out.swap_delta, out.fee_total, out.protocol_fee, out.arrays_read)
    };
    let (sqrt_after_trade, tick_after_trade) = snapshot_tick_sqrt(&p);

    // ---- R1 -------------------------------------------------------------------------------------
    let mut rec1: Option<HookRecordV1> = None;
    if let Some(h) = hq.as_deref_mut() {
        if flags & AFTER_SWAP_FLAG != 0 {
            let a = AfterSwapIn {
                post_sqrt_price_x64: sqrt_after_trade,
                post_tick: tick_after_trade,
                swap_delta,
                before_delta_specified: spec,
                before_delta_unspecified: unspec,
            };
            let r = h.after_swap(&ctx, &a)?;
            hk::validate_record(&r, &expect(&p, PHASE_AFTER_SWAP, 1))?;
            if settles {
                credit_settled(h, &mut p, &mut settled)?;
                let fill = settle_specified(amount_specified, amount_to_swap, zfo, &swap_delta, r.delta_specified)?;
                spec = spec.checked_add(fill).ok_or(QuoteError::Overflow)?;
            }
            unspec = unspec.checked_add(r.delta_unspecified).ok_or(QuoteError::Overflow)?;
            rec1 = Some(r);
        }
    }

    // ---- the ONE credit of hook deltas ----------------------------------------------------------
    let hook_delta = HookDeltaAssembly::assemble(amount_specified, zfo, spec, unspec);
    // `fee_split = assemble_fees(...)`: the same mapping, the records' splits; a round that did not
    // run contributes zeros.
    let fees_after = rec1.as_ref().map(HookRecordV1::fees).unwrap_or_default();
    let (fee_split0, fee_split1) = HookDeltaAssembly::assemble_fees(amount_specified, zfo, fees_before, fees_after);
    let trader_delta = BalanceDelta {
        amount0: swap_delta.amount0.checked_sub(hook_delta.amount0).ok_or(QuoteError::Overflow)?,
        amount1: swap_delta.amount1.checked_sub(hook_delta.amount1).ok_or(QuoteError::Overflow)?,
    };
    credit_claims(&mut p, hook_delta, settles)?;

    // ---- actions and after_actions rounds ------------------------------------------------------
    let mut hook_actions = Vec::new();
    let mut action_arrays = Vec::new();
    let mut signals = 0u8;
    if let Some(r1) = rec1 {
        signals = r1.signals;
        hook_actions.extend_from_slice(&r1.actions);
        let mut done = execute_actions(&mut p, &mut set, &r1.actions, &mut action_arrays)?;
        let mut rec = r1;
        let mut round: u8 = 1;
        while rec.want_callback {
            if u16::from(round) + 1 >= u16::from(p.hook_max_rounds) {
                return Err(QuoteError::HookRoundsExceeded);
            }
            round += 1;
            let h = hq.as_deref_mut().expect("rounds only run on a hooked pool");
            let next = h.after_actions(&ctx, round, &done.results, done.results_carry_fees, p.hook_claims0, p.hook_claims1)?;
            hk::validate_record(&next, &expect(&p, PHASE_AFTER_ACTIONS, round))?;
            if settles {
                credit_settled(h, &mut p, &mut settled)?;
            }
            hook_actions.extend_from_slice(&next.actions);
            done = execute_actions(&mut p, &mut set, &next.actions, &mut action_arrays)?;
            rec = next;
        }
    }

    // ---- settlement checks and counters --------------------------------------------------------
    let (inp, outp) = if zfo { (trader_delta.amount0, trader_delta.amount1) } else { (trader_delta.amount1, trader_delta.amount0) };
    if !(inp <= 0 && outp >= 0) {
        return Err(QuoteError::HookDeltaExceedsSwapOutput);
    }
    let amount_in = u64::try_from(-inp).map_err(|_| QuoteError::AmountOverflow)?;
    let amount_out = u64::try_from(outp).map_err(|_| QuoteError::AmountOverflow)?;

    if zfo {
        p.volume0 = U128::new(p.volume0.get().checked_add(amount_in as u128).ok_or(QuoteError::Overflow)?);
        p.lp_fees0 = U128::new(p.lp_fees0.get().checked_add(lp_fee_total as u128).ok_or(QuoteError::Overflow)?);
    } else {
        p.volume1 = U128::new(p.volume1.get().checked_add(amount_in as u128).ok_or(QuoteError::Overflow)?);
        p.lp_fees1 = U128::new(p.lp_fees1.get().checked_add(lp_fee_total as u128).ok_or(QuoteError::Overflow)?);
    }
    p.swap_count = p.swap_count.checked_add(1).ok_or(QuoteError::Overflow)?;

    let mut read: Vec<i32> = arrays_touched.clone();
    for s in &action_arrays {
        if !read.contains(s) {
            read.push(*s);
        }
    }
    let arrays_after = read.iter().filter_map(|s| set.arrays.get(s).copied()).collect();

    Ok(SwapQuote {
        amount_in,
        amount_out,
        fees: QuoteFees { lp: lp_fee_total, protocol: protocol_fee_total, protocol_currency: walk_fee.currency },
        sqrt_after: p.sqrt_price_x64.get(),
        tick_after: p.tick_current,
        sqrt_after_trade,
        tick_after_trade,
        arrays_touched,
        hook_actions,
        action_arrays,
        signals,
        swap_delta,
        hook_delta,
        fee_split0,
        fee_split1,
        trader_delta,
        pool_after: p,
        arrays_after,
        settled,
    })
}
