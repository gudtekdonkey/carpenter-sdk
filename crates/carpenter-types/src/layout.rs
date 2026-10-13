//! Every `carpenter_amm` account layout, as `#[repr(C)]` + `Pod` structs, the [`Layout`] trait
//! every account layout (this crate's and `floor_types::layout`'s) implements, the [`layout!`]
//! macro that implements it with the size / offset / contiguity asserts, and the 8-byte
//! [`HookInterfaceHeader`] the AMM reads under any approved hook.
//!
//! These are the on-chain bytes. Each program declares its own `#[account(zero_copy)]` twin
//! (Anchor needs the struct in the program crate for the discriminator, `Owner` and the IDL),
//! and those twins are held to these in two places:
//! - at compile time, each program asserts `size_of::<Twin>() == <carpenter_types::layout::T>::SIZE`;
//! - the program repository's layout test walks each built IDL and requires the same field names,
//!   in the same order, at the same offsets and sizes as [`Layout::FIELDS`] below, and the same
//!   discriminator as [`Layout::DISCRIMINATOR`].
//!
//! Offsets are STRUCT offsets; account offset = struct offset + 8 (the discriminator).
//!
//! Layout rules: every u64 / U128 / I128 sits at an offset that is a multiple of 8, no
//! implicit padding (`Pod` refuses to derive otherwise), `size % 8 == 0 && align <= 8`.
//! `MintExclusivity` aligns to 4 (and `floor_types`' `HookConfig` to 2), so `align == 8` is never
//! asserted.
//! The `layout!` macro also requires the field list to be CONTIGUOUS (`offset + size` of one
//! field is the next field's offset, and the last ends at `SIZE`), so a field whose listed type
//! disagrees with the struct cannot pass by accident.
//! No FLOOR layout is here: `HookConfig`, `FloorPool`, `LaunchConfig`, `RegistryEntry`,
//! `Launch` and `BuyerPosition` are `floor_types::layout`, built with this macro and trait.

use crate::disc::account as disc;
use crate::pod::{I128, U128};
use bytemuck::{Pod, Zeroable};
/// One field of a layout: `(name, struct offset, size)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Field {
    pub name: &'static str,
    pub offset: usize,
    pub size: usize,
}

/// An account layout.
pub trait Layout: Pod {
    /// The Anchor account name (`sha256("account:<NAME>")[..8]` is the discriminator).
    const NAME: &'static str;
    /// `size_of::<Self>()`, the struct without the 8-byte discriminator.
    const SIZE: usize;
    /// Every field in declaration order.
    const FIELDS: &'static [Field];
    /// Anchor's account discriminator.
    const DISCRIMINATOR: [u8; 8];
    /// `8 + SIZE`: the account's data length (the bitmap adds its words on top).
    const ACCOUNT_LEN: usize = 8 + Self::SIZE;
}

/// Implement [`Layout`] for a `#[repr(C)]` `Pod` struct and assert its size, every field's
/// offset, and that the field list is contiguous and ends at `SIZE` — all at compile time.
///
/// `layout!(T, name = "Anchor account name", size = N, disc = <[u8; 8] expr>, { field: Type = offset, … });`
///
/// `#[macro_export]`ed so `floor_types::layout` builds the hook's accounts with the same
/// asserts; every path inside is `$crate::`-qualified, so it works from any crate.
#[macro_export]
macro_rules! layout {
    (
        $t:ident, name = $name:literal, size = $size:expr, disc = $disc:expr,
        { $( $f:ident : $fty:ty = $off:expr ),* $(,)? }
    ) => {
        impl $crate::layout::Layout for $t {
            const NAME: &'static str = $name;
            const SIZE: usize = $size;
            const FIELDS: &'static [$crate::layout::Field] = &[
                $( $crate::layout::Field { name: stringify!($f), offset: $off, size: core::mem::size_of::<$fty>() } ),*
            ];
            const DISCRIMINATOR: [u8; 8] = $disc;
        }
        const _: () = {
            assert!(core::mem::size_of::<$t>() == $size, concat!(stringify!($t), ": size"));
            assert!(core::mem::size_of::<$t>() % 8 == 0, concat!(stringify!($t), ": size % 8"));
            assert!(core::mem::align_of::<$t>() <= 8, concat!(stringify!($t), ": align"));
            $( assert!(core::mem::offset_of!($t, $f) == $off, concat!(stringify!($t), ".", stringify!($f), ": offset")); )*
            let fields = <$t as $crate::layout::Layout>::FIELDS;
            let mut i = 0;
            let mut end = 0;
            while i < fields.len() {
                assert!(fields[i].offset == end, concat!(stringify!($t), ": fields not contiguous"));
                end = fields[i].offset + fields[i].size;
                i += 1;
            }
            assert!(end == $size, concat!(stringify!($t), ": fields do not end at SIZE"));
        };
    };
}

// =========================================================================================
// carpenter_amm
// =========================================================================================

