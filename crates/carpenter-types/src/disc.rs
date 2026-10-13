//! Anchor discriminators, as bytes, so a CPI never needs the callee's crate.
//! GENERATED once from the instruction and account names (sha256 prefix, the Anchor default);
//! the program repository's layout test checks every one against the built IDLs, so a rename cannot drift.
//!
//! Three tables: the AMM's instructions ([`amm`]), the hook CALLBACKS every approved hook answers
//! to ([`callback`]), and the AMM's accounts ([`account`]). `floor_hook`'s and `floor_launch`'s
//! own instructions and accounts are `floor_types::disc`, which re-exports these beside them.
//! A discriminator hashes the INSTRUCTION or ACCOUNT name (`global:<ix>`, `account:<Name>`),
//! never the program's, so a program rename moves no byte here.

/// `carpenter_amm` instructions.
pub mod amm {
    /// `sha256("global:initialize_config")[..8]`
    pub const INITIALIZE_CONFIG: [u8; 8] = [0xd0, 0x7f, 0x15, 0x01, 0xc2, 0xbe, 0xc4, 0x46];
    /// `sha256("global:set_config_authority")[..8]`
    pub const SET_CONFIG_AUTHORITY: [u8; 8] = [0x10, 0xc8, 0xd4, 0x12, 0x5f, 0x2b, 0x6b, 0x59];
    /// `sha256("global:accept_config_authority")[..8]`
    pub const ACCEPT_CONFIG_AUTHORITY: [u8; 8] = [0x8a, 0x22, 0x82, 0x10, 0xfc, 0xcf, 0x18, 0x5a];
    /// `sha256("global:approve_hook")[..8]`
    pub const APPROVE_HOOK: [u8; 8] = [0x47, 0xe1, 0xa9, 0xda, 0x90, 0x99, 0x52, 0x2e];
    /// `sha256("global:set_hook_paused")[..8]`
    pub const SET_HOOK_PAUSED: [u8; 8] = [0x36, 0x2d, 0xde, 0xfe, 0xa9, 0xac, 0x70, 0x89];
    /// `sha256("global:reapprove_hook")[..8]`
    pub const REAPPROVE_HOOK: [u8; 8] = [0x9f, 0x93, 0xa8, 0xa4, 0x4c, 0xb6, 0x70, 0x3b];
    /// `sha256("global:set_base_transfer_hook")[..8]` (the ONE armed transfer-hook program a hook's pools may carry on the base
    /// mint, set once on `ApprovedHook.base_transfer_hook` by the config authority)
    pub const SET_BASE_TRANSFER_HOOK: [u8; 8] = [0x8e, 0x53, 0xf5, 0xea, 0xa8, 0xf2, 0x02, 0xcf];
    // wSOL is the implied quote; every other quote is on the roster (`list_quote_mint`).
    /// `sha256("global:list_quote_mint")[..8]`
    pub const LIST_QUOTE_MINT: [u8; 8] = [0x64, 0x9b, 0x7a, 0x62, 0x74, 0x58, 0x39, 0x99];
    /// `sha256("global:retire_quote_mint")[..8]`
    pub const RETIRE_QUOTE_MINT: [u8; 8] = [0x5d, 0x39, 0xad, 0x30, 0x76, 0x9f, 0xf1, 0x1d];
    /// `sha256("global:delist_quote_mint")[..8]`
    pub const DELIST_QUOTE_MINT: [u8; 8] = [0x56, 0xe0, 0x27, 0x54, 0x97, 0xa5, 0x05, 0xd1];
    /// `sha256("global:relist_quote_mint")[..8]` (the way
    /// back from `retire`)
    pub const RELIST_QUOTE_MINT: [u8; 8] = [0x92, 0xfe, 0xd4, 0x0b, 0x62, 0x05, 0x1b, 0x0e];
    /// `sha256("global:set_quote_caps_admitted")[..8]` (the governance
    /// admission mask for listed quotes, `AmmConfig.quote_caps_admitted`)
    pub const SET_QUOTE_CAPS_ADMITTED: [u8; 8] = [0xa2, 0x7b, 0x4c, 0x23, 0xec, 0x26, 0xd5, 0xfa];
    /// `sha256("global:set_pool_paused")[..8]` (the per-pool pause)
    pub const SET_POOL_PAUSED: [u8; 8] = [0xea, 0x77, 0x48, 0xb3, 0xa9, 0x65, 0x6e, 0x5d];
    /// `sha256("global:set_pause_guardian")[..8]` (the bounded pause's key)
    pub const SET_PAUSE_GUARDIAN: [u8; 8] = [0xc0, 0x52, 0xc1, 0xd8, 0x1a, 0x16, 0x0a, 0x75];
    /// `sha256("global:guardian_pause_pool")[..8]` (a pool paused ≤ 72 h by the guardian)
    pub const GUARDIAN_PAUSE_POOL: [u8; 8] = [0xab, 0x03, 0x14, 0xfd, 0x91, 0xb5, 0x66, 0xc1];
    /// `sha256("global:guardian_pause_hook")[..8]` (a hook's new pools paused ≤ 72 h by the guardian)
    pub const GUARDIAN_PAUSE_HOOK: [u8; 8] = [0xc8, 0x4f, 0xac, 0x17, 0xe4, 0x7c, 0x11, 0xb7];
    /// `sha256("global:set_hook_quotes")[..8]` (the config authority lets an approved hook list
    /// quote mints for its own pools)
    pub const SET_HOOK_QUOTES: [u8; 8] = [0xb4, 0xcc, 0xac, 0x70, 0x8d, 0x9f, 0x31, 0x89];
    /// `sha256("global:hook_list_quote_mint")[..8]` (a hook-listed quote, `QUOTE_STATE_HOOK_LISTED`)
    pub const HOOK_LIST_QUOTE_MINT: [u8; 8] = [0x36, 0x31, 0x70, 0x02, 0x49, 0x80, 0xc4, 0xc5];
    /// `sha256("global:initialize_pool")[..8]`
    pub const INITIALIZE_POOL: [u8; 8] = [0x5f, 0xb4, 0x0a, 0xac, 0x54, 0xae, 0xe8, 0x28];
    /// `sha256("global:initialize_tick_array")[..8]`
    pub const INITIALIZE_TICK_ARRAY: [u8; 8] = [0x0b, 0xbc, 0xc1, 0xd6, 0x8d, 0x5b, 0x95, 0xb8];
    /// `sha256("global:initialize_position")[..8]`
    pub const INITIALIZE_POSITION: [u8; 8] = [0xdb, 0xc0, 0xea, 0x47, 0xbe, 0xbf, 0x66, 0x50];
    /// `sha256("global:modify_liquidity")[..8]`
    pub const MODIFY_LIQUIDITY: [u8; 8] = [0x0e, 0xd6, 0xa8, 0x29, 0x37, 0x0f, 0x35, 0x11];
    /// `sha256("global:close_position")[..8]`
    pub const CLOSE_POSITION: [u8; 8] = [0x7b, 0x86, 0x51, 0x00, 0x31, 0x44, 0x62, 0x62];
    /// `sha256("global:close_tick_array")[..8]` (refunds the array's rent owner)
    pub const CLOSE_TICK_ARRAY: [u8; 8] = [0xb9, 0x0a, 0x31, 0x50, 0x41, 0x70, 0xcd, 0xc7];
    /// `sha256("global:transfer_position")[..8]`
    pub const TRANSFER_POSITION: [u8; 8] = [0x8b, 0x82, 0x66, 0x93, 0x87, 0x4d, 0x71, 0xde];
    /// `sha256("global:swap")[..8]`
    pub const SWAP: [u8; 8] = [0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8];
    /// `sha256("global:swap_from_balance")[..8]` (`swap`'s exact-in with
    /// the amount resolved on chain as `trader_token_in.amount - keep`; the same accounts as `swap`)
    pub const SWAP_FROM_BALANCE: [u8; 8] = [0xcc, 0xa0, 0xe9, 0x06, 0x87, 0xf9, 0x47, 0x8a];
    /// `sha256("global:donate")[..8]`
    pub const DONATE: [u8; 8] = [0x79, 0xba, 0xda, 0xd3, 0x49, 0x46, 0xc4, 0xb4];
    /// `sha256("global:hook_deposit")[..8]`
    pub const HOOK_DEPOSIT: [u8; 8] = [0xd9, 0x45, 0x81, 0x55, 0xd6, 0xbe, 0xf6, 0xb3];
    /// `sha256("global:hook_withdraw")[..8]`
    pub const HOOK_WITHDRAW: [u8; 8] = [0xbd, 0xc4, 0xc4, 0x6a, 0x5d, 0xd3, 0x90, 0x28];
    /// `sha256("global:register_mint_exclusivity")[..8]`
    pub const REGISTER_MINT_EXCLUSIVITY: [u8; 8] = [0x9b, 0x74, 0xed, 0x39, 0x49, 0x9f, 0x21, 0xd7];
    /// `sha256("global:revoke_mint_exclusivity")[..8]`
    pub const REVOKE_MINT_EXCLUSIVITY: [u8; 8] = [0x4d, 0xa4, 0x09, 0xc7, 0xb3, 0x21, 0xa1, 0x77];
    /// `sha256("global:update_dynamic_lp_fee")[..8]`
    pub const UPDATE_DYNAMIC_LP_FEE: [u8; 8] = [0x7c, 0xf4, 0xc7, 0xa4, 0x87, 0x47, 0xb5, 0x3c];

