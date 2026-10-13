//! Several clusters in one build: the pool account's owner selects the cluster, and every program
//! id and PDA the adapter names comes from that cluster's entry. The second cluster here is a
//! synthetic one that exists only in this test (the shipped tables list mainnet-beta and devnet).

mod common;

use carpenter_amm::accounts::{event_authority_pda, hook_event_authority_pda, tick_array_pda, trader_bucket_pda};
use carpenter_amm::hook::floor_pool_pda;
use carpenter_amm::{CarpenterAmm, Deployment, FIXED_METAS};
use carpenter_quote::{no_limit, PoolAccount};
use carpenter_types::ids::{AmmClusterIds, AMM_DEVNET};
use common::*;
use floor_types::constants as fc;
use floor_types::ids::{ClusterIds, DEVNET};
use jupiter_amm_interface::{Amm, SwapMode};
use solana_pubkey::Pubkey;

const SYNTH_AMM: AmmClusterIds = AmmClusterIds { name: "synthetic", carpenter_amm: [0xD1; 32] };
const SYNTH: ClusterIds = ClusterIds {
    name: "synthetic",
    carpenter_amm: [0xD1; 32],
    floor_hook: [0xD2; 32],
    floor_launch: [0xD3; 32],
    floor_holders: [0xD4; 32],
    floor_vesting: [0xD5; 32],
};
const AMM_TABLE: &[AmmClusterIds] = &[AMM_DEVNET, SYNTH_AMM];
const FLOOR_TABLE: &[ClusterIds] = &[DEVNET, SYNTH];

fn built_in(s: &Synth) -> CarpenterAmm {
    let mut amm = CarpenterAmm::from_keyed_account_in(&s.keyed_account(), &context(), AMM_TABLE, FLOOR_TABLE).expect("from_keyed_account_in");
    for _ in 0..2 {
        amm.update(s.provider()).expect("update");
    }
    amm
}

/// Every devnet program id, and the devnet PDAs a swap could name without a pool key.
fn devnet_keys() -> Vec<Pubkey> {
    let progs = [DEVNET.carpenter_amm, DEVNET.floor_hook, DEVNET.floor_launch, DEVNET.floor_holders, DEVNET.floor_vesting].map(pk);
    let mut v = progs.to_vec();
    v.push(event_authority_pda(&pk(DEVNET.carpenter_amm)));
    v.push(hook_event_authority_pda(&pk(DEVNET.floor_hook)));
    v
}

#[test]
fn the_owner_selects_the_cluster_and_no_devnet_id_reaches_the_swap() {
    let (amm_id, hook) = (pk(SYNTH.carpenter_amm), pk(SYNTH.floor_hook));
    let s = floor_pool_on(amm_id, hook);
    let a = built_in(&s);
    assert_eq!(a.program_id(), amm_id);
    assert_eq!(a.deployment().amm.name, "synthetic");
    assert_eq!(a.deployment().floor, Some(SYNTH));
    assert!(a.hook().calls_back(), "FLOOR pool on its own cluster");
    assert!(a.is_active());
    assert_eq!(s.floor_pool, Some(floor_pool_pda(&hook, &s.key)));
    assert!(a.get_accounts_to_update().contains(&floor_pool_pda(&hook, &s.key)));

    let user = Pubkey::new_from_array([0x5E; 32]);
    let jup = JUP;
    let amount = 1_000_000_000u64;
    for zfo in [true, false] {
        // The quote is the synthetic cluster's quoter on the same bytes.
        let q = a.quote(&s.quote_params(zfo, amount, SwapMode::ExactIn)).unwrap();
        let params = floor_quote::SwapQuoteParams {
            zero_for_one: zfo,
            amount_specified: -(amount as i128),
            sqrt_price_limit_x64: no_limit(zfo),
            supply0: RICH,
            supply1: fc::SUPPLY,
            cushion_top: None,
        };
        let reference = floor_quote::quote_swap_for(
            &SYNTH,
            &PoolAccount { key: s.key.to_bytes(), pool: s.pool() },
            &s.synth.tick_array_set().unwrap(),
            s.floor_state.as_ref(),
            &params,
        )
        .unwrap();
        assert_eq!((q.in_amount, q.out_amount), (reference.amount_in, reference.amount_out));

        let m = a.get_swap_and_account_metas(&swap_params(&s, zfo, amount, user, &jup, false)).unwrap().account_metas;
        assert_eq!(m[11].pubkey, hook, "hook_program");
        assert_eq!(m[12].pubkey, event_authority_pda(&amm_id));
        assert_eq!(m[13].pubkey, amm_id);
        assert_eq!(m[FIXED_METAS].pubkey, floor_pool_pda(&hook, &s.key));
        assert_eq!(m[FIXED_METAS + 1].pubkey, trader_bucket_pda(&hook, &user, &s.mint0));
        assert_eq!(m[FIXED_METAS + 2].pubkey, hook_event_authority_pda(&hook));
        assert_eq!(m[FIXED_METAS + 3].pubkey, hook);
        for st in reference.all_arrays() {
            assert!(m.iter().any(|x| x.pubkey == tick_array_pda(&amm_id, &s.key, st)), "array {st} under the synthetic AMM");
        }
        let devnet = devnet_keys();
        for x in &m {
            assert!(!devnet.contains(&x.pubkey), "a devnet key {} reached the synthetic cluster's swap", x.pubkey);
        }
    }
}