/// `["amm_config"]`. Account 216 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct AmmConfig {
    pub version: u8,
    pub bump: u8,
    pub pad0: [u8; 6],
    /// The config authority: hook approvals, `set_hook_paused`, `set_pool_paused`, quote mints,
    /// exclusivity revocation (v1: the deployer key on every cluster).
    pub authority: [u8; 32],
    pub pending_authority: [u8; 32],
    /// Reserved; always default. v4's per-pool fee controller is not how the protocol fee is set
    /// here: the protocol treasury's governance (`ProtocolTreasury.governance`, not
    /// `AmmConfig.authority`) sets the CURRENT SETTING below (`set_protocol_fee`) and new pools copy it.
    pub protocol_fee_controller: [u8; 32],
    /// The pause guardian: the key that may pause ONE pool (`guardian_pause_pool`) or one approved
    /// hook's new pools (`guardian_pause_hook`) for at most `GUARDIAN_PAUSE_MAX_SECS`, never extended,
    /// then not again on the same pool / hook for `GUARDIAN_COOLOFF_SECS`. Zero = no guardian. Written
    /// only by `set_pause_guardian` (the config authority). Zero on a freshly initialized config.
    pub pause_guardian: [u8; 32],
    /// Unused padding. wSOL is the implied quote; every other quote is on the roster (`QuoteMint`).
    pub pad1: [u8; 40],
    /// The `QUOTE_CAP_*` bits governance has admitted for a LISTED QUOTE (the program's mint gate,
    /// `check_admitted`; written by `set_quote_caps_admitted`, config authority, SET not OR).
    /// 0 = no custodial power admitted. A zeroed byte reads as nothing admitted.
    pub quote_caps_admitted: u8,
    /// Unused padding; `initialize_config` always writes the fee.
    pub pad2: [u8; 3],
    /// The CURRENT SETTING, v4-packed (`zero_for_one | one_for_zero << 12`, each ≤
    /// `MAX_PROTOCOL_FEE`): what `initialize_pool` COPIES into `Pool.protocol_fee`, so a change
    /// applies to pools created after it only.
    pub protocol_fee: u32,
    pub reserved: [u8; 24],
}
layout!(AmmConfig, name = "AmmConfig", size = 208, disc = disc::AMM_CONFIG, {
    version: u8 = 0, bump: u8 = 1, pad0: [u8; 6] = 2,
    authority: [u8; 32] = 8, pending_authority: [u8; 32] = 40, protocol_fee_controller: [u8; 32] = 72,
    pause_guardian: [u8; 32] = 104, pad1: [u8; 40] = 136,
    quote_caps_admitted: u8 = 176, pad2: [u8; 3] = 177,
    protocol_fee: u32 = 180, reserved: [u8; 24] = 184,
});

/// `["approved_hook", hook_program]`. Account 200 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ApprovedHook {
    pub version: u8,
    pub bump: u8,
    /// Blocks NEW pools only.
    pub paused: u8,
    pub max_rounds: u8,
    pub callback_accounts: u8,
    pub allow_public_pools: u8,
    pub flags: u16,
    pub hook_program: [u8; 32],
    pub hook_authority: [u8; 32],
    pub hook_authority_bump: u8,
    pub pad0: [u8; 7],
    /// The hook's `ProgramData.slot` at approval.
    pub programdata_slot: u64,
    pub approved_slot: u64,
    /// UTF-8, zero-padded.
    pub name: [u8; 32],
    /// Set only by the config authority, in `approve_hook`.
    pub allow_exclusivity: u8,
    /// The ONE Token-2022 transfer-hook PROGRAM a pool of this approved hook may carry ARMED on its
    /// BASE mint. Zero = none. SET ONCE, by the config authority, in `set_base_transfer_hook`;
    /// `reapprove_hook` preserves it (it writes the slot pin alone). Copied into
    /// `Pool.base_transfer_hook` at `initialize_pool`, which is what the transfer legs read.
    pub base_transfer_hook: [u8; 32],
    /// Hook-listed quotes: 1 = this approved hook may list a quote mint FOR ITS OWN POOLS
    /// (`hook_list_quote_mint`). Set and cleared by the config authority (`set_hook_quotes`); `approve_hook`
    /// writes 1. Clearing it stops NEW hook listings only: an entry already written keeps admitting that
    /// hook's pools, and governance's `retire_quote_mint` is the way to stop a quote.
    pub allow_hook_quotes: u8,
    pub reserved: [u8; 22],
    /// The unix time a guardian's pause of this hook's NEW pools ends (`guardian_pause_hook`; 0 = never
    /// paused by the guardian). `initialize_pool` is refused while `now < guardian_paused_until`; a FLOOR
    /// pool is created at graduation, so this is a bounded brake on a live sale's graduation. `paused`
    /// above is the config authority's unbounded one.
    pub guardian_paused_until: i64,
}
layout!(ApprovedHook, name = "ApprovedHook", size = 192, disc = disc::APPROVED_HOOK, {
    version: u8 = 0, bump: u8 = 1, paused: u8 = 2, max_rounds: u8 = 3, callback_accounts: u8 = 4,
    allow_public_pools: u8 = 5, flags: u16 = 6, hook_program: [u8; 32] = 8, hook_authority: [u8; 32] = 40,
    hook_authority_bump: u8 = 72, pad0: [u8; 7] = 73, programdata_slot: u64 = 80, approved_slot: u64 = 88,
    name: [u8; 32] = 96, allow_exclusivity: u8 = 128, base_transfer_hook: [u8; 32] = 129, allow_hook_quotes: u8 = 161,
    reserved: [u8; 22] = 162,
    guardian_paused_until: i64 = 184,
});

/// `["mint_excl", mint]`. Struct 72 B, account 80 B. Present when the account exists with this
/// discriminator.
///
/// `register_mint_exclusivity` writes the AMM's OWN
/// `AmmConfig.protocol_fee` here — a FLOOR launch's `create_launch` reaches it by CPI — and
/// `initialize_pool` charges the recorded rate instead of the config's current one, so a launch's
/// pool pays the Carpenter Platform fee of the launch's CREATION. Never a caller's number.
/// `has_protocol_fee` is 1 when written (0 is a valid rate, so the flag tells it from padding).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MintExclusivity {
    pub mint: [u8; 32],
    pub hook_program: [u8; 32],
    pub bump: u8,
    pub has_protocol_fee: u8,
    pub pad0: [u8; 2],
    pub protocol_fee: u32,
}
layout!(MintExclusivity, name = "MintExclusivity", size = 72, disc = disc::MINT_EXCLUSIVITY, {
    mint: [u8; 32] = 0, hook_program: [u8; 32] = 32, bump: u8 = 64, has_protocol_fee: u8 = 65, pad0: [u8; 2] = 66,
    protocol_fee: u32 = 68,
});

