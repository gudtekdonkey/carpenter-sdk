//! The quoters' golden vectors replayed through the `Amm` trait.
//!
//! Each vector stores the account bytes a swap was quoted from. Here those bytes are laid out as a
//! router would fetch them (the pool at its key, the bitmap and tick arrays at their PDAs, a mint
//! account per side, FLOOR's `FloorPool` at its PDA), the adapter is built with `from_keyed_account`
//! and `update`, and every swap the trait can express (exact-in with no price limit) is quoted through
//! `Amm::quote` and `Amm::get_swap_and_account_metas` and required to equal the quoters called
//! directly on the same bytes: the same amounts, the same refusal, the same tick arrays. Exact-out
//! swaps must be refused by the adapter; swaps with a price limit cannot be expressed through
//! `QuoteParams` and are counted as skipped.
//!
//! - `carpenter-sdk/fixtures/quote-vectors/plain.json`: plain pools, the quoter's own answers.
//! - `carpenter-sdk/fixtures/quote-vectors/parity-plain.json`: plain swaps a program run agreed with.
//! - `floor-sdk/rust/fixtures/quote-vectors/floor.json`: FLOOR pools.

mod common;

use std::collections::HashMap;
use std::path::PathBuf;

use carpenter_amm::accounts::{bitmap_pda, tick_array_pda};
use carpenter_amm::hook::floor_pool_pda;
use carpenter_amm::CarpenterAmm;
use carpenter_quote::state::{decode_pool, decode_tick_array, TickArraySet};
use carpenter_quote::{no_limit, PoolAccount};
use common::*;
use floor_quote::state::decode_floor_pool;
use floor_types::constants::FLOOR_CALLBACK_ACCOUNTS;
use floor_types::twap::{TwapOracle, FLOOR_POOL_ACCOUNT_LEN_WITH_TWAP};
use jupiter_amm_interface::{Amm, KeyedAccount, SwapMode};
use serde_json::Value;
use solana_pubkey::Pubkey;

fn unhex(s: &str) -> Vec<u8> {
    assert!(s.len() % 2 == 0, "odd hex");
    (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex")).collect()
}

fn int<T: std::str::FromStr>(v: &Value) -> T
where
    T::Err: std::fmt::Debug,
{
    match v {
        Value::String(s) => s.parse().unwrap(),
        Value::Number(n) => n.to_string().parse().unwrap(),
        other => panic!("not an integer: {other}"),
    }
}

fn load(rel: &str) -> Value {
    let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel);
    let raw = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    serde_json::from_str(&raw).unwrap()
}

/// One pool's account bytes, as a vector stores them.
struct Case<'a> {
    name: String,
    key: [u8; 32],
    pool: Vec<u8>,
    bitmap: Vec<u8>,
    arrays: Vec<Vec<u8>>,
    floor_pool: Option<Vec<u8>>,
    swaps: &'a [Value],
}

fn case<'a>(v: &'a Value, swaps: &'static str) -> Case<'a> {
    Case {
        name: v["name"].as_str().unwrap_or("swap").to_string(),
        key: unhex(v["pool_key"].as_str().unwrap()).try_into().unwrap(),
        pool: unhex(v["pool"].as_str().unwrap()),
        bitmap: unhex(v["bitmap"].as_str().unwrap()),
        arrays: v["tick_arrays"].as_array().unwrap().iter().map(|a| unhex(a.as_str().unwrap())).collect(),
        floor_pool: v["floor_pool"].as_str().map(unhex),
        swaps: if swaps.is_empty() { std::slice::from_ref(v) } else { v[swaps].as_array().unwrap() },
    }
}

/// `Pool.hook`: the 8-byte discriminator, then the struct's bytes 88..120.
const POOL_HOOK: usize = 8 + 88;

/// The `floor_hook` id `floor.json` was recorded under: its first case's pool hook. It names a
/// deployment this build does not list, so its pools are replayed under this build's devnet
/// `floor_hook` ([`rehooked`]); both sides of every comparison read the same rewritten bytes.
fn recorded_hook(doc: &Value) -> [u8; 32] {
    let hook = decode_pool(&unhex(doc["cases"][0]["pool"].as_str().unwrap())).unwrap().hook;
    assert_ne!(hook, [0u8; 32], "the first case is a FLOOR pool");
    hook
}

