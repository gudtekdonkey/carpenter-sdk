//! `carpenter-amm`: a [`jupiter_amm_interface::Amm`] implementation for Carpenter pools.
//!
//! The adapter is a shell over the network-free quoters; it computes no swap amount itself.
//!
//! - [`amm::CarpenterAmm`]: the `Amm` impl. It converts keys between `Pubkey` and `[u8; 32]`, names
//!   the accounts a quote needs, decodes them in `update`, quotes through
//!   [`carpenter_quote::quote_swap_with`] with the hook quoters this crate knows, maps the answer onto
//!   Jupiter's `Quote`, and lists the native `swap` instruction's accounts.
//! - [`hook::Deployment`]: the cluster a pool belongs to, looked up by the pool account's owner among
//!   `carpenter_types::ids::AMM_CLUSTERS` (FLOOR's ids beside it from `floor_types::ids::CLUSTERS`).
//!   One build routes every listed cluster; every program id and PDA comes from the pool's entry.
//! - [`hook::KnownHook`]: how a pool's hook is classified. A pool with no hook, or a hook with no swap
//!   flag, is quoted as plain. A pool hooked by `floor_hook` is quoted through
//!   [`floor_quote::FloorHookQuoter`]. Any other swap hook is unknown: the pool is inactive and every
//!   quote is refused with `QuoteHookNotQuotable`, never answered as if the pool were plain.
//! - [`encode`]: the `Swap` value this adapter emits (`Swap::Placeholder`, or `Swap::Carpenter` with
//!   the `upstream-variant` feature), and the native `swap` instruction data built from it.
//! - [`transfer_hook`]: the extra accounts a Token-2022 transfer hook on the pool's base mint needs,
//!   resolved from the hook's own `extra-account-metas` list.
//!
//! Nothing here reads a clock, the network, a file or a random source.
#![forbid(unsafe_code)]

pub mod accounts;
pub mod amm;
pub mod convert;
pub mod encode;
pub mod hook;
pub mod transfer_hook;

pub use amm::{BaseHook, CarpenterAmm};
pub use encode::swap_instruction_data;
pub use hook::{known_hook_name, transfer_hook_name, Deployment, HookQuoteErr, KnownHook};

/// `Amm::label()`. Jupiter's test kit derives the program fixture path from the label (lowercased,
/// spaces to underscores), so this one expects `tests/fixtures/carpenter.so`. Keep it stable.
pub const LABEL: jupiter_amm_interface::AmmLabel = "Carpenter";

/// The `swap` instruction's fixed accounts, `trader` through `program`. The hook slice
/// (`Pool.hook_callback_accounts` accounts) follows when a swap callback will run, then the tick
/// arrays.
pub const FIXED_METAS: usize = 14;

/// The hook positions an in-swap action on a FLOOR pool may modify: the bid re-place
/// (`ModifyPosition` −L, the nudge `Swap`, `ModifyPosition` +L) touches the bid position alone.
pub const ACTION_POSITIONS_FLOOR: usize = 1;

/// The most tick arrays one swap may carry, and the size of the window `update` reads. A FLOOR pool
/// whose own positions alone span more arrays than this is reported inactive rather than routed with
/// an account list the transaction could not complete.
pub const MAX_ARRAYS: usize = 12;

/// Initialized arrays read on each side of the current one for the trader's walk when the bitmap
/// marks more than [`MAX_ARRAYS`] arrays. At tick spacing 60 an array spans 3,600 ticks.
pub const TRADER_ARRAYS_PER_SIDE: usize = 3;
