//! How a pool's hook is classified, and the registry of hooks this crate can quote.
//!
//! Nothing simulates a hook program in the quoting path, so a hook that changes swaps can be quoted
//! only through a Rust replay of its callbacks (a [`carpenter_quote::HookQuoter`]). This crate
//! registers one: `floor_hook`, replayed by [`floor_quote::FloorHookQuoter`].
//!
//! | Pool | Classification |
//! |---|---|
//! | no hook, or a hook with neither `BEFORE_SWAP_FLAG` nor `AFTER_SWAP_FLAG` | [`KnownHook::Inert`]: quoted as plain |
//! | `floor_hook` of the cluster that owns the pool, with a swap flag | [`KnownHook::Floor`] |
//! | `floor_hook` of another cluster | refused: [`HookQuoteErr::ClusterMismatch`] |
//! | any other hook with a swap flag | [`KnownHook::Unknown`]: inactive, quotes refused with `QuoteHookNotQuotable` |
//!
//! The plain / hooked split is decided by `Pool.hook_flags` exactly as
//! [`carpenter_quote::is_swap_hooked`] decides it, so the adapter and the quoter never disagree.
//!
//! Every program id comes from the pool's [`Deployment`] (looked up by the pool account's owner):
//! `floor_hook` is FLOOR's hook only on the cluster whose `carpenter_amm` owns the pool. A pool that
//! names another cluster's `floor_hook` is refused ([`HookQuoteErr::ClusterMismatch`]).

use core::fmt;

use carpenter_quote::is_swap_hooked;
use carpenter_types::ids::{AmmClusterIds, AMM_CLUSTERS};
use carpenter_types::layout::Pool;
use floor_types::constants::FLOOR_CALLBACK_ACCOUNTS;
use floor_types::ids::{ClusterIds, CLUSTERS};
use floor_types::layout::FloorPool;
use floor_types::seeds;
use solana_pubkey::Pubkey;

use crate::convert::pk;

/// The program ids a pool's adapter derives every address from: the `carpenter_amm` that owns the
/// pool, and FLOOR's programs on the same cluster when FLOOR is deployed there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deployment {
    /// The AMM cluster entry.
    pub amm: AmmClusterIds,
    /// FLOOR's programs on that cluster (`None`: no FLOOR deployment, every swap hook is unknown).
    pub floor: Option<ClusterIds>,
}

impl Deployment {
    /// The deployment whose `carpenter_amm` is `owner`, over the shipped tables
    /// (`carpenter_types::ids::AMM_CLUSTERS`, `floor_types::ids::CLUSTERS`).
    pub fn for_owner(owner: &Pubkey) -> Option<Self> {
        Self::for_owner_in(owner, AMM_CLUSTERS, CLUSTERS)
    }

    /// [`Deployment::for_owner`] over a caller's tables.
    pub fn for_owner_in(owner: &Pubkey, amm: &[AmmClusterIds], floor: &[ClusterIds]) -> Option<Self> {
        let owner = owner.to_bytes();
        let amm = *amm.iter().find(|c| c.carpenter_amm == owner)?;
        let floor = floor.iter().find(|c| c.carpenter_amm == owner).copied();
        Some(Self { amm, floor })
    }

    /// `carpenter_amm`'s program id.
    pub fn amm_program(&self) -> Pubkey {
        pk(&self.amm.carpenter_amm)
    }

    /// `floor_hook` on this cluster, when FLOOR is deployed here.
    pub fn floor_hook(&self) -> Option<Pubkey> {
        self.floor.map(|f| pk(&f.floor_hook))
    }
}

/// The name `program_dependencies` reports for a hook program this crate has a quoter for:
/// `floor_hook` on any cluster in `floor_types::ids::CLUSTERS`.
pub fn known_hook_name(program: &[u8; 32]) -> Option<&'static str> {
    CLUSTERS.iter().any(|c| c.floor_hook == *program).then_some("floor_hook")
}

/// The `program_dependencies` name of an admitted base transfer hook other than `floor_holders`.
pub const BASE_TRANSFER_HOOK: &str = "base_transfer_hook";

/// The `program_dependencies` name for a pool's admitted base transfer hook: `floor_holders` (on any
/// cluster in `floor_types::ids::CLUSTERS`) by name, so a test harness that loads `<name>.so` finds
/// the right binary; any other as [`BASE_TRANSFER_HOOK`].
pub fn transfer_hook_name(program: &[u8; 32]) -> &'static str {
    if CLUSTERS.iter().any(|c| c.floor_holders == *program) {
        "floor_holders"
    } else {
        BASE_TRANSFER_HOOK
    }
}