/// `["mint_pools", mint]`. Struct 56 B, account 64 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct MintPoolCount {
    pub mint: [u8; 32],
    pub count: u64,
    pub bump: u8,
    pub reserved: [u8; 15],
}
layout!(MintPoolCount, name = "MintPoolCount", size = 56, disc = disc::MINT_POOL_COUNT, {
    mint: [u8; 32] = 0, count: u64 = 32, bump: u8 = 40, reserved: [u8; 15] = 41,
});

/// `["quote_mint", mint]`. Struct 120 B, account 128 B.
///
/// ONE account per listed quote mint, so the roster is unbounded, removable, and carries the
/// capability snapshot the gate accepted.
/// `state` is never 0: a zeroed account of this length must read as ABSENT. The discriminator
/// already guarantees that; the invariant is written down so nobody introduces a `State::Unset`
/// that an all-zero account would satisfy.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct QuoteMint {
    pub mint: [u8; 32],
    /// The LISTING-POLICY snapshot: `sha256` over the mint's capability authorities as they read
    /// at `list_quote_mint` (freeze, permanent delegate, pause and transfer-hook authority, each
    /// 32 B, absent = zero). It grants nothing and gates nothing on chain; it is the record that
    /// lets an operator see that the key set MOVED since listing. Sunrise's family shares one key
    /// across 62 mints - that refusal lives in the listing process, and this is the evidence for it.
    pub authority_digest: [u8; 32],
    pub listed_slot: u64,
    /// 0 while listed.
    pub retired_slot: u64,
    /// 1 = Listed, 2 = Retired, 3 = Hook-listed. Never 0 (see above); a delisted entry is CLOSED, not zeroed.
    pub state: u8,
    pub bump: u8,
    /// The mint's decimals at listing (a client convenience; the chain is still the authority).
    pub decimals: u8,
    /// The capability bits `check_mint_supported` accepted at listing (`QUOTE_CAP_*`).
    pub caps: u8,
    pub pad0: [u8; 4],
    /// The approved hook that listed this mint as a quote FOR ITS OWN POOLS (`hook_list_quote_mint`, state
    /// [`QUOTE_STATE_HOOK_LISTED`]). Zero on every governance listing (state 1 / 2).
    pub hook_program: [u8; 32],
}
layout!(QuoteMint, name = "QuoteMint", size = 120, disc = disc::QUOTE_MINT, {
    mint: [u8; 32] = 0, authority_digest: [u8; 32] = 32, listed_slot: u64 = 64, retired_slot: u64 = 72,
    state: u8 = 80, bump: u8 = 81, decimals: u8 = 82, caps: u8 = 83, pad0: [u8; 4] = 84,
    hook_program: [u8; 32] = 88,
});

/// `QuoteMint.state`: the entry admits new pools.
pub const QUOTE_STATE_LISTED: u8 = 1;
/// `QuoteMint.state`: no NEW pool may open against it; every live pool keeps trading.
pub const QUOTE_STATE_RETIRED: u8 = 2;
/// `QuoteMint.state`: a quote ONLY for pools of the hook named in `QuoteMint.hook_program`,
/// listed by that APPROVED hook (`hook_list_quote_mint`, gated on `ApprovedHook.allow_hook_quotes`). It is not an
/// AMM-wide listing: a pool of any other hook, or a hookless pool, reads the mint as a BASE mint, exactly as if no
/// entry existed. Governance may retire it (→ [`QUOTE_STATE_RETIRED`]).
pub const QUOTE_STATE_HOOK_LISTED: u8 = 3;

