# DefiLlama adapter

Carpenter's DefiLlama TVL adapter, in DefiLlama's own format. The file is copied as-is into a fork of
[DefiLlama/DefiLlama-Adapters](https://github.com/DefiLlama/DefiLlama-Adapters), at the same path, and opened as a pull
request there.

| File | Goes to | Measures |
|---|---|---|
| [`DefiLlama-Adapters/projects/carpenter/index.js`](DefiLlama-Adapters/projects/carpenter/index.js) | DefiLlama-Adapters | TVL on Solana |
| [`PR-tvl.md`](PR-tvl.md) | the pull request body | |

## What it counts

Every `carpenter_amm` Pool account (found by its discriminator, `sha256("account:Pool")[..8]` = `f19a6d0411b16dbc`), and
the balances of its two token vaults (`vault0` / `vault1`, at offsets 152 and 184 after the 8-byte discriminator).

Volume is not covered yet. The Pool account keeps lifetime `volume0` / `volume1` counters, but a DefiLlama volume adapter
needs per-window figures, so it has to read swaps instead.

## Testing

From a checkout of DefiLlama-Adapters:

```sh
node test.js projects/carpenter/index.js
```