    // ---- the protocol treasury (the Carpenter Platform fee) ----
    /// `sha256("global:initialize_protocol_treasury")[..8]`
    pub const INITIALIZE_PROTOCOL_TREASURY: [u8; 8] = [0x7d, 0x94, 0x35, 0xc5, 0x88, 0x51, 0x41, 0x3a];
    /// `sha256("global:sweep_protocol_fees")[..8]` (permissionless: `protocol_fees_owed` → the treasury)
    pub const SWEEP_PROTOCOL_FEES: [u8; 8] = [0x75, 0xe1, 0xf0, 0x4b, 0x47, 0xc3, 0x2c, 0x2b];
    /// `sha256("global:withdraw_protocol_fees")[..8]` (governance only, uncapped)
    pub const WITHDRAW_PROTOCOL_FEES: [u8; 8] = [0x0b, 0x44, 0xa5, 0x62, 0x12, 0xd0, 0x86, 0x49];
    /// `sha256("global:set_protocol_treasury_governance")[..8]` (handover, step one)
    pub const SET_PROTOCOL_TREASURY_GOVERNANCE: [u8; 8] = [0xa9, 0x17, 0x66, 0x9b, 0xa9, 0x65, 0xa7, 0xdd];
    /// `sha256("global:accept_protocol_treasury_governance")[..8]` (handover, step two)
    pub const ACCEPT_PROTOCOL_TREASURY_GOVERNANCE: [u8; 8] = [0x41, 0x00, 0xaf, 0xe8, 0xc2, 0x34, 0x56, 0xc0];
    /// `sha256("global:set_protocol_fee")[..8]` (governance only; the rate NEW pools copy, capped)
    pub const SET_PROTOCOL_FEE: [u8; 8] = [0xad, 0xef, 0x53, 0xf2, 0x88, 0x2b, 0x90, 0xd9];
}

