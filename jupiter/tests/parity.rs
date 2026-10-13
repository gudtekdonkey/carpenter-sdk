//! Parity with the programs, through Jupiter's `jupiter-amm-test-kit`.
//!
//! The kit loads a pool snapshot (`tests/fixtures/accounts/<pool>/<pubkey>.bin`), runs
//! `Amm::quote`, executes the program's own `swap` in LiteSVM with the programs in
//! `tests/fixtures/<name>.so`, and asserts that the destination token delta equals
//! `quote.out_amount` to the unit.
//!
//! The programs: `carpenter.so`, `floor_hook.so` and `floor_holders.so` are an `anchor build` of the
//! program source this crate is synced to (anchor-cli 1.2.0, Agave 3.1.14, platform-tools v1.57);
//! `spl_token_2022.so` is the cluster's. Each must hash to [`PROGRAMS`].
//!
//! The snapshots are devnet accounts, taken from the deployment these binaries were built for (the
//! three build binaries are byte-identical to the programs deployed there). Its FLOOR pools carry the
//! four-account hook slice (`floor_pool`, the trader's `["trader_ref", trader, mint0]` bucket,
//! `floor_hook`'s `["__event_authority"]` PDA, `floor_hook`): `hook_callback_accounts = 4`. A snapshot
//! taken under an older one-account header is still replayable: the replay copies each snapshot into
//! the test's temporary directory and sets that one byte (`Pool`, account offset 15) to
//! `FLOOR_CALLBACK_ACCOUNTS`; every other byte is the cluster's ([`patched_snapshot`]). On a current
//! snapshot the step changes nothing.
//!
//! Running:
//!
//! - offline (the default): every pool whose snapshot directory exists is replayed. A missing
//!   snapshot or binary is reported and skipped.
//! - binaries: `PROGRAMS_DIR=<target/deploy of the build> cargo test --test parity` copies any
//!   missing build binary in (hash-checked); `RPC=<devnet url>` dumps a missing `spl_token_2022.so`
//!   from the live ProgramData account, trimmed and hash-checked. Nothing is sent to the cluster.
//! - snapshots: `RPC=<devnet url>` writes any missing snapshot; add `REFRESH=1` to re-snapshot
//!   existing pools. The adapter refuses a pool with an old header, so a snapshot of a FLOOR pool
//!   can be taken only from a cluster that carries the four-account slice.

use std::path::{Path, PathBuf};
use std::panic::{catch_unwind, AssertUnwindSafe};

use carpenter_amm::{swap_instruction_data, CarpenterAmm};
use carpenter_quote::state::decode_pool;
use floor_types::constants::FLOOR_CALLBACK_ACCOUNTS;
use floor_types::ids::is_floor_hook;
use jupiter_amm_interface::{AccountProvider, Amm, AmmContext, KeyedAccount, QuoteParams, Swap};
use jupiter_amm_test_kit::{assert_pool_parity, PoolSnapshot, PoolTest};
use sha2::{Digest, Sha256};
use solana_pubkey::Pubkey;

// ---- the programs ------------------------------------------------------------------------------

/// Where a fixture binary comes from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    /// `<PROGRAMS_DIR>/<file>`: the build of the program source.
    Build(&'static str),
    /// The cluster's ProgramData account, kept whole as `solana program dump` writes it.
    Cluster,
}

/// One program the kit loads: its fixture name, id, length, sha256 and source.
struct Program {
    name: &'static str,
    id: &'static str,
    len: usize,
    sha256: &'static str,
    source: Source,
}

const PROGRAMS: &[Program] = &[
    Program {
        name: "carpenter",
        id: "Arv1M4LifBrWMywvxpeAGvh37aVHPjpXPCg7sacbe3Hy",
        len: 1_110_216,
        sha256: "bbce91244c4313d8655ee01b400dfde5b2824253d8b6613b552a23b033e0d0d3",
        source: Source::Build("carpenter_amm.so"),
    },
    Program {
        name: "floor_hook",
        id: "6foted69N6wB1oJzsxccbXUxefBt3Hkn7tN2yHifXnTS",
        len: 1_497_776,
        sha256: "edc645b96c0b1aebc2401d46dd3fc637be590363ff3c164a0aec6a0bbfba7920",
        source: Source::Build("floor_hook.so"),
    },
    Program {
        name: "floor_holders",
        id: "DShmCjpc34oRb3mqjLvp1fs1rYfY6ZLvj1G38d1s6XPZ",
        len: 244_792,
        sha256: "6de0bedb3e8d2d56003b5ea0478f427539c335a84021d9c720b7a8f33e125a62",
        source: Source::Build("floor_holders.so"),
    },
    Program {
        name: "spl_token_2022",
        id: "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb",
        len: 711_008,
        sha256: "b781b71d230a9910de4d3c103e72608f597fc73c83ee207242ac82b708382a6a",
        source: Source::Cluster,
    },
];