/// A pool's hook, as far as quoting is concerned.
#[derive(Clone, Debug)]
pub enum KnownHook {
    /// No hook, or a hook with no swap flag: it cannot change a swap.
    Inert,
    /// `floor_hook` (this cluster's). `floor_pool` is the `["floor_pool", pool]` PDA under it; `state` and
    /// `cushion_top` are what the last `update` decoded from that account (`None` before the first
    /// `update`, or when the provider did not carry the account).
    Floor { floor_pool: Pubkey, state: Option<Box<FloorPool>>, cushion_top: Option<i32> },
    /// A swap hook this crate has no quoter for.
    Unknown { hook_program: Pubkey },
}

/// A pool this adapter refuses to construct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookQuoteErr {
    /// A `floor_hook` pool whose `hook_callback_accounts` is not the slice this adapter builds
    /// (`floor_pool`, `trader_bucket`, the hook's event authority, `floor_hook`). The program splits the remaining accounts at that count, so a
    /// meta list built for a different count would misplace the tick arrays.
    CallbackAccounts { found: u8, expected: u8 },
    /// The pool's hook is `floor_hook` of ANOTHER cluster than the one whose `carpenter_amm` owns the
    /// pool: no such pool can exist on chain, so the account pair is refused rather than quoted.
    ClusterMismatch { hook: [u8; 32] },
}

impl fmt::Display for HookQuoteErr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HookQuoteErr::CallbackAccounts { found, expected } => write!(
                f,
                "pool.hook_callback_accounts is {found}, not {expected}: floor_hook's slice is floor_pool, trader_bucket, the hook's event authority, floor_hook"
            ),
            HookQuoteErr::ClusterMismatch { hook } => {
                write!(f, "pool.hook {} is floor_hook of another cluster than the carpenter_amm that owns the pool", pk(hook))
            }
        }
    }
}

impl std::error::Error for HookQuoteErr {}

impl KnownHook {
    /// Classify a pool by its `hook` and `hook_flags`, on `deployment` (the cluster whose
    /// `carpenter_amm` owns the pool). `pool_key` seeds FLOOR's `floor_pool` PDA.
    pub fn classify(deployment: &Deployment, pool_key: &Pubkey, pool: &Pool) -> Result<Self, HookQuoteErr> {
        Self::classify_in(deployment, CLUSTERS, pool_key, pool)
    }

    /// [`KnownHook::classify`], with `floor_clusters` as the table a FLOOR hook of another cluster is
    /// recognised (and refused) by.
    pub fn classify_in(deployment: &Deployment, floor_clusters: &[ClusterIds], pool_key: &Pubkey, pool: &Pool) -> Result<Self, HookQuoteErr> {
        if !is_swap_hooked(pool) {
            return Ok(KnownHook::Inert);
        }
        if let Some(floor) = deployment.floor.filter(|f| f.floor_hook == pool.hook) {
            if pool.hook_callback_accounts != FLOOR_CALLBACK_ACCOUNTS {
                return Err(HookQuoteErr::CallbackAccounts { found: pool.hook_callback_accounts, expected: FLOOR_CALLBACK_ACCOUNTS });
            }
            return Ok(KnownHook::Floor { floor_pool: floor_pool_pda(&pk(&floor.floor_hook), pool_key), state: None, cushion_top: None });
        }
        if floor_clusters.iter().any(|c| c.floor_hook == pool.hook) {
            return Err(HookQuoteErr::ClusterMismatch { hook: pool.hook });
        }
        Ok(KnownHook::Unknown { hook_program: pk(&pool.hook) })
    }

    /// Whether a swap on this pool runs a hook callback, and so carries `hook_program` and the hook
    /// slice.
    pub fn calls_back(&self) -> bool {
        matches!(self, KnownHook::Floor { .. })
    }

    /// The decoded `FloorPool`, when this is FLOOR's hook and `update` has read it.
    pub fn floor_state(&self) -> Option<&FloorPool> {
        match self {
            KnownHook::Floor { state, .. } => state.as_deref(),
            _ => None,
        }
    }
}

/// `["floor_pool", pool]` under `floor_hook` (the cluster's `hook_program`).
pub fn floor_pool_pda(hook_program: &Pubkey, pool_key: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[seeds::FLOOR_POOL, pool_key.as_ref()], hook_program).0
}