/// `c` with its pool's hook moved from `recorded` to this build's devnet `floor_hook` (any other hook,
/// e.g. a test-only one, is kept).
fn rehooked<'a>(mut c: Case<'a>, recorded: &[u8; 32]) -> Case<'a> {
    if decode_pool(&c.pool).unwrap().hook == *recorded {
        c.pool[POOL_HOOK..POOL_HOOK + 32].copy_from_slice(&floor_types::ids::DEVNET.floor_hook);
        assert_eq!(decode_pool(&c.pool).unwrap().hook, floor_types::ids::DEVNET.floor_hook, "the hook offset");
    }
    c
}

/// `Pool.hook_callback_accounts`: the 8-byte discriminator, then the struct's byte 7.
const POOL_CALLBACK_ACCOUNTS: usize = 8 + 7;

/// `FloorPool.ladder_seeded`: the 8-byte discriminator, then the struct's byte 4.
const FLOOR_POOL_LADDER_SEEDED: usize = 8 + 4;

/// The accounts a router would fetch for this case, with the mints reporting `supply0` / `supply1`
/// and, when `cushion_top` is set, the `FloorPool` account carrying it in its TWAP tail.
fn accounts(c: &Case, supply0: u64, supply1: u64, cushion_top: Option<i32>) -> HashMap<Pubkey, solana_account::Account> {
    let key = Pubkey::new_from_array(c.key);
    let pool = decode_pool(&c.pool).unwrap();
    let mut m = HashMap::new();
    let mut pool_data = c.pool.clone();
    // The FLOOR vectors were written under an older hook header (`hook_callback_accounts` 2); the quoter never
    // reads that byte, but the adapter refuses a FLOOR pool whose slice is not `FLOOR_CALLBACK_ACCOUNTS`, so the
    // pool is replayed under the current header. Nothing else in the bytes changes.
    if floor_types::ids::is_floor_hook(&pool.hook) && pool_data[POOL_CALLBACK_ACCOUNTS] != FLOOR_CALLBACK_ACCOUNTS {
        pool_data[POOL_CALLBACK_ACCOUNTS] = FLOOR_CALLBACK_ACCOUNTS;
        assert_eq!(decode_pool(&pool_data).unwrap().hook_callback_accounts, FLOOR_CALLBACK_ACCOUNTS, "the hook_callback_accounts offset");
    }
    m.insert(key, account(amm_id(), pool_data));
    m.insert(bitmap_pda(&amm_id(), &key), account(amm_id(), c.bitmap.clone()));
    for a in &c.arrays {
        let start = decode_tick_array(a).unwrap().start_index;
        m.insert(tick_array_pda(&amm_id(), &key, start), account(amm_id(), tick_array_account(a.clone())));
    }
    m.insert(pk(pool.mint0), account(pk(pool.token_program0), mint_data(supply0, pool.decimals0)));
    m.insert(pk(pool.mint1), account(pk(pool.token_program1), mint_data(supply1, pool.decimals1)));
    if let Some(fp) = &c.floor_pool {
        let mut data = fp.clone();
        // The vector states carry `ladder_seeded = 0`, which `floor_hook` refuses (`NotSeeded`) and
        // the quoter and adapter therefore refuse too. The vectors record the answers of seeded pools,
        // so they are replayed seeded; the refusal itself is covered in `tests/conformance.rs`.
        if data.len() > FLOOR_POOL_LADDER_SEEDED {
            data[FLOOR_POOL_LADDER_SEEDED] = 1;
            assert_eq!(decode_floor_pool(&data).unwrap().ladder_seeded, 1, "the ladder_seeded offset");
        }
        if let Some(top) = cushion_top {
            data.resize(FLOOR_POOL_ACCOUNT_LEN_WITH_TWAP, 0);
            let mut o = TwapOracle::empty();
            o.cushion_top_set = 1;
            o.cushion_top_tick = top;
            assert!(o.store(&mut data));
            assert_eq!(floor_quote::state::decode_cushion_top(&data), Some(top));
        }
        m.insert(floor_pool_pda(&floor_hook(), &key), account(floor_hook(), data));
    }
    m
}

