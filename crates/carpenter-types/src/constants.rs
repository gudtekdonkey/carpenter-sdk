//! The AMM's constants: what the program, the quoter and the SDK share about the Carpenter AMM
//! itself and the hook interface's flag bits. Each names, where one exists, its Uniswap v4 twin.
//!
//! Nothing of the FLOOR mechanism is here — `SUPPLY`, the salts, the skim ramp, the launch enums
//! and every other mechanism number are `floor_types::constants`, which re-exports this module
//! beside them. The AMM program links only this crate, so a mechanism constant cannot leak into it
//! by a glob import.
//! Every scalar here is a plain integer LITERAL or a `1 << n`, so tools may read this file as text.

// =========================================================================================
// AMM
// =========================================================================================

/// Raydium `tick_math.rs:12-19`.
pub const MIN_TICK: i32 = -443_636;
/// Raydium `tick_math.rs:12-19`.
pub const MAX_TICK: i32 = 443_636;
/// Q64.64.
pub const MIN_SQRT_PRICE_X64: u128 = 4_295_048_016;
/// Q64.64.
pub const MAX_SQRT_PRICE_X64: u128 = 79_226_673_521_066_979_257_578_248_091;

/// Ticks per fixed tick array.
pub const TICK_ARRAY_SIZE: i32 = 60;
/// v4 `PoolManager.sol:119-120`.
pub const MIN_TICK_SPACING: u16 = 1;
/// v4 `PoolManager.sol:119-120`.
pub const MAX_TICK_SPACING: u16 = 32_767;

/// v4 `LPFeeLibrary.DYNAMIC_FEE_FLAG`: a pool key fee of exactly this value is dynamic.
pub const DYNAMIC_FEE_FLAG: u32 = 0x80_0000;
/// v4 `LPFeeLibrary.MAX_LP_FEE`, in pips.
pub const MAX_LP_FEE: u32 = 1_000_000;
/// v4 `Pool.sol:65` `SwapMath.MAX_SWAP_FEE`. `swap_fee >= this && exact_output` →
/// `InvalidFeeForExactOut`.
pub const MAX_SWAP_FEE: u32 = 1_000_000;
/// `donate` requires `liquidity >= MIN_DONATE_LIQUIDITY` (a named v4 deviation).
pub const MIN_DONATE_LIQUIDITY: u128 = 1_000_000;
/// `donate` requires `amount <= DONATE_MAX_AMOUNT`.
pub const DONATE_MAX_AMOUNT: u64 = u64::MAX / 2;

/// Kept for API compatibility; the quote roster (`QuoteMint`) is unbounded.
pub const MAX_QUOTE_MINTS: usize = 2;

// =========================================================================================
// The Carpenter Platform fee — v4's PROTOCOL fee
// =========================================================================================
//
// A CURRENT SETTING on `AmmConfig` (governance-set, capped), COPIED into `Pool.protocol_fee` at
// `initialize_pool`, never written into a live pool. It applies to every pool, FLOOR pools included.
// The pure arithmetic over these is `crate::protocol_fee`.

/// The default Carpenter Platform fee, in pips
/// PER DIRECTION (500 = 0.05 %), on top of the pool's LP fee. What `AmmConfig` answers while its
/// setting was never written (`crate::protocol_fee::current_setting`).
pub const PROTOCOL_FEE_DEFAULT: u32 = 500;
/// The most governance may
/// set, pips per direction (0.1 %). Uniswap v4's own `ProtocolFeeLibrary.MAX_PROTOCOL_FEE`.
/// `set_protocol_fee` refuses above it (`ProtocolFeeTooLarge`); 0 is allowed.
pub const MAX_PROTOCOL_FEE: u32 = 1_000;
/// [`PROTOCOL_FEE_DEFAULT`] in both directions, v4-packed (`zero_for_one | one_for_zero << 12`):
/// `500 | 500 << 12`. A literal so this file stays readable as text; the const block below derives it and fails the build if the two ever disagree.
pub const PROTOCOL_FEE_DEFAULT_PACKED: u32 = 2_048_500;
/// v4 `ProtocolFeeLibrary.PIPS_DENOMINATOR`: fees are pips of 1,000,000.
pub const PIPS_DENOMINATOR: u32 = 1_000_000;
/// `Pool.protocol_fee_currency` = 0: the fee is charged in the swap's INPUT currency (v4's rule). A pool
/// with no listed quote side, or with two listed quotes neither of which is wSOL (a wSOL side wins).
pub const PROTOCOL_FEE_ON_INPUT: u8 = 0;
/// `Pool.protocol_fee_currency` = 1: currency 0 is the pool's wSOL side, or its ONE listed quote (a
/// `Listed` roster mint), so the fee is always charged in currency 0 — on the input of a
/// `zero_for_one` swap, on the OUTPUT of the other direction (PumpSwap's rule). Every FLOOR pool
/// (the quote is always currency 0).
pub const PROTOCOL_FEE_ON_CURRENCY0: u8 = 1;
/// `Pool.protocol_fee_currency` = 2: currency 1 is the pool's one listed quote.
pub const PROTOCOL_FEE_ON_CURRENCY1: u8 = 2;