fn program_path(name: &str) -> PathBuf {
    PathBuf::from("tests/fixtures").join(format!("{name}.so"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Dump `p` from the cluster: the ProgramData account's payload after its 45-byte header, hash-checked.
fn dump_program(rpc: &str, p: &Program) -> Result<Vec<u8>, String> {
    let id: Pubkey = p.id.parse().unwrap();
    let program = jupiter_amm_test_kit::fetch_account(rpc, &id).map_err(|e| e.to_string())?.ok_or("program account missing")?;
    let programdata: Pubkey = Pubkey::try_from(&program.data[4..36]).map_err(|_| "malformed program account")?;
    let pd = jupiter_amm_test_kit::fetch_account(rpc, &programdata).map_err(|e| e.to_string())?.ok_or("programdata missing")?;
    let payload = &pd.data[45..];
    if payload.len() < p.len || payload[p.len..].iter().any(|b| *b != 0) {
        return Err(format!("{}: payload {} B does not end in zeros after {} B", p.name, payload.len(), p.len));
    }
    let elf = payload[..p.len].to_vec();
    let got = sha256_hex(&elf);
    if got != p.sha256 {
        return Err(format!("{}: deployed sha256 {got}, expected {}", p.name, p.sha256));
    }
    Ok(elf)
}

/// Every program present with the recorded hash; a missing one is copied from `PROGRAMS_DIR` or
/// dumped through `RPC` first. Returns why not, for the skip message.
fn programs_ready() -> Result<(), String> {
    for p in PROGRAMS {
        let path = program_path(p.name);
        if !path.exists() {
            let bytes = match p.source {
                Source::Build(file) => {
                    let Some(dir) = std::env::var_os("PROGRAMS_DIR") else {
                        return Err(format!("{path:?} is missing; rerun with PROGRAMS_DIR=<target/deploy of the build>"));
                    };
                    std::fs::read(Path::new(&dir).join(file)).map_err(|e| format!("{file}: {e}"))?
                }
                Source::Cluster => {
                    let Some(rpc) = std::env::var("RPC").ok() else {
                        return Err(format!("{path:?} is missing; rerun with RPC=<devnet url> to dump it"));
                    };
                    dump_program(&rpc, p)?
                }
            };
            if sha256_hex(&bytes) != p.sha256 {
                return Err(format!("{}: sha256 {}, expected {}", p.name, sha256_hex(&bytes), p.sha256));
            }
            std::fs::create_dir_all("tests/fixtures").map_err(|e| e.to_string())?;
            std::fs::write(&path, bytes).map_err(|e| e.to_string())?;
        }
        let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
        assert_eq!(sha256_hex(&bytes), p.sha256, "{path:?} is not the recorded build");
        assert_eq!(bytes.len(), p.len, "{path:?} length");
    }
    Ok(())
}

// ---- the snapshot header -----------------------------------------------------------------------

/// `Pool.hook_callback_accounts`: after the 8-byte discriminator, the struct's byte 7.
const CALLBACK_ACCOUNTS_OFFSET: usize = 8 + 7;

/// The snapshot directory as the replay reads it: a copy under the test's temporary directory in
/// which a FLOOR pool's `hook_callback_accounts` is `FLOOR_CALLBACK_ACCOUNTS`. Returns the
/// directory and the header the snapshot carried. No other byte of any account changes.
fn patched_snapshot(pool: &Pubkey, dir: &Path) -> (PathBuf, u8) {
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("parity-snapshots").join(pool.to_string());
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out).expect("snapshot copy");
    for entry in std::fs::read_dir(dir).expect("snapshot") {
        let path = entry.expect("snapshot entry").path();
        std::fs::copy(&path, out.join(path.file_name().unwrap())).expect("copy a snapshot account");
    }
    let snap = PoolSnapshot::load_dir(dir).expect("snapshot");
    let mut account = snap.get(pool).expect("the pool account").clone();
    let before = account.data[CALLBACK_ACCOUNTS_OFFSET];
    let decoded = decode_pool(&account.data).expect("pool");
    if is_floor_hook(&decoded.hook) && before != FLOOR_CALLBACK_ACCOUNTS {
        account.data[CALLBACK_ACCOUNTS_OFFSET] = FLOOR_CALLBACK_ACCOUNTS;
        assert_eq!(decode_pool(&account.data).unwrap().hook_callback_accounts, FLOOR_CALLBACK_ACCOUNTS, "the callback-accounts offset");
        PoolSnapshot::from_accounts([(*pool, account)].into_iter().collect()).save_dir(&out).expect("write the patched pool");
    }
    (out, before)
}

/// The kit's encoder: the native `swap` data from the adapter's `Swap` and the input amount, with
/// no minimum out (the kit asserts the amount itself).
fn encode(swap: &Swap, in_amount: u64) -> Vec<u8> {
    swap_instruction_data(swap, in_amount, 0).expect("encode swap")
}

// ---- the pools ---------------------------------------------------------------------------------

const WSOL: &str = "So11111111111111111111111111111111111111112";

/// What a pool's cases assert.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Every swap executes and its output delta equals the quote.
    Parity,
    /// The launch is still bonding (`FloorPool.ladder_seeded == 0`): `floor_hook` refuses every swap
    /// with `NotSeeded`, so the adapter must be inactive and refuse every quote.
    NotSeeded,
}

/// A pool and its cases: `(input is mint0, amount in raw units)`.
struct Case {
    pool: &'static str,
    mint0: &'static str,
    mint1: &'static str,
    note: &'static str,
    expect: Expect,
    swaps: &'static [(bool, u64)],
}

/// wSOL in: 0.001, 0.01, 0.1, 1 SOL. Base in (6 decimals): 1,000 / 10,000 / 100,000 / 1,000,000
/// tokens.
const SIZES: &[(bool, u64)] = &[
    (true, 1_000_000),
    (true, 10_000_000),
    (true, 100_000_000),
    (true, 1_000_000_000),
    (false, 1_000_000_000),
    (false, 10_000_000_000),
    (false, 100_000_000_000),
    (false, 1_000_000_000_000),
];

const CASES: &[Case] = &[
    Case {
        pool: "6adMuquw3XfNY8Rz67GW5gT3GyeE5sZYf4B8b8kMbYGF",
        mint0: WSOL,
        mint1: "AJquQCZqk71HwSr5SismUKMicvCneswrXFuXM4pAxB1D",
        note: "KLE: FLOOR pool, base mint armed with floor_holders",
        expect: Expect::Parity,
        swaps: SIZES,
    },
    Case {
        pool: "EEGuQPrpVxPnkdBmzMEnnf26ReU57d7uNkTBpMk1oJXC",
        mint0: WSOL,
        mint1: "5swrdx3cBuiWPYx2KXedvpnMNixRUBTmSA6242vsiB1D",
        note: "FLOOR pool of a launch still bonding (hooked)",
        expect: Expect::NotSeeded,
        swaps: SIZES,
    },
    Case {
        pool: "4zdnm3GVBHXrccvU5Uz6htmcj493rCJLSpUZw7crYd4P",
        mint0: WSOL,
        mint1: "9S4YM9ETMJTTdZEXhJRT5qUwymSifHG1Ro4TxZ96rB1D",
        note: "FLOOR pool of a launch still bonding (Cook preset)",
        expect: Expect::NotSeeded,
        swaps: SIZES,
    },
    Case {
        pool: "4d2Bioq9UHkkxWZnujqtHjw8wfZBm4tRnTuUFFnqAqVm",
        mint0: WSOL,
        mint1: "9gijWgMZrBCF888SQgJuCLAA72bWMT1uHE3M2tbptB1D",
        note: "FLOOR pool of a launch still bonding",
        expect: Expect::NotSeeded,
        swaps: SIZES,
    },
    Case {
        pool: "9aEUwtP3HgTXL17PXKM5pErQHqNDWNr2QBTsDgNCtGmf",
        mint0: WSOL,
        mint1: "2P7sDUcvcWsNhbAPQ7eVZkzha9168H7K4sSuXazJeB1D",
        note: "FLOOR pool of a launch still bonding",
        expect: Expect::NotSeeded,
        swaps: SIZES,
    },
];

fn pk(s: &str) -> Pubkey {
    s.parse().unwrap()
}

fn fixtures_dir(pool: &Pubkey) -> PathBuf {
    PathBuf::from("tests/fixtures/accounts").join(pool.to_string())
}

/// Snapshot (when asked) and replay one pool. Each swap is asserted on its own so one failure
/// does not hide the others; the results are returned as lines.
fn run_case(case: &Case) -> Result<Vec<(String, bool)>, String> {
    let pool = pk(case.pool);
    let (m0, m1) = (pk(case.mint0), pk(case.mint1));
    let dir = fixtures_dir(&pool);
    let all = case.swaps.iter().fold(PoolTest::new(pool), |t, (zfo, amount)| {
        if *zfo { t.add_swap(m0, m1, *amount) } else { t.add_swap(m1, m0, *amount) }
    });
    let rpc = std::env::var("RPC").ok();
    if std::env::var_os("REFRESH").is_some() || !dir.exists() {
        let Some(rpc) = rpc else {
            return Err(format!("{} has no snapshot at {dir:?}; rerun with RPC=<devnet url>", case.pool));
        };
        // A refused pool has no swap account list to discover: snapshot what `update` reads.
        let snap = if case.expect == Expect::NotSeeded { PoolTest { swaps: Vec::new(), ..all } } else { all };
        snap.regenerate_fixtures::<CarpenterAmm>(&rpc).map_err(|e| e.to_string())?;
    }
    let (dir, header) = patched_snapshot(&pool, &dir);
    let mut lines = Vec::new();
    if header != FLOOR_CALLBACK_ACCOUNTS {
        eprintln!("parity: {} snapshot header hook_callback_accounts {header}, replayed as {FLOOR_CALLBACK_ACCOUNTS}", case.pool);
    }
    if case.expect == Expect::NotSeeded {
        let amm = hydrated(&pool, &dir);
        let inactive = !amm.is_active();
        lines.push((format!("{} is_active() == false", case.pool), inactive));
        for (zfo, amount) in case.swaps {
            let (i, o) = if *zfo { (m0, m1) } else { (m1, m0) };
            let q = amm.quote(&QuoteParams { amount: *amount, input_mint: i, output_mint: o, swap_mode: Default::default(), fee_mode: Default::default() });
            let ok = matches!(&q, Err(e) if e.to_string() == "NotSeeded");
            let side = if *zfo { "mint0 in" } else { "mint1 in" };
            lines.push((format!("{} {side} {amount}: quote refused (NotSeeded)", case.pool), ok));
        }
        return Ok(lines);
    }
    for (zfo, amount) in case.swaps {
        let mut one = if *zfo { PoolTest::new(pool).add_swap(m0, m1, *amount) } else { PoolTest::new(pool).add_swap(m1, m0, *amount) };
        one.fixtures_dir = dir.clone();
        let quoted = quote_of(&one, *amount);
        let hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let result = catch_unwind(AssertUnwindSafe(|| assert_pool_parity::<CarpenterAmm>(&one, encode)));
        std::panic::set_hook(hook);
        let side = if *zfo { "mint0 in" } else { "mint1 in" };
        let ok = result.is_ok();
        let why = match result {
            Ok(()) => "equal".to_string(),
            Err(e) => short(&e.downcast_ref::<String>().cloned().or_else(|| e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()),
        };
        lines.push((format!("{} {side} {amount}: quote {quoted} -> {why}", case.pool), ok));
    }
    Ok(lines)
}

/// A kit panic message cut to what names the failure: the program's error line when the swap
/// failed, else the first 400 characters (an `assert_eq!` of quote and delta fits).
fn short(msg: &str) -> String {
    if let Some(i) = msg.find("Error Code: ") {
        let rest = &msg[i..];
        let end = rest.find("Error Message").unwrap_or(rest.len().min(120));
        return format!("swap failed: {}", rest[..end].trim_end_matches([' ', '.']));
    }
    msg.chars().take(400).collect::<String>().replace('\n', " ")
}

/// The adapter's quote for the line (the kit asserts the same figure against the swap).
fn quote_of(test: &PoolTest, amount: u64) -> String {
    let amm = hydrated(&test.pool, &test.fixtures_dir);
    let s = &test.swaps[0];
    match amm.quote(&QuoteParams { amount, input_mint: s.input_mint, output_mint: s.output_mint, swap_mode: Default::default(), fee_mode: Default::default() }) {
        Ok(q) => format!("out {} fee {}", q.out_amount, q.fee_amount),
        Err(e) => format!("refused ({e})"),
    }
}

/// The adapter built and updated from a snapshot directory, as the kit does it.
fn hydrated(pool: &Pubkey, dir: &Path) -> CarpenterAmm {
    let snap = PoolSnapshot::load_dir(dir).expect("snapshot");
    let ctx = AmmContext { clock_ref: Default::default() };
    let keyed = KeyedAccount { key: *pool, account: snap.get(pool).unwrap().clone(), params: None };
    let mut amm = CarpenterAmm::from_keyed_account(&keyed, &ctx).expect("from_keyed_account");
    for _ in 0..3 {
        amm.update(Provider(&snap)).expect("update");
    }
    amm
}

struct Provider<'a>(&'a PoolSnapshot);

impl<'a> AccountProvider for Provider<'a> {
    fn get(&self, pubkey: &Pubkey) -> Option<impl solana_account::ReadableAccount + use<'_, 'a>> {
        self.0.get(pubkey).cloned()
    }
}

#[test]
fn devnet_pools_match_the_programs() {
    if let Err(why) = programs_ready() {
        eprintln!("parity: skipped: {why}");
        return;
    }
    let mut failed = Vec::new();
    let mut ran = 0;
    for case in CASES {
        match run_case(case) {
            Err(why) => eprintln!("parity: skipped {} ({}): {why}", case.pool, case.note),
            Ok(lines) => {
                for (line, ok) in lines {
                    ran += 1;
                    eprintln!("parity: {} {line}", if ok { "ok  " } else { "FAIL" });
                    if !ok {
                        failed.push(line);
                    }
                }
            }
        }
    }
    eprintln!("parity: {ran} checks, {} failed", failed.len());
    assert!(failed.is_empty(), "parity failures:\n{}", failed.join("\n"));
}

/// The adapter refuses a FLOOR pool whose hook header is not the four-account slice, by name,
/// rather than building an account list the program would split differently. The snapshot (the
/// current header) constructs; the same account with the header byte set to 1 is refused.
#[test]
fn the_adapter_refuses_an_old_hook_header() {
    let pool = pk(CASES[0].pool);
    let path = fixtures_dir(&pool).join(format!("{pool}.bin"));
    if !Path::new(&path).exists() {
        eprintln!("parity: skipped: no snapshot at {path:?}");
        return;
    }
    let snap = PoolSnapshot::load_dir(fixtures_dir(&pool)).unwrap();
    let account = snap.get(&pool).unwrap().clone();
    let header = decode_pool(&account.data).unwrap().hook_callback_accounts;
    let keyed = KeyedAccount { key: pool, account, params: None };
    let built = CarpenterAmm::from_keyed_account(&keyed, &AmmContext { clock_ref: Default::default() });
    if header == FLOOR_CALLBACK_ACCOUNTS {
        assert!(built.is_ok(), "a pool with the current header constructs");
    } else {
        let err = built.err().expect("refused");
        assert!(err.to_string().contains(&format!("hook_callback_accounts is {header}, not {FLOOR_CALLBACK_ACCOUNTS}")), "{err}");
    }
    let mut old = keyed.account.clone();
    old.data[CALLBACK_ACCOUNTS_OFFSET] = 1;
    let err = CarpenterAmm::from_keyed_account(&KeyedAccount { key: pool, account: old, params: None }, &AmmContext { clock_ref: Default::default() })
        .err()
        .expect("an old header is refused");
    assert!(err.to_string().contains(&format!("hook_callback_accounts is 1, not {FLOOR_CALLBACK_ACCOUNTS}")), "{err}");
}
