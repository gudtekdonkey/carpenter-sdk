//! Golden vectors: `fixtures/quote-vectors/plain.json`.
//!
//! The vectors are the Rust quoter's answers on fixed account bytes; any other implementation (a wasm
//! build, a TypeScript port) must reproduce them exactly. Chain-measured vectors are
//! `parity-plain.json`, replayed by `tests/parity_plain.rs`.
//!
//! ```bash
//! FLOOR_WRITE_VECTORS=1 cargo test -p carpenter-quote --test vectors   # (re)write
//! cargo test -p carpenter-quote --test vectors                         # compare
//! ```
//!
//! Compare mode FAILS when the file is absent: a golden test that passes on a missing file guards
//! nothing. Write it once, commit it, and every later run compares.

use std::path::PathBuf;

use carpenter_quote::clmm::tick_math;
use carpenter_quote::state::{encode_account, encode_bitmap};
use carpenter_quote::synth::{build_pool, PoolKeys, SynthKind, SynthPosition};
use carpenter_quote::{no_limit, quote_swap, SwapQuoteParams};
use serde_json::{json, Value};

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/quote-vectors/plain.json")
}

fn pos(lo: i32, up: i32, l: u128) -> SynthPosition {
    SynthPosition { tick_lower: lo, tick_upper: up, liquidity: l }
}

struct Case {
    name: &'static str,
    fee: u32,
    spacing: u16,
    tick: i32,
    positions: Vec<SynthPosition>,
    swaps: Vec<(bool, i128, Option<i32>)>,
}

fn cases() -> Vec<Case> {
    vec![
        Case {
            name: "one-range-3000",
            fee: 3_000,
            spacing: 60,
            tick: 0,
            positions: vec![pos(-6_000, 6_000, 10_000_000_000_000)],
            swaps: vec![(true, -1_000_000, None), (false, -1_000_000, None), (true, 500_000, None), (false, 123_456_789, None)],
        },
        Case {
            name: "crossing-500",
            fee: 500,
            spacing: 60,
            tick: 0,
            positions: vec![pos(-600, 600, 5_000_000_000_000), pos(-60_000, 60_000, 1_000_000_000_000)],
            swaps: vec![(true, -200_000_000_000, None), (false, -200_000_000_000, None), (true, -200_000_000_000, Some(-3_000))],
        },
        Case {
            name: "spacing-1-negative",
            fee: 100,
            spacing: 1,
            tick: -207_243,
            positions: vec![pos(-207_300, -207_200, 3_000_000_000_000), pos(-210_000, -200_000, 40_000_000)],
            swaps: vec![(true, -9_999, None), (false, -9_999, None), (false, 1_000_000, None)],
        },
        Case {
            name: "empty-to-limit",
            fee: 3_000,
            spacing: 10,
            tick: 12_345,
            positions: vec![],
            swaps: vec![(true, -1_000, Some(12_000))],
        },
        Case {
            name: "r12-full-fee",
            fee: 1_000_000,
            spacing: 60,
            tick: 0,
            positions: vec![pos(-600, 600, 1_000_000_000_000)],
            swaps: vec![(true, -1_000, None), (true, 1_000, None)],
        },
    ]
}

fn build() -> Value {
    let keys = PoolKeys {
        pool: [7; 32],
        mint0: [1; 32],
        mint1: [2; 32],
        vault0: [3; 32],
        vault1: [4; 32],
        token_program0: [5; 32],
        token_program1: [6; 32],
        hook: [0; 32],
        hook_authority: [0; 32],
    };
    let mut out = Vec::new();
    for c in cases() {
        let sqrt = tick_math::get_sqrt_price_at_tick(c.tick).unwrap();
        let sp = build_pool(&keys, SynthKind::Plain { fee: c.fee }, c.spacing, sqrt, 9, 6, &c.positions).unwrap();
        let set = sp.tick_array_set().unwrap();
        let mut swaps = Vec::new();
        for (zfo, amount, limit_tick) in &c.swaps {
            let limit = match limit_tick {
                Some(t) => tick_math::get_sqrt_price_at_tick(*t).unwrap(),
                None => no_limit(*zfo),
            };
            let params = SwapQuoteParams { zero_for_one: *zfo, amount_specified: *amount, sqrt_price_limit_x64: limit, supply0: 0, supply1: 0 };
            let expect = match quote_swap(&sp.account, &set, None, &params) {
                Ok(q) => json!({
                    "ok": {
                        "amount_in": q.amount_in.to_string(),
                        "amount_out": q.amount_out.to_string(),
                        "fee_lp": q.fees.lp.to_string(),
                        "sqrt_after": q.sqrt_after.to_string(),
                        "tick_after": q.tick_after,
                        "arrays_touched": q.arrays_touched,
                        "pool_after": hex(&encode_account(&q.pool_after)),
                    }
                }),
                Err(e) => json!({ "err": e.name() }),
            };
            swaps.push(json!({
                "zero_for_one": zfo,
                "amount_specified": amount.to_string(),
                "sqrt_price_limit_x64": limit.to_string(),
                "expect": expect,
            }));
        }
        out.push(json!({
            "name": c.name,
            "pool_key": hex(&keys.pool),
            "pool": hex(&encode_account(&sp.account.pool)),
            "bitmap": hex(&encode_bitmap(&keys.pool, c.spacing, &sp.words)),
            "tick_arrays": sp.arrays.iter().map(|a| hex(&encode_account(a))).collect::<Vec<_>>(),
            "swaps": swaps,
        }));
    }
    json!({
        "schema": "floor-quote/plain/v1",
        "note": "Each swap is quoted independently against the case's initial state. Integers above 2^53 are decimal strings.",
        "cases": out,
    })
}

#[test]
fn plain_vectors() {
    let now = build();
    let p = path();
    if std::env::var("FLOOR_WRITE_VECTORS").as_deref() == Ok("1") {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, serde_json::to_string_pretty(&now).unwrap() + "\n").unwrap();
        eprintln!("wrote {}", p.display());
        return;
    }
    let raw = std::fs::read_to_string(&p)
        .unwrap_or_else(|e| panic!("{}: {e} — generate it with FLOOR_WRITE_VECTORS=1 and commit it", p.display()));
    let golden: Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(golden, now, "the quoter's answers moved; if that is intended, rewrite with FLOOR_WRITE_VECTORS=1");
}