/// `["pool", mint0, mint1, fee u32 LE, tick_spacing u16 LE, hook]`. Struct 632 B, account 640 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Pool {
    pub version: u8,
    pub bump: u8,
    pub hook_flags: u16,
    pub tick_spacing: u16,
    pub hook_max_rounds: u8,
    pub hook_callback_accounts: u8,
    /// Key fee, pips; `0x800000` = dynamic.
    pub fee: u32,
    /// Current LP fee; a dynamic pool starts at 0.
    pub lp_fee: u32,
    /// The Carpenter Platform fee this pool charges, v4 packing (`zero_for_one | one_for_zero << 12`,
    /// pips). COPIED once at `initialize_pool` from `AmmConfig`'s current setting (or the mint's
    /// recorded rate) and never written again, so a pool keeps the rate it was created under.
    pub protocol_fee: u32,
    pub tick_current: i32,
    pub mint0: [u8; 32],
    pub mint1: [u8; 32],
    pub hook: [u8; 32],
    pub hook_authority: [u8; 32],
    pub vault0: [u8; 32],
    pub vault1: [u8; 32],
    pub token_program0: [u8; 32],
    pub token_program1: [u8; 32],
    pub sqrt_price_x64: U128,
    pub liquidity: U128,
    /// Wrapping.
    pub fee_growth_global0_x64: U128,
    pub fee_growth_global1_x64: U128,
    pub protocol_fees_owed0: u64,
    pub protocol_fees_owed1: u64,
    /// The Solana form of v4's ERC-6909 claims.
    pub hook_claims0: u64,
    pub hook_claims1: u64,
    /// Σ input per currency.
    pub volume0: U128,
    pub volume1: U128,
    pub lp_fees0: U128,
    pub lp_fees1: U128,
    /// Σ hook deltas credited.
    pub hook_fees0: U128,
    pub hook_fees1: U128,
    pub swap_count: u64,
    pub created_slot: u64,
    pub decimals0: u8,
    pub decimals1: u8,
    pub bitmap_bump: u8,
    pub vault0_bump: u8,
    pub vault1_bump: u8,
    pub pad0: [u8; 3],
    /// Unused; nothing assigns it.
    pub reserved0: u64,
    /// `set_pool_paused`: 1 while the pool is paused. While set,
    /// `swap`, `donate`, `hook_deposit` and a `modify_liquidity` with a positive delta are refused
    /// (`PoolPaused`); removals, pokes, `close_position` and `hook_withdraw` stay open.
    pub paused: u8,
    /// The transfer-hook PROGRAM this pool admits ARMED on its base mint — copied ONCE, at
    /// `initialize_pool`, from `ApprovedHook.base_transfer_hook`, and never written again. Zero = none:
    /// every leg of the pool refuses an armed hook (`QuoteHookArmed`).
    pub base_transfer_hook: [u8; 32],
    /// The side (0 / 1) `base_transfer_hook` applies to — the pool's single `Base`-role mint at
    /// creation (FLOOR's launch mint is side 1). Meaningless while `base_transfer_hook` is
    /// zero. Only THIS side's legs may pass an admitted program to `require_transfer_safe`.
    pub base_hook_side: u8,
    /// Which currency this pool's `protocol_fee` is charged in — `PROTOCOL_FEE_ON_INPUT` (0, v4's input
    /// rule: no listed quote side, or two listed quotes neither of which is wSOL),
    /// `PROTOCOL_FEE_ON_CURRENCY0` (1) or `PROTOCOL_FEE_ON_CURRENCY1` (2): the pool's wSOL side if it
    /// has one (a wSOL / listed-quote pool included), else its ONE listed quote, on buys AND sells.
    /// Set once at `initialize_pool` from the mints' roles (`protocol_fee::currency_for_roles`), never
    /// written again.
    pub protocol_fee_currency: u8,
    pub reserved: [u8; 85],
    /// The unix time a guardian's pause of this pool ends (`guardian_pause_pool`; 0 = never paused by the
    /// guardian). The pool is paused while `paused == 1` (the config authority's, unbounded) OR
    /// `now < guardian_paused_until` ([`pool_is_paused`]). It lapses on its own: no transaction resumes it.
    pub guardian_paused_until: i64,
}
layout!(Pool, name = "Pool", size = 632, disc = disc::POOL, {
    version: u8 = 0, bump: u8 = 1, hook_flags: u16 = 2, tick_spacing: u16 = 4,
    hook_max_rounds: u8 = 6, hook_callback_accounts: u8 = 7, fee: u32 = 8, lp_fee: u32 = 12,
    protocol_fee: u32 = 16, tick_current: i32 = 20, mint0: [u8; 32] = 24, mint1: [u8; 32] = 56,
    hook: [u8; 32] = 88, hook_authority: [u8; 32] = 120, vault0: [u8; 32] = 152, vault1: [u8; 32] = 184,
    token_program0: [u8; 32] = 216, token_program1: [u8; 32] = 248, sqrt_price_x64: U128 = 280,
    liquidity: U128 = 296, fee_growth_global0_x64: U128 = 312, fee_growth_global1_x64: U128 = 328,
    protocol_fees_owed0: u64 = 344, protocol_fees_owed1: u64 = 352, hook_claims0: u64 = 360,
    hook_claims1: u64 = 368, volume0: U128 = 376, volume1: U128 = 392, lp_fees0: U128 = 408,
    lp_fees1: U128 = 424, hook_fees0: U128 = 440, hook_fees1: U128 = 456, swap_count: u64 = 472,
    created_slot: u64 = 480, decimals0: u8 = 488, decimals1: u8 = 489, bitmap_bump: u8 = 490,
    vault0_bump: u8 = 491, vault1_bump: u8 = 492, pad0: [u8; 3] = 493, reserved0: u64 = 496,
    paused: u8 = 504, base_transfer_hook: [u8; 32] = 505, base_hook_side: u8 = 537,
    protocol_fee_currency: u8 = 538, reserved: [u8; 85] = 539,
    guardian_paused_until: i64 = 624,
});

/// `["bitmap", pool]`: the FIXED part. The words follow as raw little-endian `u64`s at
/// account offset `8 + 40`; they are never a struct field, because their count depends on
/// the spacing and a zero-copy struct has one compile-time size.
///
/// Named `TickArrayBitmapHeader` here; the Anchor ACCOUNT is
/// `TickArrayBitmap`, so the discriminator is `sha256("account:TickArrayBitmap")`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TickArrayBitmapHeader {
    pub pool: [u8; 32],
    pub tick_spacing: u16,
    pub pad0: u16,
    pub arrays_per_side: u32,
}
layout!(TickArrayBitmapHeader, name = "TickArrayBitmap", size = 40, disc = disc::TICK_ARRAY_BITMAP, {
    pool: [u8; 32] = 0, tick_spacing: u16 = 32, pad0: u16 = 34, arrays_per_side: u32 = 36,
});

/// Bitmap sizing: `arrays_per_side = ceil(443,636 / (60·s))`.
pub const fn bitmap_arrays_per_side(tick_spacing: u16) -> u32 {
    let span = (crate::constants::TICK_ARRAY_SIZE as u32) * (tick_spacing as u32);
    (crate::constants::MAX_TICK as u32).div_ceil(span)
}

/// Bitmap sizing: `N = ceil((2·arrays_per_side + 1) / 64)` words.
pub const fn bitmap_word_count(tick_spacing: u16) -> usize {
    ((2 * bitmap_arrays_per_side(tick_spacing) + 1) as usize).div_ceil(64)
}

/// The bitmap account is allocated to exactly `8 + 40 + 8·N` bytes.
pub const fn bitmap_account_len(tick_spacing: u16) -> usize {
    8 + TickArrayBitmapHeader::SIZE + 8 * bitmap_word_count(tick_spacing)
}

