# Third-party dependencies of `carpenter-amm`

No third-party source is copied into this crate. Its direct dependencies, as resolved in this
workspace's `Cargo.lock`:

| Crate | Pin | Source | Licence | Kind |
|---|---|---|---|---|
| `jupiter-amm-interface` | `0.6.1` @ git rev `95bd18485e580625a39eb92a0c0ad2061579daad` | https://github.com/jup-ag/jupiter-amm-interface | Apache-2.0 | normal |
| `solana-pubkey` | `4.2.0` (feature `curve25519`) | crates.io | Apache-2.0 | normal |
| `solana-instruction` | `3.4.0` | crates.io | Apache-2.0 | normal |
| `solana-account` | `4.3.1` | crates.io | Apache-2.0 | normal |
| `rust_decimal` | `1.40.0` | crates.io | MIT | normal |
| `carpenter-types`, `carpenter-quote` | path `../crates/*` | this repository | Apache-2.0 | normal |
| `floor-types`, `floor-quote` | path `../../floor-sdk/rust/crates/*` | https://github.com/gudtekdonkey/floor-sdk | Apache-2.0 | normal |
| `serde_json` | `1.0.149` | crates.io | MIT OR Apache-2.0 | dev |
| `solana-clock` | `3.1.1` | crates.io | Apache-2.0 | dev |
| `jupiter-amm-test-kit` | `0.1.0` @ git rev `95bd18485e580625a39eb92a0c0ad2061579daad` (pulls `litesvm` `0.14.0`) | https://github.com/jup-ag/jupiter-amm-interface | Apache-2.0 | dev |
| `sha2` | `0.10.9` | crates.io | MIT OR Apache-2.0 | dev |

## Test fixtures

`tests/fixtures/` holds the parity test's inputs (`tests/parity.rs`): account snapshots read from
Solana devnet, the Token-2022 program dumped from devnet, and three compiled programs built from source
for the test. No source is copied.

| File | Pin (sha256) | Source | Licence |
|---|---|---|---|
| `tests/fixtures/spl_token_2022.so` | `b781b71d230a9910de4d3c103e72608f597fc73c83ee207242ac82b708382a6a` (711,008 B, ProgramData payload of `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`, deploy slot 503,153,936) | devnet; built from https://github.com/solana-program/token-2022 | Apache-2.0 |
| `tests/fixtures/carpenter.so` | `bbce91244c4313d8655ee01b400dfde5b2824253d8b6613b552a23b033e0d0d3` (1,110,216 B) | compiled from the Carpenter program source this crate is synced to (`anchor build`, anchor-cli 1.2.0, Agave 3.1.14, platform-tools v1.57); byte-identical to the program deployed at this id on devnet | a compiled program (no source included), included only as a test fixture |
| `tests/fixtures/floor_hook.so` | `edc645b96c0b1aebc2401d46dd3fc637be590363ff3c164a0aec6a0bbfba7920` (1,497,776 B) | compiled from the FLOOR program source of the same commit, as above; byte-identical to the program deployed at this id on devnet | a compiled program (no source included), included only as a test fixture |
| `tests/fixtures/floor_holders.so` | `6de0bedb3e8d2d56003b5ea0478f427539c335a84021d9c720b7a8f33e125a62` (244,792 B) | compiled from the FLOOR program source of the same commit, as above; byte-identical to the program deployed at this id on devnet | a compiled program (no source included), included only as a test fixture |
| `tests/fixtures/accounts/<pool>/*.bin` | account snapshots | devnet | data |
