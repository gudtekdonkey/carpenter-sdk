# carpenter-sdk

SDKs for Carpenter, a concentrated-liquidity AMM on Solana with Uniswap-v4-style hooks (the `carpenter_amm` program):
pools keyed by `(mint0, mint1, fee, tickSpacing, hook)`, outside LPs, and an allowlist of approved hook programs. Read
pools, quote and build swaps, add and remove liquidity, and create pools. Rust crates and a Jupiter adapter serve routers
and aggregators.

| | |
|---|---|
| Release | `0.1.1` |
| npm | `@floorlaunchpad/carpenter-sdk` (peer: `@solana/kit` `8.3.0`) |
| Rust | git dependencies at tag `0.1.0` (not on crates.io) |
| Docs | [floor.supply/developers/carpenter](https://floor.supply/developers/carpenter) |
| Licence | Apache-2.0 |

The programs are upgradeable through a timelock on mainnet-beta (a 10-minute delay that can only be raised, proposed by
the deployer key) and by the deployer key directly on devnet, and pausable by the deployer key. Nothing here claims they
are audited or immutable.

## Start here

| You build | Use | Read |
|---|---|---|
| **A listing app or data site** (pool lists, token pages, screeners, TVL) | `stateView` reads (every read tagged with its slot), the account decoders and the event decoders; DefiLlama's adapters are in [`integrations/defillama`](integrations/defillama). | [Read a pool](#read-a-pool) · [Interfaces](#addresses-and-interfaces) |
| **An aggregator or router** | Rust: `carpenter-quote` (the network-free quoter, the program's own arithmetic) and `carpenter-amm` (a `jupiter-amm-interface` `Amm`, FLOOR pools included). | [`jupiter/`](jupiter) · [`crates/`](crates) |
| **A terminal, trading app or bot** | The wasm quoter, `buildSwap` and `sendSolanaTransactions` (one wallet prompt per action with a batch-signing wallet). | [Quote and swap](#quote-and-swap) · [`packages/sdk`](packages/sdk/README.md) |
| **A liquidity app** | `buildModifyLiquidity` (add, remove, collect fees) and `planCreatePool`. | [`packages/sdk`](packages/sdk/README.md#create-a-pool-and-add-liquidity) |
| **A hook author** | `carpenter_quote::HookQuoter`: an approved hook that changes swaps ships a Rust replay of its callbacks, so routers can quote its pools. | [`crates/carpenter-quote`](crates/carpenter-quote) |
| **Trading FLOOR launches** | FLOOR pools are hooked Carpenter pools, quoted and traded by FLOOR's own SDK. | [`floor-sdk`](https://github.com/gudtekdonkey/floor-sdk) |

## Install

```sh
npm install @floorlaunchpad/carpenter-sdk @solana/kit@8.3.0
```

The quoter wasm is shipped in the package, so no Rust toolchain is needed.

```toml
# Rust
[dependencies]
carpenter-types = { git = "https://github.com/gudtekdonkey/carpenter-sdk", tag = "0.1.0" }
carpenter-quote = { git = "https://github.com/gudtekdonkey/carpenter-sdk", tag = "0.1.0" }
```

## Read a pool

```ts
import { createSolanaRpc } from '@solana/kit';
import { deploymentFor, fromKitRpc, MAINNET_CHAIN_KEY, programIdsFromDeployment, stateView } from '@floorlaunchpad/carpenter-sdk';

const rpc = createSolanaRpc('https://api.mainnet-beta.solana.com');
const src = fromKitRpc(rpc);
const ids = programIdsFromDeployment(deploymentFor(MAINNET_CHAIN_KEY)); // DEVNET_CHAIN_KEY for devnet

const { slot, value: pool } = await stateView.getPool(src, poolAddress, ids);
```

Take program ids from `deploymentFor`, never from a constant in an app. `DEFAULT_PROGRAM_IDS` (mainnet's) is for tests
and scripts.

## Quote and swap

```ts
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { buildSwap, compileSendable, decodePool, readSwapQuoteAccounts, sendSolanaTransactions, swapTickArrays } from '@floorlaunchpad/carpenter-sdk';
import { initQuote } from '@floorlaunchpad/carpenter-sdk/quote/wasm';
import { getBase64EncodedWireTransaction } from '@solana/kit';

// Node: read the shipped wasm. A browser fetches `@floorlaunchpad/carpenter-sdk/carpenter_quote_bg.wasm` and passes the bytes.
const quoter = await initQuote(readFileSync(createRequire(import.meta.url).resolve('@floorlaunchpad/carpenter-sdk/carpenter_quote_bg.wasm')));

const { value: v } = await readSwapQuoteAccounts(src, poolAddress, { radius: 4, ids });
const q = quoter.quoteSwap({ ...v, zeroForOne: true, amountSpecified: -1_000_000n }); // negative: exact in
const { starts } = await swapTickArrays(poolAddress, q, ids);

const tx = await buildSwap({
  trader, pool: poolAddress, poolState: decodePool(v.pool), zeroForOne: true,
  amountSpecified: -1_000_000n, amountLimit: (q.amountOut * 99n) / 100n, // the minimum out: 1% slippage
  tickArrayStarts: starts, ids,
});
const { value: bh } = await rpc.getLatestBlockhash().send();
const { transaction } = compileSendable('swap', { feePayer: trader, instructions: tx.instructions, lifetime: bh });
await sendSolanaTransactions(wallet, [getBase64EncodedWireTransaction(transaction)], rpc, { lastValidBlockHeight: bh.lastValidBlockHeight });
```

Things a trading integration must know:

- **Hooked pools.** The quoter quotes pools with no swap hook. A pool whose hook takes part in swaps is refused
  (`QuoteHookNotQuotable`) and is quoted by its hook's SDK (`@floorlaunchpad/sdk-solana` for FLOOR pools).
- **Slippage.** An exact-in swap's `amountLimit` is the minimum out and is required: `0` is refused unless
  `allowNoMinimum: true`.
- **Native SOL.** A wSOL side goes through a temporary wSOL account the builder creates, funds and closes in the same
  transaction.
- **Pauses.** A paused pool is refused `PoolPaused`. A guardian pause (`Pool.guardianPausedUntil`) needs the cluster's
  clock: check `poolPausedAt(pool, nowUnix)`.
- **Compute.** `CU_LIMITS` are estimates. Set the limit from a simulation's `unitsConsumed` when it matters.

Liquidity, pool creation, the wallet prompt table and the send caveats: [`packages/sdk/README.md`](packages/sdk/README.md).

## Addresses and interfaces

| Account | mainnet-beta (`MAINNET_CHAIN_KEY`) | devnet (`DEVNET_CHAIN_KEY`) |
|---|---|---|
| `carpenter_amm` (program) | `DdeiDx4ncHo9DXGZm1GrPe7mFKv7kNU3yx95uQEV2APA` | `Arv1M4LifBrWMywvxpeAGvh37aVHPjpXPCg7sacbe3Hy` |
| `amm_config` | `85S8wiRLyv9Urc6g9tF8gDC56F5Fq4JBu9Zub3BEtyuV` | `4MQRpJ85JE6FqBWFj47GcCoJpdFMVsLsMVPAq9865iiE` |

The shipped IDL and the generated client name the mainnet program. Devnet runs the same program source under its own id:
pass its row's ids. Machine-readable: [`packages/sdk/deployments.json`](packages/sdk/deployments.json)
(`@floorlaunchpad/carpenter-sdk/deployments.json`). FLOOR's program ids are in [`floor-sdk`](https://github.com/gudtekdonkey/floor-sdk#solana-programs).

| What | Where | Import |
|---|---|---|
| IDL (`carpenter_amm`) | [`packages/sdk/idl/carpenter_amm.json`](packages/sdk/idl/carpenter_amm.json) | `@floorlaunchpad/carpenter-sdk/idl/carpenter_amm.json` |
| Codama client | `packages/sdk/src/generated/` | `carpenterAmm` from `@floorlaunchpad/carpenter-sdk` |
| Account layouts (`AmmConfig`, `Pool`, `TickArray`, `Position`, the hook allowlist, the quote roster) | `packages/sdk/src/`, `crates/carpenter-types` | `decodePool`, … from `@floorlaunchpad/carpenter-sdk` |
| Events | `packages/sdk/src/events.ts` | `decodeInnerInstructionEvents`, `decodeEmitCpiData`, `decodeLogEvents`, `EVENT_NAMES` from `@floorlaunchpad/carpenter-sdk` |
| Quote vectors (golden and chain-measured) | [`fixtures/quote-vectors/`](fixtures/quote-vectors) | — |

The IDL ships without its `docs` fields and with a few error `msg` strings shortened; error names and codes are
unchanged. `IDL_SHA256` is its hash as shipped. Governance and admin instructions are in the IDL and the generated client
but have no builders.

## Repository

| Path | What |
|---|---|
| [`packages/sdk`](packages/sdk) | `@floorlaunchpad/carpenter-sdk`: IDL and Codama client, layouts, PDAs, reads, swap / liquidity / pool builders, the one-prompt sender, the plain-pool quoter (wasm) |
| [`crates/`](crates) | `carpenter-types` (layouts, seeds, constants, the hook interface), `carpenter-clmm-math` (Q64.64 CLMM math, see `NOTICE`), `carpenter-quote` (the network-free quoter, with `HookQuoter`) |
| [`wasm/`](wasm) | `carpenter-quote-wasm` and `build.sh`, which builds what `packages/sdk/wasm/` ships |
| [`jupiter/`](jupiter) | `carpenter-amm`: a `jupiter-amm-interface` `Amm` over the quoters; its own Cargo workspace |
| [`integrations/defillama/`](integrations/defillama) | Carpenter's DefiLlama adapters (TVL; volume and fees), kept here and copied into DefiLlama's repositories |
| [`fixtures/quote-vectors`](fixtures/quote-vectors) | golden quote vectors and chain-measured outcomes |

Building, testing, the quoter wasm and adding a cluster: [`CONTRIBUTING.md`](CONTRIBUTING.md). Changes:
[`CHANGELOG.md`](CHANGELOG.md). Reporting a vulnerability: [`SECURITY.md`](SECURITY.md).

## License

Apache-2.0 (`LICENSE`). `carpenter-clmm-math` is derived from Raydium's CLMM and Uniswap v4 and keeps its `NOTICE`.
