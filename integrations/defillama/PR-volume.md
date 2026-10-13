##### Name (to be shown on DefiLlama):
Carpenter

##### Chain:
Solana

##### Category:
Dexs

##### Methodology:
From the `Swapped` event every Carpenter swap emits (an `emit_cpi!` inner instruction of the `carpenter_amm` program, `DdeiDx4ncHo9DXGZm1GrPe7mFKv7kNU3yx95uQEV2APA`), summed on Dune:

- **Volume**: one side of each swap: the SOL / USDC / USDT side where a pool has one, otherwise the input.
- **Fees**: the LP fee (`lp_fee_total`, in the input token) plus the protocol fee (`protocol_fee_total`).
- **Revenue**: the protocol fee.
- **Supply-side revenue**: the LP fee, paid to liquidity providers.

Fees charged by a pool's hook belong to the hook's protocol and are not counted here.
