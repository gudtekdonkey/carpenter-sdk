//! The AMM's PDA seed prefixes, the two seeds of the hook INTERFACE, and the pure helpers that
//! build seed bytes.
//!
//! A seed is a byte string, not a `str`: `b"pool"`, never `"pool".as_bytes()` in one place
//! and a differently spelled literal in another. Every program and client takes them from here.
//! Integers in seeds are LITTLE-endian (`fee u32 LE`, `tick_spacing u16 LE`, `salt u64 LE`,
//! `start_index i32 LE`, `index u64 LE`, `nonce u64 LE`).
//! No FLOOR seed is here — `floor_pool`, `launch`, `buyer`, `mint_authority` and the rest of the
//! hook's seeds are `floor_types::seeds`, which re-exports this module beside them. The AMM program
//! links only this crate, so a FLOOR seed cannot be spelled from it.

// ---- carpenter_amm ---------------------------------------------------------------------
/// `["amm_config"]`
pub const AMM_CONFIG: &[u8] = b"amm_config";
/// `["approved_hook", hook_program]`
pub const APPROVED_HOOK: &[u8] = b"approved_hook";
/// `["mint_excl", mint]`
pub const MINT_EXCLUSIVITY: &[u8] = b"mint_excl";
/// `["mint_pools", mint]`
pub const MINT_POOLS: &[u8] = b"mint_pools";
/// `["quote_mint", mint]` — the quote ROSTER entry, one account per listed quote mint.
pub const QUOTE_MINT: &[u8] = b"quote_mint";
/// `["pool", mint0, mint1, fee u32 LE, tick_spacing u16 LE, hook]`
pub const POOL: &[u8] = b"pool";
/// `["bitmap", pool]`
pub const BITMAP: &[u8] = b"bitmap";
/// `["tick_array", pool, start_index i32 LE]`
pub const TICK_ARRAY: &[u8] = b"tick_array";
/// `["position", pool, position_id, salt u64 LE]`.
/// The third seed is the position's ID, NEVER its owner: the owner is a mutable FIELD, so a
/// position can change hands. A caller may pass its own key as the id; the FLOOR hook's positions
/// use `position_id = hook_authority`, told apart by their salts.
pub const POSITION: &[u8] = b"position";
/// `["pool_index", pool]`
pub const POOL_INDEX: &[u8] = b"pool_index";
/// `["vault", pool, mint]`
pub const VAULT: &[u8] = b"vault";
/// `["protocol_treasury"]` — the Carpenter Platform fee's treasury: the ONE
/// program-owned account whose ADDRESS owns every token account a pool's `protocol_fees_owed` is
/// swept into (`ATA(this, mint, token program)`, one per mint), and whose DATA is the governance
/// that alone may withdraw from them and set the protocol fee new pools copy. Not a FLOOR seed:
/// the AMM's own treasury, distinct from `floor_hook`'s `["platform_treasury"]`.
pub const PROTOCOL_TREASURY: &[u8] = b"protocol_treasury";
/// `["__event_authority"]`, Anchor's `emit_cpi!` signer, in every program.
pub const EVENT_AUTHORITY: &[u8] = b"__event_authority";

// ---- the hook interface — under ANY approved hook program ------------------------------
/// `["hook_config"]`. The AMM reads the 8-byte interface header of the account at this
/// seed under ANY approved hook program: `approve_hook` / `reapprove_hook` derive it.
pub const HOOK_CONFIG: &[u8] = b"hook_config";
/// `["hook_authority"]` under the hook program. One per hook program; the AMM derives it
/// at `approve_hook` and every hook-initiated instruction is signed by it.
pub const HOOK_AUTHORITY: &[u8] = b"hook_authority";
/// `["hook_settle", pool]` under the AMM (`SETTLES_FLAG`): the signer the AMM invokes a
/// settling hook with, in place of the pool PDA. It owns nothing and holds no authority over any
/// account, so a settling hook may be handed the pool's vaults writable — nothing can debit them
/// without the pool's signature, which this call never carries. The hook authenticates the AMM by it.
pub const HOOK_SETTLE: &[u8] = b"hook_settle";

// ---- seed-byte helpers ------------------------------------------------------------------

/// The integer seeds of a pool PDA (fee, tick spacing), as LE bytes. `mint0 < mint1` is the caller's
/// check, not this function's.
pub fn pool_seed_parts(fee: u32, tick_spacing: u16) -> ([u8; 4], [u8; 2]) {
    (fee.to_le_bytes(), tick_spacing.to_le_bytes())
}

/// `tick_array` seed bytes for a start index.
pub fn tick_array_seed(start_index: i32) -> [u8; 4] {
    start_index.to_le_bytes()
}

/// `position` / `entry` / `mint` seed bytes for a u64.
pub fn u64_seed(v: u64) -> [u8; 8] {
    v.to_le_bytes()
}

/// Are two mints in pool order (`mint0 < mint1`, bytewise)?
pub fn mints_sorted(mint0: &[u8; 32], mint1: &[u8; 32]) -> bool {
    mint0 < mint1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_the_expected_strings() {
        assert_eq!(AMM_CONFIG, b"amm_config");
        assert_eq!(MINT_EXCLUSIVITY, b"mint_excl");
        assert_eq!(HOOK_CONFIG, b"hook_config");
        assert_eq!(EVENT_AUTHORITY, b"__event_authority");
        assert_eq!(PROTOCOL_TREASURY, b"protocol_treasury");
    }

    #[test]
    fn integers_are_little_endian() {
        assert_eq!(pool_seed_parts(0x80_0000, 60), ([0x00, 0x00, 0x80, 0x00], [60, 0]));
        assert_eq!(tick_array_seed(-3600), (-3600i32).to_le_bytes());
        assert_eq!(u64_seed(1), [1, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn mints_sort_bytewise_above_wsol() {
        let mut above = crate::keys::WSOL_MINT;
        above[31] = 2;
        assert!(mints_sorted(&crate::keys::WSOL_MINT, &above));
        assert!(!mints_sorted(&above, &crate::keys::WSOL_MINT));
    }
}
