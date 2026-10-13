//! Account resolution: the PDAs a swap names, the tick-array window `update` reads, and the `swap`
//! instruction's accounts in on-chain order.

use std::collections::BTreeSet;

use carpenter_quote::clmm::bitmap;
use carpenter_quote::QuoteError;
use carpenter_types::constants::{MAX_TICK, MIN_TICK};
use carpenter_types::hook::HookAction;
use carpenter_types::layout::Pool;
use floor_types::layout::FloorPool;
use floor_types::seeds;
use solana_instruction::AccountMeta;
use solana_pubkey::Pubkey;

use crate::convert::pk;
use crate::hook::KnownHook;
use crate::{FIXED_METAS, MAX_ARRAYS, TRADER_ARRAYS_PER_SIDE};

/// `["bitmap", pool]` under `program` (the pool's `carpenter_amm`).
pub fn bitmap_pda(program: &Pubkey, pool: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::BITMAP, pool.as_ref()], program).0
}

/// `["tick_array", pool, start_index i32 LE]` under `program` (the pool's `carpenter_amm`).
pub fn tick_array_pda(program: &Pubkey, pool: &Pubkey, start_index: i32) -> Pubkey {
    Pubkey::find_program_address(&[seeds::TICK_ARRAY, pool.as_ref(), &seeds::tick_array_seed(start_index)], program).0
}

/// `["position", pool, owner, salt u64 LE]` under `program` (the pool's `carpenter_amm`). A hook's
/// position is owned by the pool's `hook_authority`; a `ModifyPosition` action finds it by this key
/// among the swap's remaining accounts and fails the swap (`HookActionAccountMissing`) when it is
/// absent or read-only.
pub fn position_pda(program: &Pubkey, pool: &Pubkey, owner: &Pubkey, salt: u64) -> Pubkey {
    Pubkey::find_program_address(&[carpenter_types::seeds::POSITION, pool.as_ref(), owner.as_ref(), &salt.to_le_bytes()], program).0
}

/// The positions a quote's hook actions modify, in first-use order, without repeats: the
/// `hook_authority`'s position for each distinct `ModifyPosition` salt, under `program`.
pub fn action_positions(program: &Pubkey, pool_key: &Pubkey, pool: &Pool, actions: &[HookAction]) -> Vec<Pubkey> {
    let owner = pk(&pool.hook_authority);
    let mut salts: Vec<u64> = Vec::new();
    for a in actions {
        if let HookAction::ModifyPosition { salt, .. } = a {
            if !salts.contains(salt) {
                salts.push(*salt);
            }
        }
    }
    salts.into_iter().map(|salt| position_pda(program, pool_key, &owner, salt)).collect()
}

/// `["__event_authority"]` under `program` (Anchor's event-CPI signer; `carpenter_amm`'s, or a
/// hook's).
pub fn event_authority_pda(program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::EVENT_AUTHORITY], program).0
}

/// `["trader_ref", trader, quote_mint]` under `hook_program` (`floor_hook`): the second account of
/// FLOOR's hook slice. It is derived from the signer and needs no fetch; for a trader with no
/// referral link the account does not exist and the hook books nothing to it.
pub fn trader_bucket_pda(hook_program: &Pubkey, trader: &Pubkey, quote_mint: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::TRADER_REF, trader.as_ref(), quote_mint.as_ref()], hook_program).0
}

/// `["__event_authority"]` under `hook_program` (`floor_hook`): the third account of FLOOR's hook
/// slice, the signer of the hook callbacks' `emit_cpi!` events. The fourth is the hook program itself.
pub fn hook_event_authority_pda(hook_program: &Pubkey) -> Pubkey {
    event_authority_pda(hook_program)
}

/// The tick arrays `update` reads. `get_accounts_to_update` has no amount, so this is a superset;
/// `get_swap_and_account_metas` then carries exactly the arrays the quote for the actual amount read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Window {
    /// Start indices, ascending, each one marked in the bitmap. At most [`MAX_ARRAYS`].
    pub starts: Vec<i32>,
    /// FLOOR's own position span needs more than [`MAX_ARRAYS`] arrays: the pool is reported
    /// inactive.
    pub floor_span_too_wide: bool,
}