/// `start_index = floor(tick / (60·s)) · 60·s` (floor toward −∞).
pub const fn tick_array_start_index(tick: i32, tick_spacing: u16) -> i32 {
    let span = crate::constants::TICK_ARRAY_SIZE * tick_spacing as i32;
    tick.div_euclid(span) * span
}

/// The bitmap bit of the array starting at `start_index` is
/// `start_index / (60·s) + arrays_per_side`.
pub const fn bitmap_bit(start_index: i32, tick_spacing: u16) -> i64 {
    let span = crate::constants::TICK_ARRAY_SIZE * tick_spacing as i32;
    (start_index / span) as i64 + bitmap_arrays_per_side(tick_spacing) as i64
}

/// One tick (64 B). No reward fields.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Tick {
    pub liquidity_gross: U128,
    pub liquidity_net: I128,
    pub fee_growth_outside0_x64: U128,
    pub fee_growth_outside1_x64: U128,
}
const _: () = assert!(core::mem::size_of::<Tick>() == 64);

/// `["tick_array", pool, start_index i32 LE]`. Struct 3,880 B, account 3,888 B.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct TickArray {
    pub pool: [u8; 32],
    pub start_index: i32,
    pub initialized_tick_count: u16,
    pub pad0: u16,
    pub ticks: [Tick; 60],
}
layout!(TickArray, name = "TickArray", size = 3880, disc = disc::TICK_ARRAY, {
    pool: [u8; 32] = 0, start_index: i32 = 32, initialized_tick_count: u16 = 36, pad0: u16 = 38,
    ticks: [Tick; 60] = 40,
});

/// Account offset of the 32-byte rent owner, which is also where the 3,888-byte body (`TickArray::ACCOUNT_LEN`) ends.
pub const TICK_ARRAY_RENT_OWNER_OFFSET: usize = <TickArray as Layout>::ACCOUNT_LEN; // 3,888
/// A tick array account: the body, then the rent owner (`close_tick_array` refunds it). This is the
/// only accepted length.
pub const TICK_ARRAY_LEN: usize = TICK_ARRAY_RENT_OWNER_OFFSET + 32; // 3,920
/// The one length a tick array account can have.
pub const fn tick_array_len_ok(len: usize) -> bool { len == TICK_ARRAY_LEN }
/// The recorded rent owner (it may be all-zero; callers treat that as none). `None` on any other length.
pub fn tick_array_rent_owner(data: &[u8]) -> Option<[u8; 32]> {
    if data.len() != TICK_ARRAY_LEN { return None; }
    let mut k = [0u8; 32];
    k.copy_from_slice(&data[TICK_ARRAY_RENT_OWNER_OFFSET..TICK_ARRAY_LEN]);
    Some(k)
}

/// `["position", pool, position_id, salt u64 LE]`. Struct 200 B, account 208 B.
///
/// The OWNER IS A FIELD, NOT A SEED: a position can change hands (`transfer_position`), and an
/// NFT can be minted over its id later with no change to this account.
///
/// The 32-byte `reserved` tail at struct offset 168 is a forward-compatibility reserve (for an NFT
/// wrapper: a delegate, an nft_mint, a lock-until, a fee snapshot); growing the account later would
/// be a realloc migration of every position.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Position {
    pub version: u8,
    pub bump: u8,
    pub pad0: [u8; 6],
    pub pool: [u8; 32],
    /// Account offset 48: the `positionsForOwner` memcmp. Mutable (`transfer_position`).
    pub owner: [u8; 32],
    pub salt: u64,
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub liquidity: U128,
    pub fee_growth_inside0_last_x64: U128,
    pub fee_growth_inside1_last_x64: U128,
    /// Account offset 144: the second seed, fixed at open. NEVER the owner.
    pub position_id: [u8; 32],
    /// Reserved; always default. See the type docs.
    pub reserved: [u8; 32],
}
layout!(Position, name = "Position", size = 200, disc = disc::POSITION, {
    version: u8 = 0, bump: u8 = 1, pad0: [u8; 6] = 2, pool: [u8; 32] = 8, owner: [u8; 32] = 40,
    salt: u64 = 72, tick_lower: i32 = 80, tick_upper: i32 = 84, liquidity: U128 = 88,
    fee_growth_inside0_last_x64: U128 = 104, fee_growth_inside1_last_x64: U128 = 120,
    position_id: [u8; 32] = 136, reserved: [u8; 32] = 168,
});

/// `["pool_index", pool]`. Struct 48 B, account 56 B. Carries no index.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PoolIndexEntry {
    pub pool: [u8; 32],
    pub created_slot: u64,
    pub bump: u8,
    pub reserved: [u8; 7],
}
layout!(PoolIndexEntry, name = "PoolIndexEntry", size = 48, disc = disc::POOL_INDEX_ENTRY, {
    pool: [u8; 32] = 0, created_slot: u64 = 32, bump: u8 = 40, reserved: [u8; 7] = 41,
});

