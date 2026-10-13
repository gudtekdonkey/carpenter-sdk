//! Conversions between Jupiter's types and the quoters'. No amount is computed here except the one
//! fee sum `Quote.fee_amount` reports, which never touches `in_amount` / `out_amount`.

use carpenter_quote::{QuoteError, SwapQuote};
use carpenter_types::layout::Pool;
use floor_types::layout::FloorPool;
use jupiter_amm_interface::{AmmError, Quote};
use rust_decimal::Decimal;
use solana_pubkey::Pubkey;

/// `[u8; 32]` → `Pubkey`.
pub fn pk(bytes: &[u8; 32]) -> Pubkey {
    Pubkey::new_from_array(*bytes)
}

/// `Pubkey` → `[u8; 32]`.
pub fn raw(key: &Pubkey) -> [u8; 32] {
    key.to_bytes()
}

/// A quoter refusal as Jupiter's error. The message is the `Display` of [`QuoteError`], which is the
/// on-chain error name (with the array start for the two missing-array refusals), so a caller sees
/// what the program would have reported.
pub fn quote_err(e: QuoteError) -> AmmError {
    AmmError::Custom(e.to_string())
}

/// Whether an error made by [`quote_err`] is `TickArrayMissing(start)` or
/// `HookActionAccountMissing(start)`: a walk that reached an array the adapter did not read. These
/// are the only refusals `missing_dynamic_accounts_as_default` may replace with a meta list.
pub fn is_missing_array(e: &AmmError) -> bool {
    let s = e.to_string();
    [QuoteError::TickArrayMissing { start_index: 0 }, QuoteError::HookActionAccountMissing { start_index: 0 }]
        .iter()
        .any(|k| s.starts_with(&format!("{}(", k.name())))
}

/// The swap direction from the pair of mints: `input == mint0` is `zero_for_one`.
pub fn direction(pool: &Pool, input_mint: &Pubkey, output_mint: &Pubkey) -> Result<bool, AmmError> {
    let (m0, m1) = (pk(&pool.mint0), pk(&pool.mint1));
    if *input_mint == m0 && *output_mint == m1 {
        Ok(true)
    } else if *input_mint == m1 && *output_mint == m0 {
        Ok(false)
    } else {
        Err(AmmError::Custom(format!("mints {input_mint} -> {output_mint} are not this pool's pair ({m0}, {m1})")))
    }
}

/// The `supply` of an SPL Token or Token-2022 mint: bytes 36..44 of the shared 82-byte base layout.
/// `None` when the data is too short to be a mint.
pub fn mint_supply(data: &[u8]) -> Option<u64> {
    let raw = data.get(36..44)?;
    Some(u64::from_le_bytes(raw.try_into().ok()?))
}

/// Jupiter's single `(fee_amount, fee_mint, fee_pct)` from a quote whose takes may sit in two
/// currencies.
///
/// ```text
/// fee_mint   = the input mint
/// fee_amount = every take charged in the input mint:
///              the LP fee                                  (always on the input)
///            + the hook's takes, when the input is the hook's quote currency
///              (FLOOR: skim + creator + holder + FLOOR Platform fee, `hook_total`)
///            + the Carpenter Platform fee, when it is charged in the input currency
/// fee_pct    = fee_amount / in_amount
/// ```
///
/// A take charged in the output currency has already reduced `out_amount` and is not added: adding
/// it would count it twice. On a FLOOR buy (quote in) every take is in the input; on a FLOOR sell the
/// input carries the LP fee only. `fee_amount` is informational; routing uses `in_amount` and
/// `out_amount`.
pub fn to_quote(q: &SwapQuote, hook_total: u64, zero_for_one: bool, pool: &Pool, floor: Option<&FloorPool>) -> Quote {
    let input_mint = if zero_for_one { pk(&pool.mint0) } else { pk(&pool.mint1) };
    let input_is_hook_quote = match floor {
        // The quote currency is token0 unless the FloorPool says token1.
        Some(fp) => zero_for_one == (fp.quote_is_token1 == 0),
        None => false,
    };
    let hook_in_input = if input_is_hook_quote { hook_total } else { 0 };
    let input_currency: u8 = if zero_for_one { 0 } else { 1 };
    let protocol_in_input = if q.fees.protocol_currency == input_currency { q.fees.protocol } else { 0 };
    let fee_amount = q.fees.lp.saturating_add(hook_in_input).saturating_add(protocol_in_input);
    let fee_pct = if q.amount_in == 0 { Decimal::ZERO } else { Decimal::from(fee_amount) / Decimal::from(q.amount_in) };
    Quote { in_amount: q.amount_in, out_amount: q.amount_out, fee_amount, fee_mint: input_mint, fee_pct }
}
