//! Program IDs, as raw bytes (no Solana crate), per cluster.
//!
//! One build serves every cluster in [`AMM_CLUSTERS`]: code that meets a Carpenter account looks up
//! the cluster by the account's owner ([`amm_cluster_by_program`]) and derives every address from that
//! cluster's ids, never from a single compiled-in id. Adding a cluster is one more entry in
//! [`AMM_CLUSTERS`] (and the matching entry in `floor_types::ids::CLUSTERS` when FLOOR is deployed
//! there).
//!
//! The AMM's own ids and the test-only mock hook's live here; `floor_hook`, `floor_launch`,
//! `floor_vesting` and `floor_holders` are `floor_types::ids`, which re-exports these beside them.

/// One cluster's Carpenter program ids.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AmmClusterIds {
    /// The cluster's name (`"devnet"`, `"mainnet"`, ...).
    pub name: &'static str,
    /// `carpenter_amm`'s program id on this cluster.
    pub carpenter_amm: [u8; 32],
}

/// Mainnet-beta.
///
/// `carpenter_amm` base58 `DdeiDx4ncHo9DXGZm1GrPe7mFKv7kNU3yx95uQEV2APA`
pub const AMM_MAINNET: AmmClusterIds = AmmClusterIds {
    name: "mainnet",
    carpenter_amm: [187, 175, 202, 187, 57, 132, 21, 196, 5, 126, 236, 168, 207, 126, 245, 178, 222, 205, 100, 95, 182, 104, 45, 225, 184, 213, 234, 151, 195, 112, 62, 17],
};

/// Devnet.
///
/// `carpenter_amm` base58 `Arv1M4LifBrWMywvxpeAGvh37aVHPjpXPCg7sacbe3Hy`
pub const AMM_DEVNET: AmmClusterIds = AmmClusterIds {
    name: "devnet",
    carpenter_amm: [146, 130, 147, 117, 177, 226, 177, 141, 243, 78, 105, 107, 149, 78, 61, 75, 211, 78, 202, 209, 58, 44, 105, 108, 71, 93, 19, 66, 13, 131, 106, 40],
};

/// Every cluster this build knows. Program ids are distinct across entries (a test checks it), so
/// an owner names at most one cluster.
pub const AMM_CLUSTERS: &[AmmClusterIds] = &[AMM_MAINNET, AMM_DEVNET];

/// The cluster whose `carpenter_amm` is `program`, among [`AMM_CLUSTERS`].
pub fn amm_cluster_by_program(program: &[u8; 32]) -> Option<&'static AmmClusterIds> {
    amm_cluster_by_program_in(AMM_CLUSTERS, program)
}

/// [`amm_cluster_by_program`] over a caller's table.
pub fn amm_cluster_by_program_in<'a>(clusters: &'a [AmmClusterIds], program: &[u8; 32]) -> Option<&'a AmmClusterIds> {
    clusters.iter().find(|c| c.carpenter_amm == *program)
}

/// `carpenter_amm` on mainnet-beta, the id the shipped IDL names. Code that may meet another
/// cluster's accounts uses [`amm_cluster_by_program`] instead.
///
/// base58 `DdeiDx4ncHo9DXGZm1GrPe7mFKv7kNU3yx95uQEV2APA`
pub const CARPENTER_AMM_ID: [u8; 32] = AMM_MAINNET.carpenter_amm;

/// `mock_hook`: the test-only pass-through hook. Never deployed.
///
/// base58 `8NR5f68SspkCNgyGBvjuWcqMbPw1kDzhbpKgqHkCeWJk`
pub const MOCK_HOOK_ID: [u8; 32] = [109, 126, 59, 175, 72, 83, 21, 52, 217, 98, 56, 153, 251, 48, 226, 48, 175, 152, 126, 31, 182, 73, 128, 91, 146, 231, 134, 75, 73, 62, 157, 113];

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic second cluster, test-only.
    const OTHER: AmmClusterIds = AmmClusterIds { name: "other", carpenter_amm: [7u8; 32] };
    const TABLE: &[AmmClusterIds] = &[AMM_DEVNET, OTHER];

    #[test]
    fn the_id_alias_is_the_mainnet_entry() {
        assert_eq!(CARPENTER_AMM_ID, AMM_MAINNET.carpenter_amm);
        assert_eq!(amm_cluster_by_program(&CARPENTER_AMM_ID), Some(&AMM_MAINNET));
        assert_eq!(amm_cluster_by_program(&AMM_DEVNET.carpenter_amm), Some(&AMM_DEVNET));
    }

    #[test]
    fn lookup_by_program_finds_each_cluster_and_refuses_an_unknown_program() {
        assert_eq!(amm_cluster_by_program_in(TABLE, &[7u8; 32]).map(|c| c.name), Some("other"));
        assert_eq!(amm_cluster_by_program_in(TABLE, &AMM_DEVNET.carpenter_amm).map(|c| c.name), Some("devnet"));
        assert!(amm_cluster_by_program_in(TABLE, &AMM_MAINNET.carpenter_amm).is_none(), "mainnet is not in the test table");
        assert!(amm_cluster_by_program_in(TABLE, &[8u8; 32]).is_none());
        assert!(amm_cluster_by_program(&[7u8; 32]).is_none(), "the synthetic cluster is not in the shipped table");
    }

    #[test]
    fn shipped_program_ids_are_distinct_and_names_unique() {
        for (i, a) in AMM_CLUSTERS.iter().enumerate() {
            for b in &AMM_CLUSTERS[i + 1..] {
                assert_ne!(a.carpenter_amm, b.carpenter_amm, "{} and {}", a.name, b.name);
                assert_ne!(a.name, b.name);
            }
        }
    }
}
