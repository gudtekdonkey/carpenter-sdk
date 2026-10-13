# Carpenter — Rust crates

Carpenter is a concentrated-liquidity AMM on Solana shaped like Uniswap v4: pools keyed by their two
mints, fee, tick spacing and hook; outside LPs; and an allowlist of approved hook programs that can
take part in a swap. These crates are its off-chain half: the account layouts, the CLMM math the
program runs, and a quoter that reproduces a swap from account bytes, to the unit, with no network
access.

| Crate | What it answers | Normal dependencies |
|---|---|---|
| `carpenter-types` | Account layouts (`Pool`, `TickArray`, the bitmap, `Position`, `ApprovedHook`, …) as `#[repr(C)]` `Pod` structs with size and offset asserts; PDA seeds; constants; instruction, callback and account discriminators; the program id; the hook interface (`HookRecordV1`, the callback args, the record rules); the Carpenter Platform fee arithmetic | `borsh`, `bytemuck` |
| `carpenter-clmm-math` | Q64.64 tick math, sqrt-price math, liquidity math, the swap step, the tick-array bitmap. Derived from Raydium CLMM (Apache-2.0) with Uniswap v4-core behaviour re-expressed (MIT) — see `NOTICE` | `uint =0.9.5` |
| `carpenter-quote` | `quote_swap` (the swap round protocol, plain and hooked pools), the `HookQuoter` interface for approved hooks, `amounts_for_liquidity` / `quote_liquidity` / `fees_owed`, price ↔ tick, account decoding, synthetic pools | `carpenter-types`, `carpenter-clmm-math`, `bytemuck` |

No crate depends on a Solana crate: keys are `[u8; 32]`, and the same bytes give the same answer on
every target, `wasm32` included.

## Quoting a swap

```rust
use carpenter_quote::state::{decode_pool, TickArraySet};
use carpenter_quote::{no_limit, quote_plain, PoolAccount, SwapQuoteParams};

let pool = decode_pool(&pool_account_bytes)?;
let set = TickArraySet::from_accounts(&pool_key, pool.tick_spacing, &tick_array_accounts, Some(&bitmap_account))?;
let q = quote_plain(
    &PoolAccount { key: pool_key, pool },
    &set,
    &SwapQuoteParams { zero_for_one: true, amount_specified: -1_000_000, sqrt_price_limit_x64: no_limit(true), supply0, supply1 },
)?;
// q.amount_in, q.amount_out, q.fees, q.all_arrays() (the tick arrays the transaction must carry), q.pool_after, …
```

`amount_specified` follows v4's sign: negative is exact-in, positive is exact-out.

## Hooked pools: `HookQuoter`

A pool whose hook sets a swap flag cannot be quoted without that hook's logic, and nothing can be
simulated off chain. So each approved hook that changes swaps ships a Rust replay of its callbacks,
implementing `carpenter_quote::HookQuoter`:

```rust
pub trait HookQuoter {
    fn program_id(&self) -> [u8; 32];
    // Default `program_id() == *hook`; a hook with a different program id per cluster accepts each.
    fn accepts(&self, hook: &[u8; 32]) -> bool { self.program_id() == *hook }
    fn prepare(&mut self, pool: &PoolAccount) -> QuoteResult<()> { Ok(()) }
    fn before_swap(&mut self, ctx: &HookSwapCtx) -> QuoteResult<HookRecordV1>;
    fn after_swap(&mut self, ctx: &HookSwapCtx, after: &AfterSwapIn) -> QuoteResult<HookRecordV1>;
    fn after_actions(&mut self, ctx: &HookSwapCtx, round: u8, results: &[ActionResult],
                     results_carry_fees: bool, claims0: u64, claims1: u64) -> QuoteResult<HookRecordV1>;
}
```

`quote_swap(pool, arrays, Some(&mut quoter), params)` (or `quote_swap_with(pool, arrays, &mut [..quoters], params)`)
calls it at the points the AMM calls the hook program, validates each record by the AMM's rules,
applies its deltas once, and executes the actions it requests (`ModifyPosition`, `Swap`) in-process.

| Pool | Result |
|---|---|
| no hook, or a hook with no swap flag | the plain walk; a quoter passed in is not called |
| a swap hook with a quoter that `accepts` it | the round protocol through that quoter |
| a swap hook with no quoter for it | refused: `QuoteHookNotQuotable` — never quoted as plain |

FLOOR's hook quoter is the `floor-quote` crate in [`floor-sdk`](https://github.com/gudtekdonkey/floor-sdk).

## Golden vectors

`fixtures/quote-vectors/` (at the repository root; the tests reach it as `../../fixtures/…`):

| File | What the expectation is | Test |
|---|---|---|
| `plain.json` | the quoter's own answer on five synthetic plain pools, 13 swaps (one refusal: exact output at a 100 % fee) | `carpenter-quote/tests/vectors.rs` |
| `parity-plain.json` | 40 plain swaps whose outcome an SVM run of the `carpenter_amm` program agreed with to the unit (35 answered, 5 refused), with the account bytes each was quoted from | `carpenter-quote/tests/parity_plain.rs` |

Each account is lowercase hex of the full account data (discriminator included); integers above 2^53
are decimal strings; an expectation is `{"ok": {…}}` or `{"err": "<chain error name>"}`.

## Building

```bash
cargo test --workspace
cargo tree --workspace -e normal --depth 1   # borsh, bytemuck, uint and the sibling crates, nothing else
```

The toolchain is pinned in `rust-toolchain.toml`. The crates are `publish = false`; depend on them by
git tag.

## Status

No program built from this code is deployed on mainnet. The program id in `carpenter-types` is the
current devnet deployment's and changes at a fresh deploy. The programs are
upgradeable and pausable by their deployer key. No security audit of the programs or of these crates
has been completed.

## Licence

Apache-2.0 (`LICENSE`). `carpenter-clmm-math` is `Apache-2.0 AND MIT`; its attributions are in
`NOTICE` and `THIRD_PARTY.md`.
