# @floorsupply/carpenter-sdk

TypeScript SDK for Carpenter, a concentrated-liquidity AMM on Solana with Uniswap-v4-style hooks: pools keyed by
`(mint0, mint1, fee, tickSpacing, hook)`, outside LPs, and an allowlist of approved hook programs. Pure TypeScript over
[`@solana/kit`](https://github.com/anza-xyz/kit) 8.3.0; nothing here signs or sends without a wallet you pass in.

It covers reading, quoting and trading:

- the `carpenter_amm` IDL and its [Codama](https://github.com/codama-idl/codama) client (`carpenterAmm`);
- account layouts and decoders (`AmmConfig`, `Pool`, `TickArray`, `Position`, the hook allowlist, the quote roster),
  PDAs, and read helpers that tag every read with its slot (`stateView`);
- transaction builders: swap, swap-from-balance, add / remove liquidity and collect fees (`buildModifyLiquidity`),
  close a position, tick arrays, initialize a pool, and `planCreatePool` (create a pool and its first position);
- v0 messages through address lookup tables, with the 1,232-byte packet checked before a wallet sees anything;
- `sendSolanaTransactions`: one wallet prompt for every transaction of one action;
- tick math, the protocol-fee arithmetic, event decoders, and plain-pool quotes through the bundled wasm quoter
  (`wasm/carpenter_quote_bg.wasm`, the program's own quoting crate compiled to wasm).

Governance and admin instructions (config setters, hook approval, pausing, fee withdrawal) are in the IDL and the
generated client but have no builders here.

## Install

```sh
npm install @floorsupply/carpenter-sdk @solana/kit@8.3.0
```

`@solana/kit` is a peer dependency.

## Read a pool

```ts
import { createSolanaRpc } from '@solana/kit';
import { deploymentFor, fromKitRpc, MAINNET_CHAIN_KEY, programIdsFromDeployment, stateView } from '@floorsupply/carpenter-sdk';

const rpc = createSolanaRpc('https://api.mainnet-beta.solana.com');
const src = fromKitRpc(rpc);
const ids = programIdsFromDeployment(deploymentFor(MAINNET_CHAIN_KEY)); // DEVNET_CHAIN_KEY for devnet

const { slot, value: pool } = await stateView.getPool(src, poolAddress, ids);
```

Program ids come from `deployments.json` (`deploymentFor`), never from a constant in an app; `DEFAULT_PROGRAM_IDS` exists
for tests and scripts.

## Quote and swap a plain pool

```ts
import { readFileSync } from 'node:fs';
import { createRequire } from 'node:module';
import { buildSwap, compileSendable, decodePool, readSwapQuoteAccounts, sendSolanaTransactions, swapTickArrays } from '@floorsupply/carpenter-sdk';
import { initQuote } from '@floorsupply/carpenter-sdk/quote/wasm';
import { getBase64EncodedWireTransaction } from '@solana/kit';

// Node: read the bundled wasm; a browser fetches `@floorsupply/carpenter-sdk/carpenter_quote_bg.wasm` and passes the bytes.
const quoter = await initQuote(readFileSync(createRequire(import.meta.url).resolve('@floorsupply/carpenter-sdk/carpenter_quote_bg.wasm')));
const { value: v } = await readSwapQuoteAccounts(src, poolAddress, { radius: 4, ids });
const q = quoter.quoteSwap({ ...v, zeroForOne: true, amountSpecified: -1_000_000n }); // exact in: negative
const { starts } = await swapTickArrays(poolAddress, q, ids);

const tx = await buildSwap({
  trader, pool: poolAddress, poolState: decodePool(v.pool), zeroForOne: true,
  amountSpecified: -1_000_000n, amountLimit: (q.amountOut * 99n) / 100n, tickArrayStarts: starts, ids,
});
const { value: bh } = await rpc.getLatestBlockhash().send();
const { transaction } = compileSendable('swap', { feePayer: trader, instructions: tx.instructions, lifetime: bh });
await sendSolanaTransactions(wallet, [getBase64EncodedWireTransaction(transaction)], rpc, { lastValidBlockHeight: bh.lastValidBlockHeight });
```

The quoter is the program's own quoting crate (`carpenter-quote`) compiled to wasm and shipped in this package
(`wasm/`, `WASM_SHA256`); every number a quote returns is the crate's. It quotes pools with no swap hook: a pool whose
hook takes part in swaps is refused (`QuoteHookNotQuotable`) and is quoted by its hook's SDK (`@floorsupply/sdk-solana` for
FLOOR pools). A paused pool is refused `PoolPaused`; a guardian pause (`Pool.guardianPausedUntil`, a unix time) is not
judged by the quoter, which reads no clock: check `poolPausedAt(pool, nowUnix)` against the cluster's clock.
`simulateTransaction` stays the final check before signing.

Slippage: an exact-in swap's `amountLimit` is the minimum out and is required (a `0` is refused unless
`allowNoMinimum: true`); derive it from the quote (above: 1 %). `buildModifyLiquidity` refuses a removal with neither
`amount0Min` nor `amount1Min`, and a deposit whose maximum is `u64::MAX`, under the same opt-out.

Compute: `CU_LIMITS` (`swapPlain` for a plain swap, 90,000) are estimates, not measurements of your transaction.
Simulate first and set the limit from the simulation's `unitsConsumed` when it matters.

Native SOL: a wSOL side is moved through a temporary wSOL account that the builder creates, funds and closes inside the
same transaction (`tempWsol`, a random seed under the trader's key). The trader's own wSOL account, if any, is neither
used nor closed, so wSOL already held is left as it is. Name an account (`traderTokenIn` / `traderTokenOut`,
`ownerToken0` / `ownerToken1`) to use it instead (it is topped up for an input, never closed), or pass `wrapSol: false`
to use the wSOL ATA with no wrapping at all.

A hooked pool needs its hook's accounts (`hookSlice`) and compute (`cuUnits`): the hook's own SDK provides them
(`@floorsupply/sdk-solana` for FLOOR pools). `readSwapQuoteAccounts` refuses a hooked pool by name.

FLOOR pools use the four-account FLOOR hook slice, which the mainnet and devnet deployments this release lists carry
(`@floorsupply/sdk-solana` of the same release builds it). A hook that settles (`HOOK_FLAGS.SETTLES`) is called under the
`["hook_settle", pool]` signer, which leads its slice (`findHookSettle`); pass `writableMints` when it mints into a vault
during the swap.

## Create a pool and add liquidity

```ts
import { fromKitRpc, planCreatePool, sendSolanaTransactions, NO_HOOK } from '@floorsupply/carpenter-sdk';

const plan = await planCreatePool({
  payer, mintA, mintB, tokenProgramA, tokenProgramB, decimalsA: 6, decimalsB: 9,
  fee: 3_000, tickSpacing: 60, hook: NO_HOOK, sqrtPriceX64,
  position: { tickLower: -600, tickUpper: 600, liquidity, amountAMax, amountBMax },
  lifetime: bh, ids,
});
await sendSolanaTransactions(wallet, plan.transactions, rpc, {
  steps: plan.stepNames,
  lastValidBlockHeight: bh.lastValidBlockHeight,
  beforeSend: plan.beforeSend(fromKitRpc(rpc)), // stop if the new pool's price moved before the deposit
});
```

`plan.transactions` is `[initializePool, addLiquidity]`: two transactions (together they do not fit one packet), signed
with one prompt and sent in order. One wallet signs both, so the position's owner is the payer (`owner` / `initializer`
other than `payer` are refused).

Between the two steps the pool exists and is empty, and anyone can move an empty pool's price for free.
`plan.beforeSend(src, { maxPriceMoveBps })` reads the pool just before the deposit is sent and stops the run (reason
`guard`, `PoolPriceMovedError`) when its price moved from the initialized price by more than `maxPriceMoveBps`
(default 0: exact). The price can still move between that read and the deposit, so `amountAMax` / `amountBMax` are the
deposit's only hard bound: set them to what the position needs at the initialized price plus the slippage you accept,
never larger.

## Wallet prompts per action

`sendSolanaTransactions(wallet, steps, rpc, opts?)` signs every transaction of one action together, then sends them in
order, waiting for each confirmation before the next; the first failure stops the run and names the step
(`SolanaSendError`: `index`, `step`, `reason`, the signatures that landed).

| Action | Transactions | Prompts (batch-signing wallet) |
|---|---|---|
| Swap | 1 | 1 |
| Add / remove liquidity, collect fees | 1 | 1 |
| Create a pool with its first position (`planCreatePool`) | 2 | 1 |

How the wallet is asked, in order of preference: the Wallet Standard `solana:signTransaction` feature with every
transaction in one call (`wallet.standard`), a legacy `signAllTransactions`, else `signTransaction` once per step. The
result says which (`path`) and how many prompts it took.

Caveats:

- Signing the transactions one by one costs one prompt each; a wallet without a batch call falls back to that.
- Every transaction the wallet returns is checked before anything is sent: the message bytes must equal the planned
  message, and every required signature (the expected signer's included: `opts.signer`, else the Wallet Standard
  account, else the fee payer) must verify. Anything else stops the run with reason `sign`.
- A step is never sent twice. A send that errors without a preflight refusal (a timeout, a dropped connection) may have
  delivered the transaction, so the step keeps waiting on its signature. A step counts as expired only when its
  signature is absent, the block height has passed the blockhash's `lastValidBlockHeight` (pass it, or the sender asks
  the node), two more blocks have passed, a `searchTransactionHistory` re-check still finds nothing and the node calls
  the blockhash invalid. "Blockhash not found" alone is not expiry.
