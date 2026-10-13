//! Synthetic pool states, built from a list of positions with the same math the AMM uses.
//!
//! For tests, golden vectors and app mocks. A parity harness can write these bytes into an SVM and
//! swap against them, so a pool here and a pool the program built must be the SAME bytes for the
//! fields a swap reads. What this sets, v4 `Pool.modifyLiquidity` on a
//! fresh pool would set: ticks' `liquidity_gross` / `liquidity_net`, `fee_growth_outside` = 0 (every
//! global is 0 at creation, and v4 initialises outside to the global only for ticks at or below the
//! current tick, which is 0 either way), `initialized_tick_count`, the bitmap bit of every array with
//! an initialized tick, and the active `liquidity`.
//! It does NOT write `Position` accounts: a swap never reads them.

use std::collections::BTreeMap;

use carpenter_clmm_math::{bitmap, tick_math};
use carpenter_types::constants::{MAX_TICK, MIN_TICK, TICK_ARRAY_SIZE};
use carpenter_types::layout::{Pool, TickArray};
use carpenter_types::{I128, U128};

use crate::error::{QuoteError, QuoteResult};
use crate::liquidity::amounts_for_liquidity;
use crate::state::TickArraySet;
use crate::swap::PoolAccount;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SynthPosition {
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub liquidity: u128,
}

/// The keys a pool account stores.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolKeys {
    pub pool: [u8; 32],
    pub mint0: [u8; 32],
    pub mint1: [u8; 32],
    pub vault0: [u8; 32],
    pub vault1: [u8; 32],
    pub token_program0: [u8; 32],
    pub token_program1: [u8; 32],
    /// `Pubkey::default()` for a plain pool.
    pub hook: [u8; 32],
    pub hook_authority: [u8; 32],
}

#[derive(Clone, Debug)]
pub struct SynthPool {
    pub account: PoolAccount,
    pub arrays: Vec<TickArray>,
    pub words: Vec<u64>,
    /// What the positions hold at the current price, rounded up (what their adds charged): a vault
    /// holding at least this plus claims is solvent (I-9).
    pub principal0: u64,
    pub principal1: u64,
}

impl SynthPool {
    pub fn tick_array_set(&self) -> QuoteResult<TickArraySet> {
        TickArraySet::new(&self.account.key, self.account.pool.tick_spacing, self.arrays.clone(), Some(self.words.clone()))
    }

    /// The Carpenter Platform fee: the two fields `initialize_pool` writes from the config's current
    /// setting and the mints' roles — the v4-packed rate and the fee-currency code
    /// (`carpenter_types::protocol_fee::currency_for_roles`). [`build_pool`] leaves both 0 (no protocol
    /// fee, the input rule); a harness that mirrors a pool with a fee sets them here, so the synthetic
    /// bytes stay the bytes the program writes.
    pub fn with_protocol_fee(mut self, packed: u32, protocol_fee_currency: u8) -> Self {
        self.account.pool.protocol_fee = packed;
        self.account.pool.protocol_fee_currency = protocol_fee_currency;
        self
    }
}

/// How the pool's fee and hook are set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SynthKind {
    /// A hookless pool with a static fee (pips, ≤ 1,000,000).
    Plain { fee: u32 },
    /// A hooked pool: the key fee (`DYNAMIC_FEE_FLAG` for a dynamic-fee pool), the stored `lp_fee`, and
    /// the hook's flags, round cap and callback-account count as `initialize_pool` copies them from
    /// the hook's approval. A hook's crate exports its own value of this (FLOOR's is
    /// `floor_quote::synth::FLOOR`). Pass the hook's program id in [`PoolKeys::hook`].
    Hooked { fee: u32, lp_fee: u32, flags: u16, max_rounds: u8, callback_accounts: u8 },
}