/// `["protocol_treasury"]` under `carpenter_amm`: the Carpenter Platform fee's treasury. The fee is
/// owed on the pool, moved by a permissionless sweep into this program-owned treasury, and withdrawn
/// by governance only, with a two-step governance handover. Struct 104 B, account 112 B.
///
/// The account whose ADDRESS owns every token account a pool's `protocol_fees_owed0/1` is swept into
/// (`ATA(this, mint, token program)`, one per mint, opened by the first sweep in that mint — the
/// sweeper pays the rent), and whose DATA is the governance that alone may withdraw from them, move
/// the protocol-fee setting new pools copy (`set_protocol_fee`, capped at `MAX_PROTOCOL_FEE`), and
/// hand itself over in two steps (`set_protocol_treasury_governance` → `accept_…`). No private key
/// exists for it. The same shape as `floor_types::layout::PlatformTreasury` (FLOOR's treasury under
/// `floor_hook`), on purpose: two treasuries, one reading.
/// It has its OWN governance, not `AmmConfig.authority` (`AmmConfig.reserved` has no room for a
/// pending key).
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct ProtocolTreasury {
    pub version: u8,
    pub bump: u8,
    pub pad0: [u8; 6],
    /// The ONE key that may withdraw, set the protocol fee and propose a successor.
    pub governance: [u8; 32],
    /// The proposed successor (zero = none); it becomes `governance` only by signing the accept.
    pub pending_governance: [u8; 32],
    pub reserved: [u8; 32],
}
layout!(ProtocolTreasury, name = "ProtocolTreasury", size = 104, disc = disc::PROTOCOL_TREASURY, {
    version: u8 = 0, bump: u8 = 1, pad0: [u8; 6] = 2, governance: [u8; 32] = 8,
    pending_governance: [u8; 32] = 40, reserved: [u8; 32] = 72,
});

// =========================================================================================
// The hook interface header — under ANY approved hook program
// =========================================================================================
/// The 8-byte interface header EVERY approved hook carries as the first 8 struct bytes of
/// its `["hook_config"]` account (account offset 8..16). `approve_hook` reads exactly this.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Pod, Zeroable)]
pub struct HookInterfaceHeader {
    pub version: u8,
    pub bump: u8,
    pub max_rounds: u8,
    pub callback_accounts: u8,
    pub flags: u16,
    pub authority_bump: u8,
    pub pad0: u8,
}
const _: () = assert!(core::mem::size_of::<HookInterfaceHeader>() == 8);

impl HookInterfaceHeader {
    /// Account data offset of the header (after the discriminator).
    pub const ACCOUNT_OFFSET: usize = 8;

