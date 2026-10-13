# DefiLlama adapters

Carpenter's DefiLlama adapters, in DefiLlama's own formats. Each file is copied as-is into a fork of DefiLlama's
repository, at the same path, and opened as a pull request there.

| File | Goes to | Measures |
|---|---|---|
| [`DefiLlama-Adapters/projects/carpenter/index.js`](DefiLlama-Adapters/projects/carpenter/index.js) | DefiLlama-Adapters | TVL on Solana |
| [`dimension-adapters/dexs/carpenter/index.ts`](dimension-adapters/dexs/carpenter/index.ts) | [DefiLlama/dimension-adapters](https://github.com/DefiLlama/dimension-adapters) | Volume and fees on Solana |
| [`PR-tvl.md`](PR-tvl.md), [`PR-volume.md`](PR-volume.md) | the two pull request bodies | |

## What it counts

Every `carpenter_amm` Pool account (found by its discriminator, `sha256("account:Pool")[..8]` = `f19a6d0411b16dbc`), and
the balances of its two token vaults (`vault0` / `vault1`, at offsets 152 and 184 after the 8-byte discriminator).

**Volume and fees** come from the `Swapped` event every swap emits through `emit_cpi!`, summed on Dune over
`solana.instruction_calls` (the program's own inner instructions, `EVENT_IX_TAG || sha256("event:Swapped")[..8] || borsh`).
Volume is one side of each swap: the SOL / USDC / USDT side where a pool has one, otherwise the input. Fees are the LP fee
(`lp_fee_total`, in the input token; supply-side revenue) and the protocol fee (`protocol_fee_total`, in the currency the
event names; revenue). A pool's mints are read from its account. Fees a pool's hook charges belong to the hook's
protocol and are not counted.

## Testing

From a checkout of DefiLlama-Adapters:

```sh
node test.js projects/carpenter/index.js

# dimension-adapters (needs DUNE_API_KEYS)
pnpm test dexs carpenter
```
