# carpenter-sdk

SDKs for Carpenter, a concentrated-liquidity AMM on Solana with Uniswap-v4-style hooks (the `carpenter_amm` program):
pools keyed by `(mint0, mint1, fee, tickSpacing, hook)`, outside LPs, and an allowlist of approved hook programs.

| Path | What |
|---|---|
| [`packages/sdk`](packages/sdk) | `@floorsupply/carpenter-sdk` (TypeScript): the IDL and Codama client, layouts, PDAs, reads, swap / liquidity / pool builders, the one-prompt sender, and the plain-pool quoter (wasm, bundled) |
| [`crates/`](crates) | Rust: `carpenter-types` (layouts, seeds, constants, the hook interface), `carpenter-clmm-math` (Q64.64 CLMM math, see `NOTICE`), `carpenter-quote` (the network-free quoter, with the `HookQuoter` interface) |
| [`wasm/`](wasm) | `carpenter-quote-wasm` (the quoter's plain-pool path behind wasm-bindgen) and `build.sh`, which builds what `packages/sdk/wasm/` ships |
| [`jupiter/`](jupiter) | `carpenter-amm`: a `jupiter-amm-interface` `Amm` over the quoters; its own Cargo workspace |
| [`integrations/defillama/`](integrations/defillama) | Carpenter's DefiLlama adapters (TVL; volume and fees), kept here and copied into DefiLlama's repositories |
| [`fixtures/quote-vectors`](fixtures/quote-vectors) | golden quote vectors (`plain.json`) and chain-measured outcomes (`parity-plain.json`) |

The programs are upgradeable through a timelock on mainnet-beta (a 10-minute delay that can only be raised, proposed by the deployer key) and by the deployer key directly on devnet, and pausable by the deployer key; nothing here claims they are audited or immutable.

## Two repositories, one `HookQuoter`

Hooks plug into the quoter through `carpenter_quote::HookQuoter`. FLOOR, the first approved hook, lives in
[`floor-sdk`](https://github.com/gudtekdonkey/floor-sdk), and the two repositories depend on each other in one
direction each:

- `floor-sdk`'s Rust crates (`floor-types`, `floor-quote`) depend on this repository's `carpenter-types` and
  `carpenter-quote`: `floor-quote` implements `HookQuoter`.
- `jupiter/` (`carpenter-amm`) depends on `floor-sdk`'s `floor-quote` and `floor-types`, to quote FLOOR pools.

Both are git dependencies pinned to the **release commit** of each repository, the commit its `v0.1.0` tag names:

```toml
# floor-sdk/rust/Cargo.toml (carpenter-sdk v0.1.0)
carpenter-quote = { git = "https://github.com/gudtekdonkey/carpenter-sdk", rev = "6996086514a27c348189f150916fedaa8536c5b5" }
# jupiter/Cargo.toml (floor-sdk v0.1.0; this repository's own crates stay local through a [patch] of that git source)
floor-quote     = { git = "https://github.com/gudtekdonkey/floor-sdk", rev = "72399c843d3177333ef3cbf8b06a6d78ec53f1be" }
```

The pins must agree: Cargo treats `carpenter-quote` from two different sources (a path and a git revision, or two revisions) as
two different crates, so `floor-quote`'s `HookQuoter` implementation would not be the trait `carpenter-amm` calls.
`cargo tree -p carpenter-amm -i carpenter-quote` must show a single `carpenter-quote`.

## Build and test

```sh
cargo test --workspace                       # crates/
(cd jupiter && cargo test)                   # the adapter, including the program parity replay
bash jupiter/upstream/check.sh               # the adapter against the proposed upstream Swap variant
bash wasm/build.sh && npm run sync-wasm -w @floorsupply/carpenter-sdk
npm ci && npm test                           # packages/sdk
```

## Clusters

| Cluster | `carpenter_amm` | `amm_config` |
|---|---|---|
| mainnet-beta (`MAINNET_CHAIN_KEY`) | `DdeiDx4ncHo9DXGZm1GrPe7mFKv7kNU3yx95uQEV2APA` | `85S8wiRLyv9Urc6g9tF8gDC56F5Fq4JBu9Zub3BEtyuV` |
| devnet (`DEVNET_CHAIN_KEY`) | `Arv1M4LifBrWMywvxpeAGvh37aVHPjpXPCg7sacbe3Hy` | `4MQRpJ85JE6FqBWFj47GcCoJpdFMVsLsMVPAq9865iiE` |

The shipped IDL and the generated client name the mainnet program; `DEFAULT_PROGRAM_IDS` is mainnet's. Devnet runs a
separate deployment of the same program source under its own id: pass its row's ids (below).

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
   npm run generate -w @floorsupply/carpenter-sdk           # validates deployments.json → src/generated/
   bash wasm/build.sh && npm run sync-wasm -w @floorsupply/carpenter-sdk
   cargo test --workspace && (cd jupiter && cargo test) && npm test
   ```

Applications pass the row's ids everywhere: `programIdsFromDeployment(deploymentFor(chainKey))` as `ids` to every
builder, PDA helper and read. `deploymentFor` throws by name for a cluster with no row.

## License

Apache-2.0 (`LICENSE`). `carpenter-clmm-math` is derived from Raydium's CLMM and Uniswap v4 and keeps its `NOTICE`.