    /// Read the header out of a `hook_config` account's data. `None` if too short.
    pub fn from_account_data(data: &[u8]) -> Option<Self> {
        let raw = data.get(Self::ACCOUNT_OFFSET..Self::ACCOUNT_OFFSET + 8)?;
        Some(bytemuck::pod_read_unaligned(raw))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The account lengths. The hook's accounts are pinned the same way in `floor_types::layout`.
    #[test]
    fn account_lengths() {
        assert_eq!(AmmConfig::ACCOUNT_LEN, 216);
        assert_eq!(ApprovedHook::ACCOUNT_LEN, 200);
        assert_eq!(MintExclusivity::ACCOUNT_LEN, 80);
        assert_eq!(MintPoolCount::ACCOUNT_LEN, 64);
        assert_eq!(Pool::ACCOUNT_LEN, 640);
        assert_eq!(TickArray::ACCOUNT_LEN, 3888);
        assert_eq!(TICK_ARRAY_RENT_OWNER_OFFSET, 3888);
        assert_eq!(TICK_ARRAY_LEN, 3920);
        assert_eq!(Position::ACCOUNT_LEN, 208);
        assert_eq!(PoolIndexEntry::ACCOUNT_LEN, 56);
        // The Carpenter Platform fee's treasury (the same 112 B as FLOOR's `PlatformTreasury`).
        assert_eq!(ProtocolTreasury::ACCOUNT_LEN, 112);
    }

    /// Rent at `(128 + len) × 5,080` lamports for the lengths above. (Rent is read at runtime by
    /// the programs, never this.)
    #[test]
    fn rent_figures_are_consistent_with_the_lengths() {
        let rent = |len: usize| (128 + len as u64) * 5_080;
        assert_eq!(rent(AmmConfig::ACCOUNT_LEN), 1_747_520);
        assert_eq!(rent(ApprovedHook::ACCOUNT_LEN), 1_666_240);
        assert_eq!(rent(MintExclusivity::ACCOUNT_LEN), 1_056_640);
        assert_eq!(rent(MintPoolCount::ACCOUNT_LEN), 975_360);
        assert_eq!(rent(Pool::ACCOUNT_LEN), 3_901_440);
        // An array is its body + the 32-byte rent owner.
        assert_eq!(rent(TICK_ARRAY_LEN), 20_563_840);
        assert_eq!(rent(Position::ACCOUNT_LEN), 1_706_880);
        assert_eq!(rent(PoolIndexEntry::ACCOUNT_LEN), 934_720);
    }
    #[test]
    fn bitmap_sizes() {
        assert_eq!(bitmap_arrays_per_side(60), 124);
        assert_eq!(bitmap_word_count(60), 4);
        assert_eq!(bitmap_account_len(60), 80);
        assert_eq!(bitmap_account_len(10), 240);
        assert_eq!(bitmap_account_len(1), 1_904);
        assert_eq!((128 + bitmap_account_len(60) as u64) * 5_080, 1_056_640);
        assert_eq!((128 + bitmap_account_len(1) as u64) * 5_080, 10_322_560);
        // every bit fits: the lowest and highest array starts map inside 0..64·N
        for s in [1u16, 10, 60, 200, 32_767] {
            let lo = tick_array_start_index(crate::constants::MIN_TICK, s);
            let hi = tick_array_start_index(crate::constants::MAX_TICK, s);
            assert!(bitmap_bit(lo, s) >= 0, "spacing {s}");
            assert!((bitmap_bit(hi, s) as usize) < 64 * bitmap_word_count(s), "spacing {s}");
        }
    }

    #[test]
    fn tick_array_start_floors_toward_negative_infinity() {
        assert_eq!(tick_array_start_index(0, 60), 0);
        assert_eq!(tick_array_start_index(3599, 60), 0);
        assert_eq!(tick_array_start_index(3600, 60), 3600);
        assert_eq!(tick_array_start_index(-1, 60), -3600);
        assert_eq!(tick_array_start_index(-3600, 60), -3600);
        assert_eq!(tick_array_start_index(-3601, 60), -7200);
    }

    #[test]
    fn position_owner_is_at_account_offset_48() {
        let f = Position::FIELDS.iter().find(|f| f.name == "owner").unwrap();
        assert_eq!(8 + f.offset, 48);
    }

    /// The id is a field of its own, and it is NOT where the owner sits.
    #[test]
    fn position_id_is_at_account_offset_144_and_is_not_the_owner() {
        let id = Position::FIELDS.iter().find(|f| f.name == "position_id").unwrap();
        assert_eq!((8 + id.offset, id.size), (144, 32));
        let mut p = Position::zeroed();
        p.owner = [7u8; 32];
        p.position_id = [9u8; 32];
        let b = bytemuck::bytes_of(&p);
        assert_eq!(&b[40..72], &[7u8; 32]);
        assert_eq!(&b[136..168], &[9u8; 32]);
    }

    /// `Position` keeps a forward-compatibility tail like every other account.
    #[test]
    fn position_still_has_a_reserved_tail() {
        let r = Position::FIELDS.iter().find(|f| f.name == "reserved").expect("Position keeps a reserve");
        assert_eq!((8 + r.offset, r.size), (176, 32));
        assert_eq!(Position::SIZE, 200);
        // Every account carries one.
        for fields in [AmmConfig::FIELDS, ApprovedHook::FIELDS, Pool::FIELDS, Position::FIELDS, PoolIndexEntry::FIELDS, ProtocolTreasury::FIELDS] {
            assert!(fields.iter().any(|f| f.name == "reserved"));
        }
    }

    /// The admitted base transfer hook fields sit at `ApprovedHook.base_transfer_hook` @129,
    /// `Pool.base_transfer_hook` @505 and `Pool.base_hook_side` @537 (account offsets 137 / 513 / 545).
    /// A client (the SDK's `layouts.ts`, an indexer's decoder, a router adapter) reads them at THESE offsets.
    #[test]
    fn the_admitted_base_transfer_hook_is_carved_from_the_reserve_tails() {
        let off = |fields: &[Field], name: &str| {
            let f = fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name}"));
            (f.offset, f.size)
        };
        assert_eq!(off(ApprovedHook::FIELDS, "allow_exclusivity"), (128, 1), "an earlier field did not move");
        assert_eq!(off(ApprovedHook::FIELDS, "base_transfer_hook"), (129, 32));
        // the guardian pause clock is its back 8 bytes; `allow_hook_quotes` the byte after `base_transfer_hook`
        assert_eq!(off(ApprovedHook::FIELDS, "allow_hook_quotes"), (161, 1));
        assert_eq!(off(ApprovedHook::FIELDS, "reserved"), (162, 22));
        assert_eq!(ApprovedHook::SIZE, 192);
        assert_eq!(off(Pool::FIELDS, "paused"), (504, 1), "an earlier field did not move");
        assert_eq!(off(Pool::FIELDS, "base_transfer_hook"), (505, 32));
        assert_eq!(off(Pool::FIELDS, "base_hook_side"), (537, 1));
        // `protocol_fee_currency` is the next byte (the test below); the guardian pause clock the back 8
        assert_eq!(off(Pool::FIELDS, "reserved"), (539, 85));
        assert_eq!(Pool::SIZE, 632);
        // written bytes land at the account offsets clients read
        let mut p = Pool::zeroed();
        p.base_transfer_hook = [0x5A; 32];
        p.base_hook_side = 1;
        let b = bytemuck::bytes_of(&p);
        assert_eq!(&b[505..537], &[0x5A; 32]);
        assert_eq!(b[537], 1);
        let mut a = ApprovedHook::zeroed();
        a.base_transfer_hook = [0x3C; 32];
        assert_eq!(&bytemuck::bytes_of(&a)[129..161], &[0x3C; 32]);
        // an all-zero record reads "nothing admitted"
        assert_eq!(ApprovedHook::zeroed().base_transfer_hook, [0u8; 32]);
    }

    /// The Carpenter Platform fee fields: `AmmConfig.pad2` @177, `protocol_fee` @180 (account 188),
    /// `reserved [u8;24]` @184; `Pool.protocol_fee_currency` @538 (account 546), `reserved [u8;85]` @539.
    /// A client (the SDK's `layouts.ts`, an indexer's decoder) reads them at THESE offsets. A zeroed
    /// pool reads "the input rule".
    #[test]
    fn the_protocol_fee_setting_and_currency_are_carved_from_the_reserve_tails() {
        let off = |fields: &[Field], name: &str| {
            let f = fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name}"));
            (f.offset, f.size)
        };
        assert_eq!(off(AmmConfig::FIELDS, "quote_caps_admitted"), (176, 1), "an earlier field did not move");
        assert_eq!(off(AmmConfig::FIELDS, "pad2"), (177, 3));
        assert_eq!(off(AmmConfig::FIELDS, "protocol_fee"), (180, 4));
        assert_eq!(off(AmmConfig::FIELDS, "reserved"), (184, 24));
        assert_eq!(AmmConfig::SIZE, 208);
        assert_eq!(off(Pool::FIELDS, "protocol_fee"), (16, 4), "the v4 slot did not move");
        assert_eq!(off(Pool::FIELDS, "protocol_fee_currency"), (538, 1));
        assert_eq!(off(Pool::FIELDS, "reserved"), (539, 85));

        let mut c = AmmConfig::zeroed();
        c.protocol_fee = crate::constants::PROTOCOL_FEE_DEFAULT_PACKED;
        let b = bytemuck::bytes_of(&c);
        assert_eq!(&b[180..184], &2_048_500u32.to_le_bytes());
        let mut p = Pool::zeroed();
        p.protocol_fee_currency = crate::constants::PROTOCOL_FEE_ON_CURRENCY0;
        assert_eq!(bytemuck::bytes_of(&p)[538], 1);