/// Hook callbacks. An approved hook names its handlers exactly so.
pub mod callback {
    /// `sha256("global:before_initialize")[..8]`
    pub const BEFORE_INITIALIZE: [u8; 8] = [0x22, 0x5c, 0xa3, 0x2c, 0xbb, 0xd5, 0x83, 0x0f];
    /// `sha256("global:after_initialize")[..8]`
    pub const AFTER_INITIALIZE: [u8; 8] = [0x82, 0xc2, 0xbc, 0x09, 0x22, 0x06, 0x47, 0x8d];
    /// `sha256("global:before_add_liquidity")[..8]`
    pub const BEFORE_ADD_LIQUIDITY: [u8; 8] = [0x72, 0x1b, 0x7b, 0xb5, 0xc8, 0xfc, 0xb2, 0xf0];
    /// `sha256("global:after_add_liquidity")[..8]`
    pub const AFTER_ADD_LIQUIDITY: [u8; 8] = [0x7c, 0x18, 0xa8, 0x6e, 0x8f, 0x38, 0x23, 0x0e];
    /// `sha256("global:before_remove_liquidity")[..8]`
    pub const BEFORE_REMOVE_LIQUIDITY: [u8; 8] = [0x42, 0x81, 0x8b, 0x3f, 0xc4, 0x54, 0xe7, 0xdd];
    /// `sha256("global:after_remove_liquidity")[..8]`
    pub const AFTER_REMOVE_LIQUIDITY: [u8; 8] = [0xee, 0x8a, 0x1c, 0xdc, 0x23, 0xa7, 0x08, 0x2a];
    /// `sha256("global:before_swap")[..8]`
    pub const BEFORE_SWAP: [u8; 8] = [0xe3, 0x3b, 0xf0, 0x44, 0xa4, 0x09, 0x1d, 0xfe];
    /// `sha256("global:after_swap")[..8]`
    pub const AFTER_SWAP: [u8; 8] = [0xeb, 0xd7, 0xe8, 0xb7, 0x98, 0x6d, 0x05, 0x23];
    /// `sha256("global:before_donate")[..8]`
    pub const BEFORE_DONATE: [u8; 8] = [0x76, 0xb7, 0x34, 0x5f, 0xb7, 0xd5, 0x56, 0xd2];
    /// `sha256("global:after_donate")[..8]`
    pub const AFTER_DONATE: [u8; 8] = [0x0a, 0x9a, 0x6a, 0x06, 0x90, 0xa0, 0x92, 0x02];
    /// `sha256("global:after_actions")[..8]`
    pub const AFTER_ACTIONS: [u8; 8] = [0x61, 0x96, 0xce, 0x60, 0x91, 0x02, 0x24, 0x00];
}

