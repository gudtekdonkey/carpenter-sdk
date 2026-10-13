//! Chain-measured vectors: `fixtures/quote-vectors/parity-plain.json`.
//!
//! Each entry is a plain-pool swap whose outcome an SVM run of the `carpenter_amm` program agreed with,
//! unit for unit, together with the account bytes it was quoted from (the pool, its bitmap, the tick
//! arrays the swap reads). This test replays every entry through the quoter from those bytes alone and
//! requires the recorded answer, so the file is a check any implementation can run without the program.
//!
//! Never hand-edit the file; it is produced by the program's test suite from a run with zero
//! mismatches.

use std::path::PathBuf;

use carpenter_quote::state::{decode_pool, TickArraySet};
use carpenter_quote::{quote_swap, PoolAccount, SwapQuoteParams};
use serde_json::{json, Value};

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd hex");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/quote-vectors/parity-plain.json")
}

#[test]
fn every_chain_agreed_plain_swap_replays_exactly() {
    let raw = std::fs::read_to_string(path()).unwrap_or_else(|e| panic!("{}: {e}", path().display()));
    let doc: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(doc["schema"], "floor-quote/parity-plain/v1");
    let swaps = doc["swaps"].as_array().expect("swaps");
    let (mut ok, mut refused) = (0, 0);
    for (n, s) in swaps.iter().enumerate() {
        assert!(s["floor_pool"].is_null(), "swap {n}: the plain tier carries no hook state");
        let key: [u8; 32] = unhex(s["pool_key"].as_str().unwrap()).try_into().unwrap();
        let pool = decode_pool(&unhex(s["pool"].as_str().unwrap())).unwrap();
        let arrays: Vec<Vec<u8>> = s["tick_arrays"].as_array().unwrap().iter().map(|a| unhex(a.as_str().unwrap())).collect();
        let refs: Vec<&[u8]> = arrays.iter().map(Vec::as_slice).collect();
        let bitmap = unhex(s["bitmap"].as_str().unwrap());
        let set = TickArraySet::from_accounts(&key, pool.tick_spacing, &refs, Some(&bitmap)).unwrap();
        let params = SwapQuoteParams {
            zero_for_one: s["zero_for_one"].as_bool().unwrap(),
            amount_specified: s["amount_specified"].as_str().unwrap().parse().unwrap(),
            sqrt_price_limit_x64: s["sqrt_price_limit_x64"].as_str().unwrap().parse().unwrap(),
            supply0: 0,
            supply1: s["supply1"].as_str().unwrap().parse().unwrap(),
        };
        let got = match quote_swap(&PoolAccount { key, pool }, &set, None, &params) {
            Ok(q) => {
                ok += 1;
                json!({ "ok": {
                    "all_arrays": q.all_arrays(),
                    "amount_in": q.amount_in.to_string(),
                    "amount_out": q.amount_out.to_string(),
                    // a plain pool charges no hook fee and re-places nothing
                    "fee_creator": "0",
                    "fee_lp": q.fees.lp.to_string(),
                    "fee_skim": "0",
                    "replace": null,
                    "sqrt_after": q.sqrt_after.to_string(),
                    "tick_after": q.tick_after,
                }})
            }
            Err(e) => {
                refused += 1;
                json!({ "err": e.name() })
            }
        };
        assert_eq!(got, s["expect"], "swap {n}");
    }
    assert_eq!((ok, refused), (35, 5), "the file's 40 swaps: 35 answered, 5 refused");
}