/// The window for a pool, from its bitmap words and, on a FLOOR pool, the hook state:
///
/// 1. If the bitmap marks at most [`MAX_ARRAYS`] arrays, the window is all of them. No walk reads an
///    unmarked array, so this covers every amount.
/// 2. Otherwise it is every marked array inside FLOOR's position span (the floor, the bid, the seed
///    and the ladder edges, and the current tick), plus the trader's arrays: the current one and the
///    next [`TRADER_ARRAYS_PER_SIDE`] marked arrays in each direction, admitted nearest first by tick
///    distance across both sides (a tie to the lower start) until the window is full. If the span
///    alone exceeds the cap, `floor_span_too_wide` is set.
///
/// A swap whose walk leaves the window is refused by the quoter by name (`TickArrayMissing` /
/// `HookActionAccountMissing`); with the bitmap supplied a missing array is never read as empty.
pub fn window(pool: &Pool, words: &[u64], floor: Option<&FloorPool>) -> Result<Window, QuoteError> {
    window_with(pool, words, floor, MAX_ARRAYS, TRADER_ARRAYS_PER_SIDE)
}

/// [`window`] with the cap and the per-side count as arguments, so the admission order can be tested
/// at settings where the cap binds.
pub fn window_with(pool: &Pool, words: &[u64], floor: Option<&FloorPool>, max_arrays: usize, per_side: usize) -> Result<Window, QuoteError> {
    let spacing = pool.tick_spacing;
    bitmap::check_words(words, spacing)?;
    let marked_total: usize = words.iter().map(|w| w.count_ones() as usize).sum();
    if marked_total <= max_arrays {
        let mut starts = Vec::with_capacity(marked_total);
        for (wi, w) in words.iter().enumerate() {
            let mut bits = *w;
            while bits != 0 {
                let b = bits.trailing_zeros();
                bits &= bits - 1;
                starts.push(bitmap::start_index_of_bit(wi as u32 * 64 + b, spacing)?);
            }
        }
        starts.sort_unstable();
        return Ok(Window { starts, floor_span_too_wide: false });
    }

    let cur = bitmap::array_start_index(pool.tick_current.clamp(MIN_TICK, MAX_TICK), spacing)?;
    let marked = |start: i32| -> Result<bool, QuoteError> { Ok(bitmap::is_initialized(words, bitmap::array_bit(start, spacing)?)?) };

    let mut trader: Vec<i32> = Vec::new();
    if marked(cur)? {
        trader.push(cur);
    }
    let mut sides: Vec<i32> = Vec::with_capacity(2 * per_side);
    for zfo in [true, false] {
        let mut last = cur;
        for _ in 0..per_side {
            match bitmap::next_initialized_array_start_index(words, spacing, last, zfo)? {
                Some(next) => {
                    sides.push(next);
                    last = next;
                }
                None => break,
            }
        }
    }
    sides.sort_by_key(|s| ((i64::from(*s) - i64::from(cur)).abs(), *s));
    trader.extend(sides);

    let mut span: BTreeSet<i32> = BTreeSet::new();
    if let Some(fp) = floor {
        let s = fp.tick_spacing;
        let mut lo = fp.floor_tick.min(pool.tick_current);
        let mut hi = fp.floor_tick.saturating_add(s).max(pool.tick_current);
        let mut widen = |a: i32, b: i32| {
            lo = lo.min(a);
            hi = hi.max(b);
        };
        if fp.bid_liquidity.get() != 0 {
            widen(fp.bid_lower, fp.bid_lower.saturating_add(s));
        }
        if fp.seed_liq.get() != 0 {
            widen(fp.seed_lower, fp.seed_upper);
        }
        if fp.ladder_liq.get() != 0 {
            widen(fp.ladder_lower, fp.ladder_upper);
        }
        let lo_start = bitmap::array_start_index(lo.clamp(MIN_TICK, MAX_TICK), spacing)?;
        let hi_start = bitmap::array_start_index(hi.clamp(MIN_TICK, MAX_TICK), spacing)?;
        if marked(lo_start)? {
            span.insert(lo_start);
        }
        let mut last = lo_start;
        while let Some(next) = bitmap::next_initialized_array_start_index(words, spacing, last, false)? {
            if next > hi_start {
                break;
            }
            span.insert(next);
            last = next;
        }
    }
    if span.len() > max_arrays {
        return Ok(Window { starts: span.into_iter().collect(), floor_span_too_wide: true });
    }

    let mut chosen: BTreeSet<i32> = span;
    for start in trader {
        if chosen.len() >= max_arrays {
            break;
        }
        chosen.insert(start);
    }
    Ok(Window { starts: chosen.into_iter().collect(), floor_span_too_wide: false })
}

