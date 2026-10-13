# Third-party code and dependencies

Every file in this repository's Rust crates that was copied or adapted from elsewhere, and every crate
they depend on directly, with its licence. The TypeScript packages under `packages/` record their own
dependencies in their `package.json` files.

**The rule: nothing enters this tree from elsewhere without a row here** — a file with a
`Derived from` / `Adapted from` header, or a new direct dependency in any `Cargo.toml`. A row names
the thing, its pin and its source. A pin is written out, never pointed at.

Normal dependencies of the crates are `borsh` 1.x, `bytemuck` and `uint =0.9.5` from crates.io and
nothing else: no Solana crate, no git source (never Raydium's git fork of `uint`). Test-only crates are
listed in section 3 and are outside that rule.

No Orca Whirlpools code, and nothing from any BUSL-1.1 Uniswap v4 file, is in this tree.

## 1. Copied and adapted source

| What | From | Licence | Where it lands | Attribution record |
|---|---|---|---|---|
| Raydium CLMM `programs/amm/src/libraries/*` (`mod.rs` included) and the `tick_array.rs` start-index helpers | `raydium-io/raydium-clmm` @ `ed7c84a54ced59c55981780546adb0b4583dcf85` | Apache-2.0 | `crates/carpenter-clmm-math/src/{lib,big_num,fixed_point_64,unsafe_math,full_math,tick_math,sqrt_price_math,liquidity_math,swap_math,bitmap}.rs` (10 files) | `crates/carpenter-clmm-math/NOTICE` lists each file against its upstream path (a `<path> <- <upstream>` row each); each file carries a `Derived from … Apache-2.0` header saying what was changed (Apache-2.0 section 4(b)); `crates/carpenter-clmm-math/LICENSE-APACHE-2.0` is upstream's own `LICENSE`, byte-identical (section 4(a)); the root `NOTICE` reproduces the crate's (section 4(d)) |
| Uniswap v4-core `src/libraries/{SwapMath,SqrtPriceMath,TickMath}.sol`, re-expressed at Q64.64 | `Uniswap/v4-core` @ `46c6834` | MIT (each file's `SPDX-License-Identifier: MIT` at that commit; "Copyright 2023 Universal Navigation Inc.") | `crates/carpenter-clmm-math/src/swap_math.rs`, `crates/carpenter-clmm-math/tests/v4_parity.rs` | `crates/carpenter-clmm-math/NOTICE` reproduces the MIT text in full; `tests/v4_parity.rs` opens with `SPDX-License-Identifier: Apache-2.0 AND MIT`; the package's SPDX expression is `Apache-2.0 AND MIT` |
| `sdroege/rust-muldiv` — the phantom-overflow multiply-divide idea and its doc examples, reached through Raydium's `full_math.rs` | crate `muldiv` (repo `sdroege/rust-muldiv`) | MIT, Copyright (c) 2017 Sebastian Dröge | `crates/carpenter-clmm-math/src/full_math.rs` | Upstream's credit line is kept at the top of the module; `NOTICE` names it and its copyright holder |

The three v4-core files are MIT, not BUSL-1.1 — verified by each file's SPDX line at `46c6834`.
Re-check the SPDX line, not the repository's reputation, before adapting any other v4 file.

## 2. Licence texts

| Path | What it is | sha256 | Size |
|---|---|---|---|
| `LICENSE` | Apache-2.0, the licence of this repository | `e3ba223bb1423f0aad8c3dfce0fe3148db48926d41e6fbc3afbbf5ff9e1c89cb` | 10,398 B |
| `crates/carpenter-clmm-math/LICENSE-APACHE-2.0` | `raydium-io/raydium-clmm` @ `ed7c84a5` root `LICENSE`, byte-identical (the canonical Apache-2.0 text, appendix included) | `c71d239df91726fc519c6eb72d318ec65820627232b2f796219e87dcf35d0ab4` | 11,357 B |
| `NOTICE` | This repository's notice; reproduces `crates/carpenter-clmm-math/NOTICE` in full | — | — |
| `crates/carpenter-clmm-math/NOTICE` | The crate's notice: the derived-file rows, the v4-core MIT text, the muldiv credit | — | — |

## 3. Direct dependencies

Licences as published on crates.io for the pinned version. `Cargo.lock` is committed: the versions it
resolves are the ones the tests ran on.

### 3.1 Normal dependencies

| Crate | Pin | Locked | Source | Licence | Used by |
|---|---|---|---|---|---|
| `borsh` | `1.5.7` (feature `derive`) | 1.8.1 | crates.io | MIT OR Apache-2.0 | `carpenter-types` (the hook interface's Borsh schemas) |
| `bytemuck` | `1` (features `derive`, `min_const_generics`) | 1.25.2 | crates.io | Zlib OR Apache-2.0 OR MIT | `carpenter-types`, `carpenter-quote` (`Pod` account layouts) |
| `uint` | `=0.9.5` | 0.9.5 | crates.io | MIT OR Apache-2.0 | `carpenter-clmm-math` (U128 / U256 / U512) |

### 3.2 Test-only dependencies

| Crate | Pin | Locked | Source | Licence | Used by |
|---|---|---|---|---|---|
| `quickcheck` | `=0.9.2` | 0.9.2 | crates.io | Unlicense OR MIT | `carpenter-clmm-math` (upstream Raydium's own property tests) |
| `proptest` | `=1.11.0` | 1.11.0 | crates.io | MIT OR Apache-2.0 | `carpenter-clmm-math`, `tests/v4_parity.rs` |
| `serde_json` | `1` | 1.0.154 | crates.io | MIT OR Apache-2.0 | `carpenter-quote` (golden vectors) |

### 3.3 The wasm crate (`wasm/carpenter-quote-wasm`, its own workspace and `Cargo.lock`)

| Crate | Pin | Locked | Source | Licence | Kind |
|---|---|---|---|---|---|
| `wasm-bindgen` | `=0.2.128` (equal to the wasm-bindgen-cli that binds the output) | 0.2.128 | crates.io | MIT OR Apache-2.0 | normal |
| `carpenter-quote` | path `../../crates/carpenter-quote` | — | this repository | Apache-2.0 | normal |
| `bytemuck` | `1` | 1.25.2 | crates.io | Zlib OR Apache-2.0 OR MIT | dev |

`packages/sdk/wasm/` ships the compiled output of this crate (`carpenter_quote_bg.wasm`, its sha256 in
`packages/sdk/wasm/SHA256SUMS`) and the `wasm-bindgen --target web` glue (`carpenter_quote.js`, `.d.ts`), generated by
wasm-bindgen-cli 0.2.128 (MIT OR Apache-2.0). The wasm links Rust's standard library (MIT OR Apache-2.0) and
`wasm-bindgen`'s runtime; `wasm/build.sh` produces it.

The transitive closure of these crates (the proc-macro crates behind `borsh` / `bytemuck`'s derives,
`crunchy`, `byteorder`, `hex`, `static_assertions` under `uint`, and the test crates' own trees) is in
`Cargo.lock`; a licence audit of it is a `cargo deny check licenses` run, which this file does not
replace.