/// Account discriminators, `sha256("account:<Name>")[..8]`.
pub mod account {
    /// `AmmConfig` (`carpenter_amm`)
    pub const AMM_CONFIG: [u8; 8] = [0xda, 0xf4, 0x21, 0x68, 0xcb, 0xcb, 0x2b, 0x6f];
    /// `ApprovedHook` (`carpenter_amm`)
    pub const APPROVED_HOOK: [u8; 8] = [0x1b, 0x9b, 0x38, 0x21, 0xb0, 0xad, 0xe1, 0x0f];
    /// `MintExclusivity` (`carpenter_amm`)
    pub const MINT_EXCLUSIVITY: [u8; 8] = [0x3a, 0x90, 0x07, 0x22, 0x47, 0xe3, 0x02, 0x37];
    /// `MintPoolCount` (`carpenter_amm`)
    pub const MINT_POOL_COUNT: [u8; 8] = [0xd7, 0x9e, 0xe2, 0xc8, 0x59, 0x05, 0x63, 0x6c];
    /// `QuoteMint` (`carpenter_amm`) — the quote roster entry
    pub const QUOTE_MINT: [u8; 8] = [0xf9, 0xdf, 0x0c, 0xd4, 0x7d, 0x1c, 0xb9, 0x67];
    /// `Pool` (`carpenter_amm`)
    pub const POOL: [u8; 8] = [0xf1, 0x9a, 0x6d, 0x04, 0x11, 0xb1, 0x6d, 0xbc];
    /// `TickArrayBitmap` (`carpenter_amm`)
    pub const TICK_ARRAY_BITMAP: [u8; 8] = [0x21, 0xfb, 0x69, 0xf4, 0x05, 0xfc, 0x9b, 0xce];
    /// `TickArray` (`carpenter_amm`)
    pub const TICK_ARRAY: [u8; 8] = [0x45, 0x61, 0xbd, 0xbe, 0x6e, 0x07, 0x42, 0xbb];
    /// `Position` (`carpenter_amm`)
    pub const POSITION: [u8; 8] = [0xaa, 0xbc, 0x8f, 0xe4, 0x7a, 0x40, 0xf7, 0xd0];
    /// `PoolIndexEntry` (`carpenter_amm`)
    pub const POOL_INDEX_ENTRY: [u8; 8] = [0x7a, 0xdd, 0x11, 0x66, 0xaf, 0x30, 0xb3, 0xf2];
    /// `ProtocolTreasury` (`carpenter_amm`), `["protocol_treasury"]`
    pub const PROTOCOL_TREASURY: [u8; 8] = [0xa2, 0x1a, 0x7b, 0x3d, 0x66, 0x92, 0x2f, 0x49];
}
