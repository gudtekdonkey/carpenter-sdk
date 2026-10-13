# carpenter-amm

An implementation of [`jupiter-amm-interface`](https://github.com/jup-ag/jupiter-amm-interface)'s
`Amm` trait for Carpenter pools. It quotes a pool from account bytes and lists the accounts of the
`carpenter_amm` program's `swap` instruction. It computes no swap amount itself: every amount comes
from `carpenter-quote` (the AMM's quoter, in this repository's `crates/`) and, on a pool hooked by
FLOOR's `floor_hook`, from `floor-quote`'s `FloorHookQuoter` (in
[`floor-sdk`](https://github.com/gudtekdonkey/floor-sdk)).

This crate is its own Cargo workspace with its own `Cargo.lock`; it is not a member of the
repository's root workspace. It is the only crate here that links Solana crates.

## Status

- `get_swap_and_account_metas` returns `Swap::Placeholder { data: [zero_for_one] }`, the interface's
  variant for venues that have no variant of their own. It carries only a direction byte, which does
  not describe this swap's account list, so a route cannot execute a Carpenter swap from it. This
  crate is usable for quoting, for building the native `swap` instruction (`swap_instruction_data`),
  and with the interface's test kit. A dedicated variant is proposed in `upstream/` (below).
- The clusters the adapter routes are the entries of `carpenter_types::ids::AMM_CLUSTERS` (FLOOR's ids
  beside them in `floor_types::ids::CLUSTERS`): mainnet-beta and devnet, and a cluster is added as one
  more entry. The programs are upgradeable through a timelock on mainnet-beta (a 10-minute delay that can only be raised, proposed by the deployer key) and by the deployer key directly on devnet, and pausable by the deployer key.
- FLOOR pools are quoted on the four-account FLOOR hook slice (`hook_callback_accounts = 4`), which
  the programs on both listed clusters carry. A FLOOR pool whose header says otherwise (an older
  deployment's one-account slice) is refused by name. Pools with no swap hook are unaffected.
- A pool whose hook settles (`SETTLES_FLAG`: the hook mints into a vault inside the swap) is quoted
  only through that hook's own quoter; this crate registers FLOOR's, which does not settle, so such
  a pool is refused like any other unknown swap hook.
- No security review of the programs or of this crate has been completed.
- The interface is a git dependency pinned to `jup-ag/jupiter-amm-interface` @
  `95bd18485e580625a39eb92a0c0ad2061579daad`. The trait at that commit (`AccountProvider`,
  `Swap::Placeholder`, `AmmError`, a required `get_accounts_len`) differs from every published
  release, so the crates.io versions cannot be substituted.

## What each method does

| Method | Behaviour |
|---|---|
| `from_keyed_account` | Requires an account owned by `carpenter_amm` of a listed cluster, holding a `Pool`; that owner selects the cluster every program id and PDA comes from (`from_keyed_account_in` takes the tables as arguments). Classifies its hook (below). Keeps the context's clock handle for `is_active`; does not read it. |
| `label` | `"Carpenter"` (the test kit's program fixture is then `tests/fixtures/carpenter.so`). |
| `program_id`, `key`, `get_reserve_mints` | `carpenter_amm`, the pool address, `[mint0, mint1]`. |
| `get_accounts_to_update` | The pool, its bitmap, `floor_pool` on a FLOOR pool, both mints, the tick arrays of the current window, and (last) the `extra-account-metas` account of an admitted base-mint transfer hook. |
| `update` | Decodes all of the above. All or nothing: a failed pass leaves the previous state and makes the pool inactive until a pass succeeds. |
| `quote` | Exact-in only. `amount_specified = -amount`, no price limit, direction from the mints; `quote_swap_with` with the registered hook quoters. |
| `get_swap_and_account_metas` | The `swap` accounts in program order, then the hook slice, then exactly the tick arrays the quote at `in_amount` reads (`SwapQuote::all_arrays`: the trader's walk and every hook action's), then the hook position each `ModifyPosition` action of that quote modifies, then any transfer-hook accounts. |
| `has_dynamic_accounts` | `true`. |
| `supports_exact_out` | `false`. FLOOR pools refuse exact-out on chain, and a one-byte `Placeholder` carries only a direction. |
| `unidirectional`, `requires_update_for_reserve_mints` | `false`. |
| `program_dependencies` | `floor_hook` on a FLOOR pool, `spl_token_2022` when either side uses Token-2022, the admitted base transfer hook (`floor_holders` by name, any other as `base_transfer_hook`). These names are the test kit's `tests/fixtures/<name>.so`. |
| `get_accounts_len` | 14 fixed + the hook slice + the window's arrays + the one hook position an in-swap action may modify + the transfer hook's accounts: an upper bound on what `get_swap_and_account_metas` returns. |
| `underlying_liquidities` | `None`. |
| `is_active` | False before a successful `update`, on a paused pool (the authority's `paused` flag, or a guardian pause whose `guardian_paused_until` is ahead of the router's clock — the one place the context's clock is read; `quote` never reads it), with no in-range liquidity, on an unknown swap hook, on a FLOOR pool whose `floor_pool` is missing, bound elsewhere or not yet seeded (`ladder_seeded != 1`, a launch still in its bonding sale; quotes are refused too, as `floor_hook` refuses the swap with `NotSeeded`), when FLOOR's positions span more than the window, or when a mint carries a transfer hook the program would refuse. |

### Hooks

| Pool | Quoted as |
|---|---|
| no hook, or a hook with neither `BEFORE_SWAP_FLAG` nor `AFTER_SWAP_FLAG` | plain (`carpenter-quote` alone) |
| `floor_hook` of the cluster that owns the pool, with a swap flag | `carpenter-quote` + `FloorHookQuoter` |
| `floor_hook` of another cluster | refused at `from_keyed_account` (`HookQuoteErr::ClusterMismatch`) |
| any other hook with a swap flag | not quoted: `is_active() == false`, `quote` returns `QuoteHookNotQuotable` |

A FLOOR pool without its `floor_pool` account is inactive and quotes `QuoteHookStateMissing`. Adding
another hook means adding its `HookQuoter` to the registry in `src/hook.rs` / `src/amm.rs`.

### Accounts of `swap`

`trader` (signer), `pool` (w), `bitmap` (w), `trader_token_in` (w), `trader_token_out` (w), `vault0`
(w), `vault1` (w), `mint0`, `mint1`, `token_program0`, `token_program1`, `hook_program` (the pool's
hook when a callback runs, otherwise `carpenter_amm`'s own id, Anchor's encoding of an absent
optional account), `event_authority`, `program`; then on a FLOOR pool the four-account hook slice:
`floor_pool` (w), the trader's `["trader_ref", trader, mint0]` PDA under `floor_hook` (w), `floor_hook`'s
`["__event_authority"]` PDA and `floor_hook` (both read-only); then the tick arrays (w); then,
when the quote's hook actions modify a position (FLOOR's in-swap bid re-place), that position
`["position", pool, pool.hook_authority, salt u64 LE]` under `carpenter_amm` (w). The program finds
tick arrays and positions by key and fails the swap with `HookActionAccountMissing` when an action's
account is absent.

### The fee triple

`Quote` has one `(fee_amount, fee_mint, fee_pct)`. `fee_mint` is the input mint and `fee_amount`
counts every take charged in the input currency: the LP fee; on a FLOOR buy, FLOOR's takes (skim,
creator fee, holder tax, FLOOR Platform fee), which are charged on the quote currency; and the
Carpenter Platform fee when it is charged on the input. Takes charged on the output (FLOOR's on a
sell, the Carpenter Platform fee on a FLOOR sell) are already out of `out_amount` and are not added.
`in_amount` and `out_amount` are the quoter's figures exactly.

### Tick-array window

`update` reads every initialized tick array when the bitmap marks at most 12; otherwise FLOOR's
position span plus the arrays nearest the current price, up to 12. A swap that walks past the window
is refused by name (`TickArrayMissing(start)`), never quoted against missing liquidity;
`missing_dynamic_accounts_as_default` returns the fixed accounts for exactly that refusal.

## Building and testing


```bash
cd jupiter
cargo test
```

| Test file | What it checks |
|---|---|
| `tests/conformance.rs` | each trait method on synthetic plain and FLOOR pools against the quoters called directly; the window; the encoder; no network, clock or math crate in the graph or the source |
| `tests/vectors.rs` | the golden vectors of `carpenter-sdk/fixtures/quote-vectors` and `floor-sdk/rust/fixtures/quote-vectors`, laid out as accounts and replayed through `Amm::quote` / `get_swap_and_account_metas`, equal to the direct quoters |
| `tests/idl.rs` | the account list, discriminator and argument layout against `packages/sdk/idl/carpenter_amm.json` |
| `tests/transfer_hook.rs` | the transfer-hook accounts of an admitted base mint, and the refusals |
| `tests/clusters.rs` | a second, synthetic cluster: the pool's owner selects the cluster, its program ids and PDAs alone reach the swap, an unknown owner and another cluster's `floor_hook` are refused |
| `tests/parity.rs` | Jupiter's `jupiter-amm-test-kit`: devnet pool snapshots, `Amm::quote`, and the programs' own `swap` in LiteSVM; the output delta must equal `quote.out_amount` (below) |

`carpenter-quote` and `carpenter-types` are this repository's crates (path dependencies); `floor-quote` and
`floor-types` come from `floor-sdk` at the release tag with the same version. `floor-quote` itself depends on
`carpenter-quote` at that tag, and the `[patch]` in `Cargo.toml` builds it against the local crates, so there is one
`carpenter-quote`: two sources of it would be two crates, and `FloorHookQuoter` would not
implement the `HookQuoter` this crate calls (the repository README, "Two repositories, one `HookQuoter`").

### Parity with the programs (`tests/parity.rs`)

The kit loads `tests/fixtures/accounts/<pool>/<pubkey>.bin` (bincode `Account`s), builds the adapter
from them, quotes, then sends the native `swap` built from `get_swap_and_account_metas` and
`swap_instruction_data` to LiteSVM with the program binaries below, from a fresh user whose source
token account holds exactly the input. It asserts the destination token delta equals
`quote.out_amount` to the unit. Each swap runs in a fresh SVM.

```bash
cargo test --test parity -- --nocapture                                      # offline replay (default)
PROGRAMS_DIR=<target/deploy> cargo test --test parity                        # copy in missing build binaries
RPC=https://api.devnet.solana.com cargo test --test parity                   # snapshot missing pools, dump Token-2022
REFRESH=1 RPC=https://api.devnet.solana.com cargo test --test parity         # re-snapshot every pool
```

A pool whose snapshot is absent, or a missing binary, is reported and skipped. `carpenter.so`,
`floor_hook.so` and `floor_holders.so` are an `anchor build` of the program source this crate is
synced to (anchor-cli 1.2.0, Agave 3.1.14, platform-tools v1.57), copied from `PROGRAMS_DIR`. They
are byte-identical to the programs deployed at those ids on devnet (checked against each ProgramData
payload); `spl_token_2022.so` is dumped from its ProgramData account (payload after the 45-byte header). Every
binary is refused unless its sha256 matches. Nothing is sent to the cluster. The kit's RPC client
(`ureq` with bundled roots) must reach the endpoint directly; behind a TLS-intercepting proxy, point
`RPC` at a local plain-HTTP forwarder.

Program binaries (`tests/fixtures/`, required for the offline replay):

| File | Program | Bytes | sha256 |
|---|---|---:|---|
| `carpenter.so` | `carpenter_amm` `Arv1M4LifBrWMywvxpeAGvh37aVHPjpXPCg7sacbe3Hy` | 1,110,216 | `bbce91244c4313d8655ee01b400dfde5b2824253d8b6613b552a23b033e0d0d3` |
| `floor_hook.so` | `floor_hook` `6foted69N6wB1oJzsxccbXUxefBt3Hkn7tN2yHifXnTS` | 1,497,776 | `edc645b96c0b1aebc2401d46dd3fc637be590363ff3c164a0aec6a0bbfba7920` |
| `floor_holders.so` | `floor_holders` `DShmCjpc34oRb3mqjLvp1fs1rYfY6ZLvj1G38d1s6XPZ` | 244,792 | `6de0bedb3e8d2d56003b5ea0478f427539c335a84021d9c720b7a8f33e125a62` |
| `spl_token_2022.so` | Token-2022 `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb` (whole ProgramData payload) | 711,008 | `b781b71d230a9910de4d3c103e72608f597fc73c83ee207242ac82b708382a6a` |

A new build changes these hashes; the test refuses a binary that does not match, so the table and
`PROGRAMS` in `tests/parity.rs` move together.

Pools (devnet snapshots; all FLOOR-hooked: no pool without a hook exists on the devnet deployment):

| Pool | Pair | Cases |
|---|---|---|
| `6adMuquw3XfNY8Rz67GW5gT3GyeE5sZYf4B8b8kMbYGF` | wSOL / KLE (`AJqu…xB1D`); base mint armed with `floor_holders` | buys 0.001–1 SOL, sells 1,000–1,000,000 tokens (8) |
| `EEGuQPrp…`, `4zdnm3GV…`, `4d2Bioq9…`, `9aEUwtP3…` | wSOL / a launch still bonding | `is_active() == false` and every quote refused with `NotSeeded` (the program refuses these swaps) |

The hook slice. This crate builds FLOOR's four-account slice (`floor_pool`, the trader's
`["trader_ref", trader, mint0]` bucket, `floor_hook`'s `["__event_authority"]` PDA, `floor_hook`)
and refuses a FLOOR pool whose `hook_callback_accounts` is anything else. The snapshots come from the
devnet deployment the fixture binaries were built for (the three build binaries are byte-identical to the
programs deployed there), whose pools carry `hook_callback_accounts = 4`. A snapshot taken under an older
one-account header can still be replayed: the replay copies each snapshot into the test's temporary
directory and sets that byte (`Pool`, account offset 15, which `initialize_pool` copies from the approved
hook's header and no other account the swap reads depends on) to 4; on a current snapshot the step changes
nothing. `the_adapter_refuses_an_old_hook_header` pins the refusal on the snapshot with that byte set to 1.

## The upstream `Swap` variant (`upstream/`)

| File | What |
|---|---|
| `upstream/swap-variant.patch` | `git apply`-able diff against `jupiter-amm-interface` @ `95bd1848…`: appends `Swap::Carpenter { zero_for_one: bool, hook_accounts: u8, tick_arrays: u8, remaining_accounts_info: Option<RemainingAccountsInfo> }` |
| `upstream/PR.md` | the description that would accompany it |
| `upstream/check.sh` | builds and tests this crate with the `upstream-variant` feature against a patched interface |

The `upstream-variant` feature makes `get_swap_and_account_metas` emit `Swap::Carpenter` (counts: the
hook slice; the tick arrays and positions; the base mint's transfer-hook accounts as one
`TransferHookA` / `TransferHookB` slice) instead of `Placeholder`; `swap_instruction_data` encodes
both to the same bytes. The feature does not compile against the pinned interface, so it is built
through a cargo `--config` `[patch]` for one run, leaving `Cargo.toml` alone and restoring
`Cargo.lock` afterwards:

```bash
upstream/check.sh                          # clone the pinned commit, apply the patch, cargo test
upstream/check.sh /path/to/patched/checkout -- --nocapture
```

The default build (no feature) emits `Placeholder` and is what `cargo test` runs.

## Licence

Apache-2.0 (`LICENSE`). Direct dependencies and their licences are listed in `THIRD_PARTY.md`.