#[allow(clippy::too_many_arguments)]
pub fn build_pool(
    keys: &PoolKeys,
    kind: SynthKind,
    tick_spacing: u16,
    sqrt_price_x64: u128,
    decimals0: u8,
    decimals1: u8,
    positions: &[SynthPosition],
) -> QuoteResult<SynthPool> {
    let s = i32::from(tick_spacing);
    if s == 0 {
        return Err(QuoteError::Math(carpenter_clmm_math::MathError::InvalidTickSpacing));
    }
    let tick_current = tick_math::get_tick_at_sqrt_price(sqrt_price_x64)?;

    let mut arrays: BTreeMap<i32, TickArray> = BTreeMap::new();
    let mut liquidity: u128 = 0;
    let (mut principal0, mut principal1) = (0u64, 0u64);

    for pos in positions {
        let (lo, up) = (pos.tick_lower, pos.tick_upper);
        if lo >= up || lo % s != 0 || up % s != 0 || lo < MIN_TICK || up > MAX_TICK || pos.liquidity == 0 {
            return Err(QuoteError::QuoteBadTickArray);
        }
        let l = i128::try_from(pos.liquidity).map_err(|_| QuoteError::Overflow)?;
        bump_tick(&mut arrays, keys.pool, tick_spacing, lo, pos.liquidity, l)?;
        bump_tick(&mut arrays, keys.pool, tick_spacing, up, pos.liquidity, -l)?;
        if tick_current >= lo && tick_current < up {
            liquidity = liquidity.checked_add(pos.liquidity).ok_or(QuoteError::Overflow)?;
        }
        let (a0, a1) = amounts_for_liquidity(sqrt_price_x64, tick_current, lo, up, l)?;
        principal0 = principal0.checked_add(a0).ok_or(QuoteError::Overflow)?;
        principal1 = principal1.checked_add(a1).ok_or(QuoteError::Overflow)?;
    }

    let mut words = vec![0u64; bitmap::word_count(tick_spacing)?];
    for (start, a) in &arrays {
        if a.initialized_tick_count > 0 {
            bitmap::set_initialized(&mut words, bitmap::array_bit(*start, tick_spacing)?, true)?;
        }
    }

    let mut pool: Pool = bytemuck::Zeroable::zeroed();
    pool.version = 1;
    pool.tick_spacing = tick_spacing;
    match kind {
        SynthKind::Plain { fee } => {
            pool.fee = fee;
            pool.lp_fee = fee;
        }
        SynthKind::Hooked { fee, lp_fee, flags, max_rounds, callback_accounts } => {
            pool.fee = fee;
            pool.lp_fee = lp_fee;
            pool.hook_flags = flags;
            pool.hook_max_rounds = max_rounds;
            pool.hook_callback_accounts = callback_accounts;
        }
    }
    pool.tick_current = tick_current;
    pool.mint0 = keys.mint0;
    pool.mint1 = keys.mint1;
    pool.hook = keys.hook;
    pool.hook_authority = keys.hook_authority;
    pool.vault0 = keys.vault0;
    pool.vault1 = keys.vault1;
    pool.token_program0 = keys.token_program0;
    pool.token_program1 = keys.token_program1;
    pool.sqrt_price_x64 = U128::new(sqrt_price_x64);
    pool.liquidity = U128::new(liquidity);
    pool.decimals0 = decimals0;
    pool.decimals1 = decimals1;

    Ok(SynthPool {
        account: PoolAccount { key: keys.pool, pool },
        arrays: arrays.into_values().collect(),
        words,
        principal0,
        principal1,
    })
}

fn bump_tick(
    arrays: &mut BTreeMap<i32, TickArray>,
    pool: [u8; 32],
    spacing: u16,
    tick: i32,
    gross: u128,
    net: i128,
) -> QuoteResult<()> {
    let start = bitmap::array_start_index(tick, spacing)?;
    let array = arrays.entry(start).or_insert_with(|| {
        let mut a: TickArray = bytemuck::Zeroable::zeroed();
        a.pool = pool;
        a.start_index = start;
        a
    });
    let i = (tick - start) / i32::from(spacing);
    debug_assert!((0..TICK_ARRAY_SIZE).contains(&i));
    let t = &mut array.ticks[i as usize];
    let was = t.liquidity_gross.get();
    let now = was.checked_add(gross).ok_or(QuoteError::Overflow)?;
    t.liquidity_gross = U128::new(now);
    t.liquidity_net = I128::new(t.liquidity_net.get().checked_add(net).ok_or(QuoteError::Overflow)?);
    if was == 0 {
        array.initialized_tick_count += 1;
    }
    Ok(())
}
