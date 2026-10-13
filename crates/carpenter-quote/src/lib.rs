//! `carpenter-quote`: the Carpenter AMM's network-free quoter.
//!
//! Hand it account BYTES (a pool, its tick arrays and bitmap) and it replays what the chain will do,
//! to the unit:
//!
//! - [`swap::quote_swap`]: the swap round protocol. A plain pool is the CLMM walk alone. A pool whose
//!   approved hook changes swaps is quoted through that hook's [`HookQuoter`] (`hook.rs`): its
//!   callbacks run at the points the AMM calls them, its records are validated and applied by the
//!   AMM's rules, and the actions it requests (`ModifyPosition`, `Swap`) execute in-process. A hooked
//!   pool with no quoter is refused, never quoted as plain. Exact output at a 100 % fee (`InvalidFeeForExactOut`) is
//!   refused by name.
//! - [`liquidity`]: `amounts_for_liquidity`, `quote_liquidity`, `fees_owed`.
//! - [`price`]: price ↔ tick.
//! - [`state`]: account bytes → layouts, and the [`TickArraySet`] a walk reads.
//! - [`synth`]: synthetic pools from a list of positions, for tests and mocks.
//!
//! FLOOR's hook quoter is the `floor-quote` crate (in `floor-sdk`), which implements [`HookQuoter`].
//!
//! No Solana crate. Nothing here reads a clock, a random source or the network, and no amount
//! passes through a float; the same bytes in give the same bytes out on every target, wasm included.
#![forbid(unsafe_code)]

pub mod error;
pub mod hook;
pub mod liquidity;
pub mod modify;
pub mod price;
pub mod state;
pub mod swap;
pub mod synth;
pub mod walk;

pub use error::{QuoteError, QuoteResult};
pub use hook::{AfterSwapIn, HookQuoter, HookSwapCtx};
pub use liquidity::{amounts_for_liquidity, fees_owed, quote_liquidity};
pub use state::TickArraySet;
pub use swap::{is_swap_hooked, quote_plain, quote_swap, quote_swap_with, PoolAccount, QuoteFees, SwapQuote, SwapQuoteParams};

/// Re-exported so a harness or a hook's crate builds states with the exact math and types the
/// quoter uses.
pub use carpenter_clmm_math as clmm;
pub use carpenter_types as types;

/// Every swap needs a price limit; this is "no limit" in the swap direction (one unit inside the
/// bound, which v4 `Pool.sol:322-337` requires).
pub fn no_limit(zero_for_one: bool) -> u128 {
    if zero_for_one {
        carpenter_types::constants::MIN_SQRT_PRICE_X64 + 1
    } else {
        carpenter_types::constants::MAX_SQRT_PRICE_X64 - 1
    }
}