// The fee's own invariants, at compile time.
const _: () = {
    // the packed literal is the default in both nibbles
    assert!(PROTOCOL_FEE_DEFAULT_PACKED == PROTOCOL_FEE_DEFAULT | (PROTOCOL_FEE_DEFAULT << 12));
    // 0 < default <= the cap governance may set
    assert!(PROTOCOL_FEE_DEFAULT > 0 && PROTOCOL_FEE_DEFAULT <= MAX_PROTOCOL_FEE);
    // the cap fits v4's 12-bit nibble, so a capped setting can never spill into the other direction
    assert!(MAX_PROTOCOL_FEE <= 0xfff);
    // and stays far below the denominator: an exact-out gross-up divides by `1e6 − p`
    assert!(MAX_PROTOCOL_FEE < PIPS_DENOMINATOR / 2);
    // the three fee-currency codes are distinct, and 0 (a zeroed pool) is the v4 rule
    assert!(PROTOCOL_FEE_ON_INPUT == 0);
    assert!(PROTOCOL_FEE_ON_CURRENCY0 != PROTOCOL_FEE_ON_CURRENCY1);
    assert!(PROTOCOL_FEE_ON_CURRENCY0 != PROTOCOL_FEE_ON_INPUT && PROTOCOL_FEE_ON_CURRENCY1 != PROTOCOL_FEE_ON_INPUT);
};

// ---- hook flags (v4's bits exactly, `Hooks.sol:29-47`, plus one Solana bit) -------------
pub const BEFORE_INITIALIZE_FLAG: u16 = 1 << 13;
pub const AFTER_INITIALIZE_FLAG: u16 = 1 << 12;
pub const BEFORE_ADD_LIQUIDITY_FLAG: u16 = 1 << 11;
pub const AFTER_ADD_LIQUIDITY_FLAG: u16 = 1 << 10;
pub const BEFORE_REMOVE_LIQUIDITY_FLAG: u16 = 1 << 9;
pub const AFTER_REMOVE_LIQUIDITY_FLAG: u16 = 1 << 8;
pub const BEFORE_SWAP_FLAG: u16 = 1 << 7;
pub const AFTER_SWAP_FLAG: u16 = 1 << 6;
pub const BEFORE_DONATE_FLAG: u16 = 1 << 5;
pub const AFTER_DONATE_FLAG: u16 = 1 << 4;
pub const BEFORE_SWAP_RETURNS_DELTA_FLAG: u16 = 1 << 3;
pub const AFTER_SWAP_RETURNS_DELTA_FLAG: u16 = 1 << 2;
pub const AFTER_ADD_LIQUIDITY_RETURNS_DELTA_FLAG: u16 = 1 << 1;
pub const AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA_FLAG: u16 = 1 << 0;
/// Solana-only: the hook may return actions from `after_swap` / `after_actions`.
pub const REQUESTS_ACTIONS_FLAG: u16 = 1 << 14;
/// Solana-only: Uniswap v4's settle, on Solana. A hook carrying it is called under its OWN
/// signer PDA `["hook_settle", pool]` (never the pool's), may cap the trader's walk with a price limit
/// from `before_swap`, is handed both vaults and both mints writable in the swap callbacks so it can
/// MINT into a vault (the AMM credits the measured increase to its claims and refuses a decrease), and
/// may return NEGATIVE deltas — pay the trader out of its claims — and an `after_swap`
/// `delta_specified` that takes the unfilled remainder of an exact-in. Requires `BEFORE_SWAP`,
/// `AFTER_SWAP` and `AFTER_SWAP_RETURNS_DELTA` (`hook_flags_valid`). A pool whose hook lacks it runs
/// exactly as before.
pub const SETTLES_FLAG: u16 = 1 << 15;
/// All sixteen defined bits.
pub const ALL_HOOK_FLAGS: u16 = 0xFFFF;
/// v4's fourteen bits (`Hooks.sol` `ALL_HOOK_MASK`).
pub const V4_HOOK_FLAGS: u16 = 0x3FFF;

