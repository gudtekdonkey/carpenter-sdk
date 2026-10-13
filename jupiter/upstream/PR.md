# Add `Swap::Carpenter`

Base: `jup-ag/jupiter-amm-interface` @ `95bd18485e580625a39eb92a0c0ad2061579daad`. Patch: `swap-variant.patch`
(one file, `interface/src/swap.rs`, additive).

## Change

One variant, appended after `Placeholder`; no existing variant moves.

```rust
Carpenter {
    zero_for_one: bool,
    hook_accounts: u8,
    tick_arrays: u8,
    remaining_accounts_info: Option<RemainingAccountsInfo>,
},
```

The variant may be placed anywhere in the enum (for example, to keep `Placeholder` last); nothing in
its field layout depends on its position.

## The program

`carpenter_amm` is a concentrated-liquidity AMM on Solana. A pool may name one hook program from the
AMM's allowlist; the hook runs as callbacks inside `swap`. The variant is shared by every approved
hook: it carries account counts, not a hook identifier, so a new approved hook needs no new variant.

`swap(zero_for_one: bool, amount_specified: i128, sqrt_price_limit_x64: u128, amount_limit: u64,
hook_data: Vec<u8>)`, exact-in: `amount_specified = -in_amount`, `amount_limit` = minimum out, no price
limit (the bound in the swap direction), empty `hook_data`. The instruction data is built from
`zero_for_one` and the amount alone; the three counts are not instruction data.

Accounts, in order:

| Range | Accounts | Count |
|---|---|---|
| 0–13 | `trader` (signer), `pool` (w), `bitmap` (w), `trader_token_in` (w), `trader_token_out` (w), `vault0` (w), `vault1` (w), `mint0`, `mint1`, `token_program0`, `token_program1`, `hook_program` (the program id itself when no callback runs), `event_authority`, `program` | 14 |
| then | the pool's hook slice (`Pool.hook_callback_accounts`; FLOOR's is `floor_pool` (w), the trader's bucket (w), `floor_hook`'s event authority, `floor_hook`) | `hook_accounts` (0 when no swap callback runs) |
| then | accounts the program resolves by key: tick arrays (the swap's walk and any walk a hook action makes), then any hook-owned position a hook action modifies | `tick_arrays` |
| last | Token-2022 transfer-hook accounts of the pool's base mint | `remaining_accounts_info`: one `TransferHookA` (mint0) or `TransferHookB` (mint1) slice; `None` when neither mint carries a transfer hook |

`remaining_accounts_info` follows the shape `WhirlpoolSwapV2` and `DefiTuna` already use.

## Properties relevant to routing

- Exact-in only (`supports_exact_out() == false`); a hooked pool refuses exact-out on chain.
- `has_dynamic_accounts() == true`: the number of tick arrays and positions depends on the amount and
  on the hook's actions. `get_accounts_len()` is an upper bound.
- The swap is a plain top-level instruction; no instruction-sysvar introspection, one signer.

## Verification

The SDK side (the `carpenter-amm` adapter crate) builds against this patch
behind a cargo feature, emits the variant from `get_swap_and_account_metas`, and checks that:

- the counts delimit the emitted account list exactly (plain pools, hooked pools, and a pool whose
  base mint carries a Token-2022 transfer hook);
- the variant encodes to the same instruction data as the `Placeholder` encoding it replaces;
- `jupiter-amm-test-kit` parity passes with the variant: snapshots of devnet pools, the deployed
  program binaries in LiteSVM, the output delta equal to `quote.out_amount` for buys and sells of
  several sizes.