#[derive(Default, Debug)]
struct Tally {
    answered: usize,
    refused: usize,
    exact_out_refused: usize,
    limited_skipped: usize,
    replaced: usize,
}

/// Replay one case's swaps. `direct` quotes the swap through the quoters on the vector's own bytes.
fn replay(c: &Case, tally: &mut Tally, direct: &dyn Fn(&Value) -> Result<floor_quote::SwapQuote, carpenter_quote::QuoteError>) {
    let key = Pubkey::new_from_array(c.key);
    let pool = decode_pool(&c.pool).unwrap();
    for (n, s) in c.swaps.iter().enumerate() {
        let tag = format!("{} #{n}", c.name);
        let zfo = s["zero_for_one"].as_bool().unwrap();
        let amount: i128 = int(&s["amount_specified"]);
        let limit: u128 = int(&s["sqrt_price_limit_x64"]);
        let supply0: u64 = s.get("supply0").map(int).unwrap_or(0);
        let supply1: u64 = s.get("supply1").map(int).unwrap_or(0);
        let cushion_top = s.get("cushion_top").and_then(Value::as_i64).map(|t| t as i32);

        let provider = Provider(accounts(c, supply0, supply1, cushion_top));
        let ka = KeyedAccount { key, account: provider.0[&key].clone(), params: None };
        let mut amm = CarpenterAmm::from_keyed_account(&ka, &context()).unwrap_or_else(|e| panic!("{tag}: from_keyed_account: {e}"));
        amm.update(&provider).unwrap_or_else(|e| panic!("{tag}: update: {e}"));
        amm.update(&provider).unwrap();

        let (input, output) = if zfo { (pk(pool.mint0), pk(pool.mint1)) } else { (pk(pool.mint1), pk(pool.mint0)) };
        let qp = |amount: u64, mode| jupiter_amm_interface::QuoteParams { amount, input_mint: input, output_mint: output, swap_mode: mode, fee_mode: Default::default() };

        if amount > 0 {
            let err = amm.quote(&qp(amount as u64, SwapMode::ExactOut)).unwrap_err();
            assert!(err.to_string().contains("ExactOutNotSupported"), "{tag}: {err}");
            tally.exact_out_refused += 1;
            continue;
        }
        if limit != no_limit(zfo) {
            tally.limited_skipped += 1;
            continue;
        }
        let in_amount = u64::try_from(-amount).unwrap_or_else(|_| panic!("{tag}: exact-in amount above u64"));
        let want = direct(s);
        let got = amm.quote(&qp(in_amount, SwapMode::ExactIn));
        match (&want, &got) {
            (Ok(w), Ok(g)) => {
                assert_eq!((g.in_amount, g.out_amount), (w.amount_in, w.amount_out), "{tag}");
                if let Some(e) = s["expect"].get("ok") {
                    assert_eq!(g.in_amount.to_string(), e["amount_in"].as_str().unwrap(), "{tag}: the file's own answer");
                    assert_eq!(g.out_amount.to_string(), e["amount_out"].as_str().unwrap(), "{tag}: the file's own answer");
                }
                // The fee triple: the input mint; the LP fee, plus the hook's takes when the input is
                // the hook's quote currency, plus the Carpenter Platform fee when charged on the input.
                let input_is_hook_quote = w.floor_after.map(|f| zfo == (f.quote_is_token1 == 0)).unwrap_or(false);
                let input_currency = if zfo { 0 } else { 1 };
                let fee = w.fees.lp
                    + if input_is_hook_quote { w.fees.hook_total() } else { 0 }
                    + if w.fees.protocol_currency == input_currency { w.fees.protocol } else { 0 };
                assert_eq!((g.fee_amount, g.fee_mint), (fee, input), "{tag}: fee triple");

                let sp = swap_params_for(input, output, in_amount);
                let metas = amm.get_swap_and_account_metas(&sp).unwrap_or_else(|e| panic!("{tag}: metas: {e}")).account_metas;
                let slice = if matches!(amm.hook(), carpenter_amm::KnownHook::Floor { .. }) { usize::from(FLOOR_CALLBACK_ACCOUNTS) } else { 0 };
                let arrays: Vec<Pubkey> = metas[carpenter_amm::FIXED_METAS + slice..].iter().map(|m| m.pubkey).collect();
                let mut want_arrays: Vec<Pubkey> = w.all_arrays().iter().map(|st| tick_array_pda(&amm_id(), &key, *st)).collect();
                // Then the hook authority's position for each distinct ModifyPosition salt.
                let owner = Pubkey::new_from_array(decode_pool(&c.pool).unwrap().hook_authority);
                let mut salts = Vec::new();
                for a in &w.replace_actions {
                    if let carpenter_types::hook::HookAction::ModifyPosition { salt, .. } = a {
                        if !salts.contains(salt) {
                            salts.push(*salt);
                        }
                    }
                }
                want_arrays.extend(salts.iter().map(|salt| {
                    Pubkey::find_program_address(&[b"position", key.as_ref(), owner.as_ref(), &salt.to_le_bytes()], &amm_id()).0
                }));
                assert_eq!(arrays, want_arrays, "{tag}: the metas carry exactly the quoted arrays, then the action positions");
                assert!(amm.get_accounts_len() >= metas.len(), "{tag}");
                tally.answered += 1;
                tally.replaced += usize::from(w.replace.is_some());
            }
            (Err(w), Err(g)) => {
                assert_eq!(g.to_string(), w.to_string(), "{tag}");
                if let Some(e) = s["expect"].get("err") {
                    assert!(g.to_string().starts_with(e.as_str().unwrap()), "{tag}: {g} vs the file's {e}");
                }
                tally.refused += 1;
            }
            _ => panic!("{tag}: direct {want:?} vs adapter {got:?}"),
        }
    }
}