/// The pause guardian: the longest a guardian pause of a pool or of a hook's new pools lasts. It lapses on its own; the guardian cannot extend it.
pub const GUARDIAN_PAUSE_MAX_SECS: i64 = 72 * 3_600;
/// After a guardian pause ends (lapsed, or ended by the config
/// authority), the guardian may not pause the same pool / hook again for this long, so holders always get a trading
/// window between guardian pauses.
pub const GUARDIAN_COOLOFF_SECS: i64 = 72 * 3_600;

/// "This pool is paused now": the config authority's unbounded flag, or a guardian
/// pause that has not lapsed. `carpenter_amm::require_not_paused` and `floor_hook`'s team-step withdrawal both read it.
pub const fn pool_is_paused(paused: u8, guardian_paused_until: i64, now: i64) -> bool {
    paused != 0 || now < guardian_paused_until
}

/// The guardian's terms, pure: `Ok(until)` — the new `guardian_paused_until` — or the refusal.
/// `Err(true)`: a guardian pause is still running (no extending). `Err(false)`: inside the cool-off after the last one.
pub const fn guardian_pause_until(guardian_paused_until: i64, now: i64) -> Result<i64, bool> {
    if now < guardian_paused_until {
        return Err(true);
    }
    if guardian_paused_until != 0 && now < guardian_paused_until + GUARDIAN_COOLOFF_SECS {
        return Err(false);
    }
    Ok(now + GUARDIAN_PAUSE_MAX_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two masks are exactly the fifteen (fourteen) bits named above — a bit added or dropped
    /// above without moving the mask fails here.
    #[test]
    fn flag_masks_are_the_named_bits() {
        let v4 = BEFORE_INITIALIZE_FLAG
            | AFTER_INITIALIZE_FLAG
            | BEFORE_ADD_LIQUIDITY_FLAG
            | AFTER_ADD_LIQUIDITY_FLAG
            | BEFORE_REMOVE_LIQUIDITY_FLAG
            | AFTER_REMOVE_LIQUIDITY_FLAG
            | BEFORE_SWAP_FLAG
            | AFTER_SWAP_FLAG
            | BEFORE_DONATE_FLAG
            | AFTER_DONATE_FLAG
            | BEFORE_SWAP_RETURNS_DELTA_FLAG
            | AFTER_SWAP_RETURNS_DELTA_FLAG
            | AFTER_ADD_LIQUIDITY_RETURNS_DELTA_FLAG
            | AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA_FLAG;
        assert_eq!(v4, V4_HOOK_FLAGS);
        assert_eq!(v4 | REQUESTS_ACTIONS_FLAG | SETTLES_FLAG, ALL_HOOK_FLAGS);
        assert_eq!(SETTLES_FLAG, 1 << 15, "the settle flag is the last bit");
    }

    #[test]
    fn swap_fee_caps_agree_with_v4() {
        assert_eq!(MAX_SWAP_FEE, MAX_LP_FEE);
        assert!(DYNAMIC_FEE_FLAG > MAX_LP_FEE, "the dynamic flag is not a fee");
    }

    /// The pause guardian: 72 h, no extending, a 72 h cool-off measured from the END, and the authority's
    /// flag independent of the clock. Mutations this kills: an extendable pause; a cool-off from the START; a pause
    /// that never lapses; a cool-off on a pool the guardian never paused.
    #[test]
    fn the_guardian_pause_lapses_cannot_extend_and_cools_off() {
        let t = 1_790_000_000;
        assert_eq!(guardian_pause_until(0, t), Ok(t + GUARDIAN_PAUSE_MAX_SECS), "never paused: no cool-off");
        let until = t + GUARDIAN_PAUSE_MAX_SECS;
        assert!(pool_is_paused(0, until, t) && pool_is_paused(0, until, until - 1));
        assert!(!pool_is_paused(0, until, until), "it lapses on its own at the 72nd hour");
        assert_eq!(guardian_pause_until(until, until - 1), Err(true), "no extending");
        assert_eq!(guardian_pause_until(until, until), Err(false), "the cool-off starts at the end");
        assert_eq!(guardian_pause_until(until, until + GUARDIAN_COOLOFF_SECS - 1), Err(false));
        assert_eq!(guardian_pause_until(until, until + GUARDIAN_COOLOFF_SECS), Ok(until + GUARDIAN_COOLOFF_SECS + GUARDIAN_PAUSE_MAX_SECS));
        assert!(pool_is_paused(1, 0, t) && pool_is_paused(1, 0, i64::MAX), "the authority's pause never lapses");
        assert_eq!((GUARDIAN_PAUSE_MAX_SECS, GUARDIAN_COOLOFF_SECS), (259_200, 259_200));
    }
}