/// The `swap` instruction's accounts, in the program's order:
///
/// | # | account | flags |
/// |---|---|---|
/// | 1 | `trader` | signer |
/// | 2 | `pool` | writable |
/// | 3 | `bitmap` | writable (a hook's `ModifyPosition` action can flip a word) |
/// | 4–5 | `trader_token_in`, `trader_token_out` | writable |
/// | 6–7 | `vault0`, `vault1` | writable |
/// | 8–9 | `mint0`, `mint1` | read-only |
/// | 10–11 | `token_program0`, `token_program1` | read-only |
/// | 12 | `hook_program`: `pool.hook` when a callback runs, else `carpenter_amm` itself (Anchor's encoding of an absent optional account) | read-only |
/// | 13 | `event_authority` | read-only |
/// | 14 | `program` (`carpenter_amm`, the `program` argument) | read-only |
/// | 15 | FLOOR hook slice: `floor_pool` | writable |
/// | 16 | FLOOR hook slice: `trader_bucket` = [`trader_bucket_pda`]`(pool.hook, trader, mint0)` | writable |
/// | 17 | FLOOR hook slice: `floor_hook`'s event authority = [`hook_event_authority_pda`]`(pool.hook)` | read-only |
/// | 18 | FLOOR hook slice: `floor_hook` (the program; already key 12) | read-only |
/// | 19.. | tick arrays | writable |
/// | then | the positions the hook's `ModifyPosition` actions modify ([`action_positions`]) | writable |
///
/// On a FLOOR pool the quote currency is `mint0`, so the bucket is keyed by `mint0`.
#[allow(clippy::too_many_arguments)]
pub fn swap_metas(
    program: &Pubkey,
    pool_key: &Pubkey,
    pool: &Pool,
    bitmap: &Pubkey,
    event_authority: &Pubkey,
    hook: &KnownHook,
    trader: &Pubkey,
    trader_token_in: &Pubkey,
    trader_token_out: &Pubkey,
    tick_arrays: &[Pubkey],
    positions: &[Pubkey],
) -> Vec<AccountMeta> {
    let program = *program;
    let mut metas = vec![
        AccountMeta::new_readonly(*trader, true),
        AccountMeta::new(*pool_key, false),
        AccountMeta::new(*bitmap, false),
        AccountMeta::new(*trader_token_in, false),
        AccountMeta::new(*trader_token_out, false),
        AccountMeta::new(pk(&pool.vault0), false),
        AccountMeta::new(pk(&pool.vault1), false),
        AccountMeta::new_readonly(pk(&pool.mint0), false),
        AccountMeta::new_readonly(pk(&pool.mint1), false),
        AccountMeta::new_readonly(pk(&pool.token_program0), false),
        AccountMeta::new_readonly(pk(&pool.token_program1), false),
        AccountMeta::new_readonly(if hook.calls_back() { pk(&pool.hook) } else { program }, false),
        AccountMeta::new_readonly(*event_authority, false),
        AccountMeta::new_readonly(program, false),
    ];
    debug_assert_eq!(metas.len(), FIXED_METAS);
    if let KnownHook::Floor { floor_pool, .. } = hook {
        metas.push(AccountMeta::new(*floor_pool, false));
        let hook_program = pk(&pool.hook);
        metas.push(AccountMeta::new(trader_bucket_pda(&hook_program, trader, &pk(&pool.mint0)), false));
        // The callbacks' `#[event_cpi]` pair: one new key (the PDA); the program is already key 12.
        metas.push(AccountMeta::new_readonly(hook_event_authority_pda(&hook_program), false));
        metas.push(AccountMeta::new_readonly(pk(&pool.hook), false));
    }
    metas.extend(tick_arrays.iter().map(|k| AccountMeta::new(*k, false)));
    metas.extend(positions.iter().map(|k| AccountMeta::new(*k, false)));
    metas
}