- After a failure, re-plan from chain state: the steps that landed changed it. A `timeout` may still land: check its
  `signature` before acting again.
- The transactions share one recent blockhash, valid for roughly 60–90 seconds. Pass `opts.replan` to rebuild the steps
  that have not landed when it expires (one more prompt); without it the run stops with reason `expired`.
- A wallet that simulates each transaction before signing cannot simulate a step that depends on an account an earlier
  step creates (the pool in `planCreatePool`, a lookup table): it shows that step as failing, and it lands once the
  earlier step has.

## Develop

`idl/carpenter_amm.json` is the program's Anchor IDL with every `docs` field removed and a few error `msg` strings
shortened (the error names and codes are unchanged): the shipped IDL, and the
generated client built from it, carry no doc comments. `IDL_SHA256` is the hash of that file as shipped.

```sh
npm run generate   # idl/ + deployments.json → src/generated/ (Codama client)
npx tsc -p tsconfig.json --noEmit
npx vitest run
npm run build      # dist/
npm run sync-wasm  # dist/wasm (from `bash wasm/build.sh` at the repository root) → wasm/ and WASM_SHA256
```

The wasm is built from `wasm/carpenter-quote-wasm` by `wasm/build.sh` at the repository root: rustc 1.98.1
(`wasm32-unknown-unknown`), wasm-bindgen-cli 0.2.128, `--target web`, on a copy of the sources at a fixed path with
paths remapped and the crate's doc comments removed, so the same sources give the same bytes. `test/wasm.test.ts`
replays `fixtures/quote-vectors/plain.json` and `parity-plain.json` through the shipped wasm.

The programs are upgradeable through a timelock on mainnet-beta (a 10-minute delay that can only be raised, proposed by the deployer key) and by the deployer key directly on devnet, and pausable by the deployer key; this package makes no claim that they are audited or immutable.

## License

Apache-2.0.
