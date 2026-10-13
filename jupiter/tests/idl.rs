//! The adapter's account list and instruction data against `carpenter_amm`'s IDL
//! (`packages/sdk/idl/carpenter_amm.json`): the program address, the `swap` discriminator, the fixed
//! accounts' order, names and flags, and the argument layout the encoder writes.

mod common;

use std::path::PathBuf;

use carpenter_amm::{swap_instruction_data, FIXED_METAS};
use carpenter_types::disc;
use carpenter_types::ids::AMM_MAINNET;
use common::*;
use jupiter_amm_interface::{Amm, Swap};
use serde_json::Value;
use solana_pubkey::Pubkey;

fn idl() -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../packages/sdk/idl/carpenter_amm.json");
    let raw = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&raw).unwrap()
}

fn swap_ix(idl: &Value) -> &Value {
    idl["instructions"].as_array().unwrap().iter().find(|ix| ix["name"] == "swap").expect("swap instruction")
}

#[test]
fn the_idl_names_this_program_and_this_discriminator() {
    let idl = idl();
    // The shipped IDL names the mainnet-beta program; the synthetic pools here are devnet's.
    assert_eq!(idl["address"].as_str().unwrap(), Pubkey::new_from_array(AMM_MAINNET.carpenter_amm).to_string());
    let d: Vec<u8> = swap_ix(&idl)["discriminator"].as_array().unwrap().iter().map(|b| b.as_u64().unwrap() as u8).collect();
    assert_eq!(d, disc::amm::SWAP);
}

/// The fixed metas are the IDL's accounts in order, with the IDL's signer and writable flags; an
/// optional account the swap does not use (`hook_program` on a plain pool) is the program id, Anchor's
/// encoding of `None`.
#[test]
fn the_fixed_metas_follow_the_idl_accounts() {
    let idl = idl();
    let accounts = swap_ix(&idl)["accounts"].as_array().unwrap().clone();
    assert_eq!(accounts.len(), FIXED_METAS, "the IDL's swap has the fixed accounts only; the rest are remaining accounts");
    let names: Vec<&str> = accounts.iter().map(|a| a["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "trader", "pool", "bitmap", "trader_token_in", "trader_token_out", "vault0", "vault1", "mint0", "mint1", "token_program0",
            "token_program1", "hook_program", "event_authority", "program"
        ]
    );
    let user = Pubkey::new_from_array([0x55; 32]);
    for s in [plain_pool(), floor_pool()] {
        let amm = built(&s);
        for zfo in [true, false] {
            let metas = amm.get_swap_and_account_metas(&swap_params(&s, zfo, 1_000_000_000, user, &JUP, false)).unwrap().account_metas;
            for (i, a) in accounts.iter().enumerate() {
                let signer = a["signer"].as_bool().unwrap_or(false);
                let writable = a["writable"].as_bool().unwrap_or(false);
                assert_eq!((metas[i].is_signer, metas[i].is_writable), (signer, writable), "{} (#{i})", names[i]);
                if let Some(fixed) = a.get("address").and_then(Value::as_str) {
                    // The IDL's own program id stands for the pool's cluster's `carpenter_amm`.
                    let want = if fixed == idl["address"].as_str().unwrap() { amm_id().to_string() } else { fixed.to_string() };
                    assert_eq!(metas[i].pubkey.to_string(), want, "{}", names[i]);
                }
            }
            assert_eq!(metas[13].pubkey, amm_id(), "program");
            if !amm.hook().calls_back() {
                assert_eq!(metas[11].pubkey, amm_id(), "an absent optional account is the program id");
            }
            // After the fixed ones: on a FLOOR pool the hook slice (floor_pool and trader_bucket writable, the
            // callbacks' event authority and program read-only), then the by-key accounts, all writable. None signs.
            let slice = if amm.hook().calls_back() { usize::from(amm.pool().hook_callback_accounts) } else { 0 };
            assert!(metas[FIXED_METAS..].iter().all(|m| !m.is_signer));
            if slice > 0 {
                let w: Vec<bool> = metas[FIXED_METAS..FIXED_METAS + slice].iter().map(|m| m.is_writable).collect();
                assert_eq!(w, [true, true, false, false], "the FLOOR hook slice's writability");
            }
            assert!(metas[FIXED_METAS + slice..].iter().all(|m| m.is_writable));
        }
    }
}

/// The encoder writes the IDL's argument list: `zero_for_one: bool, amount_specified: i128,
/// sqrt_price_limit_x64: u128, amount_limit: u64, hook_data: bytes`.
#[test]
fn the_encoder_writes_the_idl_args() {
    let idl = idl();
    let args: Vec<(String, String)> = swap_ix(&idl)["args"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| (a["name"].as_str().unwrap().to_string(), a["type"].as_str().unwrap_or("?").to_string()))
        .collect();
    let want = [("zero_for_one", "bool"), ("amount_specified", "i128"), ("sqrt_price_limit_x64", "u128"), ("amount_limit", "u64"), ("hook_data", "bytes")];
    assert_eq!(args, want.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect::<Vec<_>>());
    let width = |t: &str| match t {
        "bool" => 1,
        "i128" | "u128" => 16,
        "u64" => 8,
        "bytes" => 4, // an empty vector: the u32 length alone
        other => panic!("{other}"),
    };
    let d = swap_instruction_data(&Swap::Placeholder { data: Some(vec![1]) }, 7, 0).unwrap();
    assert_eq!(d.len(), 8 + want.iter().map(|(_, t)| width(t)).sum::<usize>());
}