fn swap_params_for<'a>(input: Pubkey, output: Pubkey, amount: u64) -> jupiter_amm_interface::SwapParams<'static, 'a> {
    jupiter_amm_interface::SwapParams {
        swap_mode: SwapMode::ExactIn,
        in_amount: amount,
        out_amount: 0,
        source_mint: input,
        destination_mint: output,
        source_token_account: Pubkey::new_from_array([0xA1; 32]),
        destination_token_account: Pubkey::new_from_array([0xA2; 32]),
        token_transfer_authority: Pubkey::new_from_array([0xEE; 32]),
        user: Pubkey::new_from_array([0xEE; 32]),
        payer: Pubkey::new_from_array([0xEE; 32]),
        quote_mint_to_referrer: None,
        jupiter_program_id: &JUP,
        missing_dynamic_accounts_as_default: false,
    }
}

/// The direct quote for a vector swap: `floor_quote::quote_swap` (which is `carpenter_quote::quote_swap`
/// with FLOOR's hook quoter, and the plain walk on a pool with no swap hook).
fn direct_for<'a>(c: &'a Case<'a>) -> impl Fn(&Value) -> Result<floor_quote::SwapQuote, carpenter_quote::QuoteError> + 'a {
    move |s: &Value| {
        let pool = decode_pool(&c.pool).unwrap();
        let refs: Vec<&[u8]> = c.arrays.iter().map(Vec::as_slice).collect();
        let set = TickArraySet::from_accounts(&c.key, pool.tick_spacing, &refs, Some(&c.bitmap)).unwrap();
        // Seeded, as `accounts` hands the adapter (the stored states are unseeded; see there).
        let fp = c.floor_pool.as_ref().map(|b| {
            let mut fp = decode_floor_pool(b).unwrap();
            fp.ladder_seeded = 1;
            fp
        });
        let params = floor_quote::SwapQuoteParams {
            zero_for_one: s["zero_for_one"].as_bool().unwrap(),
            amount_specified: int(&s["amount_specified"]),
            sqrt_price_limit_x64: int(&s["sqrt_price_limit_x64"]),
            supply0: s.get("supply0").map(int).unwrap_or(0),
            supply1: s.get("supply1").map(int).unwrap_or(0),
            cushion_top: s.get("cushion_top").and_then(Value::as_i64).map(|t| t as i32),
        };
        floor_quote::quote_swap(&PoolAccount { key: c.key, pool }, &set, fp.as_ref(), &params)
    }
}

