//! `carpenter-types`: the bytes the Carpenter AMM, every hook that plugs into it, and every
//! harness and client agree on. The FLOOR mechanism's own bytes are `floor-types`, which DEPENDS
//! on this crate and never the other way round.
//!
//! - [`ids`]: the AMM's program id and the test-only mock hook's.
//! - [`keys`]: well-known foreign program and mint keys (wSOL, SPL Token, Token-2022, …).
//! - [`seeds`]: the AMM's PDA seed prefixes, the two seeds of the hook INTERFACE (`hook_config`,
//!   `hook_authority`), and the small pure helpers that build seed bytes.
//! - [`constants`]: AMM constants (tick and price bounds, fee limits, the protocol fee) and the hook flag bits.
//! - [`pod`]: `U128` / `I128` as `[u64; 2]` little-endian.
//! - [`layout`]: every AMM account struct, `#[repr(C)]` + `Pod`, with size and offset asserts, the
//!   [`layout::Layout`] trait and the [`layout!`] macro, and the 8-byte [`layout::HookInterfaceHeader`]
//!   the AMM reads under ANY approved hook.
//! - [`hook`]: the FROZEN hook interface and the pure record rules.
//! - [`protocol_fee`]: the Carpenter Platform fee (v4's protocol fee) as
//!   pure arithmetic that BOTH walks run — the AMM program's and the quoter's.
//! - [`disc`]: the AMM's instruction discriminators, the hook CALLBACK discriminators, and the
//!   AMM's account discriminators, as bytes.
//!
//! No Solana crate, ever. Keys are `[u8; 32]`.
//!
//! # The boundary, as compile failures
//!
//! No FLOOR seed, layout, id or constant lives here. `cargo test` compiles each block below and
//! requires it to FAIL (`compile_fail` doctests): the day one of these names is added to this
//! crate — or re-exported into it — its block compiles and the test goes red. The other half of the
//! boundary, that the AMM crates link only THIS crate and never `floor-types`, is
//! `crates/carpenter-types/tests/boundary.rs` (a manifest and source guard).
//!
//! ```compile_fail
//! let _ = carpenter_types::seeds::FLOOR_POOL;
//! ```
//! ```compile_fail
//! let _ = carpenter_types::seeds::LAUNCH;
//! ```
//! ```compile_fail
//! let _ = core::mem::size_of::<carpenter_types::layout::FloorPool>();
//! ```
//! ```compile_fail
//! let _ = core::mem::size_of::<carpenter_types::layout::Launch>();
//! ```
//! ```compile_fail
//! let _ = carpenter_types::constants::SUPPLY;
//! ```
//! ```compile_fail
//! let _ = carpenter_types::constants::FLOOR_HOOK_FLAGS;
//! ```
//! ```compile_fail
//! let _ = carpenter_types::ids::FLOOR_HOOK_ID;
//! ```
//! ```compile_fail
//! let _ = carpenter_types::disc::account::FLOOR_POOL;
//! ```
//!
//! And the positive control, so a doctest that cannot compile for an unrelated reason is not
//! mistaken for the boundary holding:
//!
//! ```
//! assert_eq!(carpenter_types::seeds::POOL, b"pool");
//! assert_eq!(carpenter_types::seeds::HOOK_CONFIG, b"hook_config");
//! ```
#![forbid(unsafe_code)]

pub mod constants;
pub mod disc;
pub mod hook;
pub mod ids;
pub mod keys;
pub mod layout;
pub mod pod;
pub mod protocol_fee;
pub mod seeds;

pub use pod::{I128, U128};