#[test]
fn the_shipped_tables_refuse_an_unknown_owner() {
    let s = floor_pool_on(pk(SYNTH.carpenter_amm), pk(SYNTH.floor_hook));
    let err = CarpenterAmm::from_keyed_account(&s.keyed_account(), &context()).unwrap_err();
    assert!(err.to_string().contains("not carpenter_amm"), "{err}");
    assert!(Deployment::for_owner(&pk(SYNTH.carpenter_amm)).is_none());
    assert_eq!(Deployment::for_owner(&amm_id()).map(|d| d.amm), Some(AMM_DEVNET));
}

#[test]
fn a_hook_of_another_cluster_is_refused() {
    // Owned by the synthetic AMM, hooked by devnet's floor_hook.
    let s = floor_pool_on(pk(SYNTH.carpenter_amm), floor_hook());
    let err = CarpenterAmm::from_keyed_account_in(&s.keyed_account(), &context(), AMM_TABLE, FLOOR_TABLE).unwrap_err();
    assert!(err.to_string().contains("floor_hook of another cluster"), "{err}");
    // Owned by devnet's AMM, hooked by the synthetic cluster's floor_hook.
    let s = floor_pool_on(amm_id(), pk(SYNTH.floor_hook));
    let err = CarpenterAmm::from_keyed_account_in(&s.keyed_account(), &context(), AMM_TABLE, FLOOR_TABLE).unwrap_err();
    assert!(err.to_string().contains("floor_hook of another cluster"), "{err}");
}

#[test]
fn devnet_pools_route_the_same_through_the_extended_tables() {
    let s = floor_pool();
    let via_tables = built_in(&s);
    let shipped = built(&s);
    assert_eq!(via_tables.program_id(), amm_id());
    let qp = s.quote_params(true, 1_000_000_000, SwapMode::ExactIn);
    let (a, b) = (via_tables.quote(&qp).unwrap(), shipped.quote(&qp).unwrap());
    assert_eq!((a.in_amount, a.out_amount, a.fee_amount), (b.in_amount, b.out_amount, b.fee_amount));
    let user = Pubkey::new_from_array([0x5E; 32]);
    let jup = JUP;
    let sp = swap_params(&s, true, 1_000_000_000, user, &jup, false);
    let (ma, mb) = (via_tables.get_swap_and_account_metas(&sp).unwrap().account_metas, shipped.get_swap_and_account_metas(&sp).unwrap().account_metas);
    assert_eq!(ma, mb);
}