#[test]
fn plain_vectors_through_the_amm_trait() {
    let doc = load("../fixtures/quote-vectors/plain.json");
    assert_eq!(doc["schema"], "floor-quote/plain/v1");
    let mut t = Tally::default();
    for v in doc["cases"].as_array().unwrap() {
        let c = case(v, "swaps");
        replay(&c, &mut t, &direct_for(&c));
    }
    eprintln!("plain.json: {t:?}");
    assert_eq!((t.answered, t.refused, t.exact_out_refused, t.limited_skipped), (7, 0, 4, 2), "13 swaps");
}

#[test]
fn chain_agreed_plain_vectors_through_the_amm_trait() {
    let doc = load("../fixtures/quote-vectors/parity-plain.json");
    assert_eq!(doc["schema"], "floor-quote/parity-plain/v1");
    let mut t = Tally::default();
    for (n, v) in doc["swaps"].as_array().unwrap().iter().enumerate() {
        let mut c = case(v, "");
        c.name = format!("parity-plain {n}");
        replay(&c, &mut t, &direct_for(&c));
    }
    eprintln!("parity-plain.json: {t:?}");
    assert_eq!((t.answered, t.refused, t.exact_out_refused, t.limited_skipped), (21, 3, 4, 12), "40 swaps");
}

#[test]
fn floor_vectors_through_the_amm_trait() {
    let doc = load("../../floor-sdk/rust/fixtures/quote-vectors/floor.json");
    assert_eq!(doc["schema"], "floor-quote/floor/v1");
    let recorded = recorded_hook(&doc);
    let mut t = Tally::default();
    for v in doc["cases"].as_array().unwrap() {
        let c = rehooked(case(v, "swaps"), &recorded);
        replay(&c, &mut t, &direct_for(&c));
    }
    eprintln!("floor.json: {t:?}");
    assert_eq!((t.answered, t.refused, t.exact_out_refused, t.limited_skipped), (277, 44, 52, 81), "454 swaps");
    assert!(t.replaced > 0, "some answered swaps re-place the bid: {t:?}");
}

/// A FLOOR sell whose bid re-place reads an array the trader's walk never touched (the vector case
/// `replace-10000000000-1000000000-false`, sold without a price limit): the metas must carry it.
#[test]
fn a_re_place_outside_the_trader_walk_is_carried() {
    let doc = load("../../floor-sdk/rust/fixtures/quote-vectors/floor.json");
    let v = doc["cases"].as_array().unwrap().iter().find(|c| c["name"] == "replace-10000000000-1000000000-false").expect("the case");
    let first = &v["swaps"][0];
    let swaps = vec![serde_json::json!({
        "zero_for_one": false,
        "amount_specified": "-100000000000000",
        "sqrt_price_limit_x64": no_limit(false).to_string(),
        "supply0": first["supply0"],
        "supply1": first["supply1"],
        "cushion_top": null,
        "expect": {},
    })];
    let mut c = rehooked(case(v, "swaps"), &recorded_hook(&doc));
    c.swaps = &swaps;
    let q = direct_for(&c)(&swaps[0]).unwrap();
    let outside: Vec<i32> = q.action_arrays.iter().copied().filter(|s| !q.arrays_touched.contains(s)).collect();
    assert!(!outside.is_empty(), "the row must re-place outside the walk: {:?} / {:?}", q.arrays_touched, q.action_arrays);
    let mut t = Tally::default();
    replay(&c, &mut t, &direct_for(&c));
    assert_eq!((t.answered, t.replaced), (1, 1));
}
