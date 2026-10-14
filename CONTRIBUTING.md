# Contributing to carpenter-sdk

How this repository is built, tested and extended. For using the SDK, see [`README.md`](README.md).

## Build and test

```sh
cargo test --workspace                       # crates/
(cd jupiter && cargo test)                   # the adapter, including the program parity replay
bash jupiter/upstream/check.sh               # the adapter against the proposed upstream Swap variant
bash wasm/build.sh && npm run sync-wasm -w @floorlaunchpad/carpenter-sdk
npm ci && npm test                           # packages/sdk
```

## Two repositories, one `HookQuoter`

Hooks plug into the quoter through `carpenter_quote::HookQuoter`. FLOOR, the first approved hook, lives in
[`floor-sdk`](https://github.com/gudtekdonkey/floor-sdk), and the two repositories depend on each other in one
direction each:

- `floor-sdk`'s Rust crates (`floor-types`, `floor-quote`) depend on this repository's `carpenter-types` and
  `carpenter-quote`: `floor-quote` implements `HookQuoter`.
- `jupiter/` (`carpenter-amm`) depends on `floor-sdk`'s `floor-quote` and `floor-types`, to quote FLOOR pools.

Both are git dependencies pinned to a commit of each repository whose crates are the `v0.1.0` release's:

```toml
# floor-sdk/rust/Cargo.toml (carpenter-sdk v0.1.0)
carpenter-quote = { git = "https://github.com/gudtekdonkey/carpenter-sdk", rev = "6996086514a27c348189f150916fedaa8536c5b5" }
# jupiter/Cargo.toml (floor-sdk v0.1.0; this repository's own crates stay local through a [patch] of that git source)
floor-quote     = { git = "https://github.com/gudtekdonkey/floor-sdk", rev = "72399c843d3177333ef3cbf8b06a6d78ec53f1be" }
```

The pins must agree: Cargo treats `carpenter-quote` from two different sources (a path and a git revision, or two revisions) as
two different crates, so `floor-quote`'s `HookQuoter` implementation would not be the trait `carpenter-amm` calls.
`cargo tree -p carpenter-amm -i carpenter-quote` must show a single `carpenter-quote`.

## Adding a cluster

One build serves every cluster it lists; nothing assumes a single cluster's program ids. To add a cluster:

1. **`packages/sdk/deployments.json`**: add a row keyed by the cluster's CAIP-2 id.

   | Field | Value | |
   |---|---|---|
   | `name` | the cluster's name (`"testnet"`, ...) | |
   | `chainKey` | the row's key (`solana:` + the first 32 characters of the genesis hash) | |
   | `programs.carpenterAmm` | the deployed program id | may differ from the IDL's `address` |
   | `accounts.ammConfig` | `["amm_config"]` under `carpenterAmm` | derivable; checked |
   | `accounts.protocolTreasury` | `["protocol_treasury"]` under `carpenterAmm` | derivable; checked when present |
   | `programDataHash.carpenterAmm` | the deployed executable's hash, or `null` | |
   | `quotes` | optional: `{ <mint>: { decimals, tokenProgram, caps, capNames, amm: { entry, listedSlot } } }` | `entry` is `["quote_mint", mint]` under `carpenterAmm`; checked |

   Only the `mainnet` and `local` rows must equal the shipped IDL's `address`; any other row is checked to name base58
   32-byte keys, and its derivable accounts are checked against that row's program id.
2. **`crates/carpenter-types/src/ids.rs`**: add an `AmmClusterIds { name, carpenter_amm }` entry to `AMM_CLUSTERS`.
   The router adapter (`jupiter/`) selects the cluster by the pool account's owner and refuses an owner it does not
   list. When FLOOR is deployed on the cluster, `floor-sdk` adds its entry to `floor_types::ids::CLUSTERS`.
3. Regenerate and rebuild:

   ```sh
   npm run generate -w @floorlaunchpad/carpenter-sdk           # validates deployments.json → src/generated/
   bash wasm/build.sh && npm run sync-wasm -w @floorlaunchpad/carpenter-sdk
   cargo test --workspace && (cd jupiter && cargo test) && npm test
   ```

Applications pass the row's ids everywhere: `programIdsFromDeployment(deploymentFor(chainKey))` as `ids` to every
builder, PDA helper and read. `deploymentFor` throws by name for a cluster with no row.