        assert_eq!(Pool::zeroed().protocol_fee_currency, crate::constants::PROTOCOL_FEE_ON_INPUT);
    }

    /// The pause guardian fields. A client (the SDK's `layouts.ts`, an indexer's decoder) reads them at THESE
    /// account offsets: `AmmConfig.pause_guardian` @112, `ApprovedHook.guardian_paused_until` @192,
    /// `Pool.guardian_paused_until` @632.
    #[test]
    fn the_pause_guardian_is_carved_from_dead_and_reserved_bytes() {
        let off = |fields: &[Field], name: &str| {
            let f = fields.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name}"));
            (f.offset, f.size)
        };
        assert_eq!(off(AmmConfig::FIELDS, "protocol_fee_controller"), (72, 32), "an earlier field did not move");
        assert_eq!(off(AmmConfig::FIELDS, "pause_guardian"), (104, 32));
        assert_eq!(off(AmmConfig::FIELDS, "pad1"), (136, 40));
        assert_eq!(off(AmmConfig::FIELDS, "quote_caps_admitted"), (176, 1), "a later field did not move");
        assert_eq!(AmmConfig::SIZE, 208);
        assert_eq!(off(ApprovedHook::FIELDS, "guardian_paused_until"), (184, 8));
        assert_eq!(ApprovedHook::SIZE, 192);
        assert_eq!(off(Pool::FIELDS, "protocol_fee_currency"), (538, 1), "an earlier field did not move");
        assert_eq!(off(Pool::FIELDS, "guardian_paused_until"), (624, 8));
        assert_eq!(Pool::SIZE, 632);
        let mut p = Pool::zeroed();
        p.guardian_paused_until = 0x0102_0304_0506_0708;
        assert_eq!(&bytemuck::bytes_of(&p)[624..632], &0x0102_0304_0506_0708i64.to_le_bytes());
        let mut c = AmmConfig::zeroed();
        c.pause_guardian = [0x6A; 32];
        assert_eq!(&bytemuck::bytes_of(&c)[104..136], &[0x6A; 32]);
    }

    /// A hook-listed quote records its hook at struct offset 88 (account 96); `QuoteMint` is 120 B.
    #[test]
    fn a_hook_listed_quote_records_its_hook_in_the_old_reserve() {
        let f = QuoteMint::FIELDS.iter().find(|f| f.name == "hook_program").expect("hook_program");
        assert_eq!((f.offset, f.size), (88, 32));
        assert_eq!(QuoteMint::SIZE, 120);
        assert!(QuoteMint::FIELDS.iter().all(|f| f.name != "reserved"));
        assert_eq!(QUOTE_STATE_HOOK_LISTED, 3);
        assert_ne!(QUOTE_STATE_HOOK_LISTED, QUOTE_STATE_LISTED);
        assert_ne!(QUOTE_STATE_HOOK_LISTED, QUOTE_STATE_RETIRED);
    }

    /// The protocol treasury's two keys sit at the account offsets a client
    /// (the SDK, a keeper, an indexer) reads — governance @16, pending @48 — and it is FLOOR's
    /// `PlatformTreasury` shape byte for byte (`floor_types::layout`), so one decoder reads both.
    #[test]
    fn the_protocol_treasury_keys_sit_at_their_account_offsets() {
        let off = |name: &str| {
            let f = ProtocolTreasury::FIELDS.iter().find(|f| f.name == name).unwrap_or_else(|| panic!("{name}"));
            (8 + f.offset, f.size)
        };
        assert_eq!(off("governance"), (16, 32));
        assert_eq!(off("pending_governance"), (48, 32));
        assert_eq!(off("reserved"), (80, 32));
        let mut t = ProtocolTreasury::zeroed();
        t.governance = [0x11; 32];
        t.pending_governance = [0x22; 32];
        let b = bytemuck::bytes_of(&t);
        assert_eq!(&b[8..40], &[0x11; 32]);
        assert_eq!(&b[40..72], &[0x22; 32]);
        assert_ne!(ProtocolTreasury::DISCRIMINATOR, AmmConfig::DISCRIMINATOR);
    }

    /// The header is the first eight bytes after the discriminator of ANY hook's `hook_config`,
    /// read as raw bytes — this crate has no `HookConfig` to build one from, and that is the
    /// point: the AMM reads the header without knowing the hook (`floor_types::layout` pins
    /// FLOOR's own `HookConfig` to it).
    #[test]
    fn hook_interface_header_is_the_first_eight_bytes_after_the_discriminator() {
        let mut data = vec![0u8; 8];
        data.extend_from_slice(&[1, 254, 3, 1, 0xCC, 0x60, 253, 0]);
        data.extend_from_slice(&[0xAA; 224]);
        let h = HookInterfaceHeader::from_account_data(&data).unwrap();
        assert_eq!(
            h,
            HookInterfaceHeader { version: 1, bump: 254, max_rounds: 3, callback_accounts: 1, flags: 0x60CC, authority_bump: 253, pad0: 0 }
        );
        assert!(HookInterfaceHeader::from_account_data(&data[..15]).is_none());
    }
    #[test]
    fn u128_fields_write_little_endian_at_their_offsets() {
        let mut p = Pool::zeroed();
        p.sqrt_price_x64 = U128::new(crate::constants::MAX_SQRT_PRICE_X64);
        p.hook_claims1 = 0xDEAD_BEEF;
        let bytes = bytemuck::bytes_of(&p);
        assert_eq!(&bytes[280..296], &crate::constants::MAX_SQRT_PRICE_X64.to_le_bytes());
        assert_eq!(&bytes[368..376], &0xDEAD_BEEFu64.to_le_bytes());
    }
}
