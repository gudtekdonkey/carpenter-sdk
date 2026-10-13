//! The `Swap` value this adapter emits, and the native `carpenter_amm::swap` instruction data built
//! from it.
//!
//! The interface's `Swap` enum has no Carpenter variant at the pinned commit, so by default the
//! adapter returns `Swap::Placeholder { data }`, the interface's variant for venues that have no
//! variant of their own. `data` is one byte, `zero_for_one`; everything else is in the account metas
//! or derived from the amount by [`swap_instruction_data`]. A `Placeholder` swap serves the test kit
//! and any caller that encodes the instruction itself.
//!
//! With the `upstream-variant` feature, built against an interface carrying
//! `upstream/swap-variant.patch`, the adapter emits `Swap::Carpenter { zero_for_one, hook_accounts,
//! tick_arrays, remaining_accounts_info }` instead (see `upstream/PR.md`). Both encode to the same
//! instruction data.

use carpenter_quote::no_limit;
use carpenter_types::disc;
use jupiter_amm_interface::{AmmError, Swap};

/// The account counts of one swap's account list beyond the 14 fixed accounts, in order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwapCounts {
    /// The hook slice (`Pool.hook_callback_accounts` when a swap callback runs, else 0).
    pub hook_accounts: u8,
    /// The accounts the program resolves by key: tick arrays, then any hook position an action
    /// modifies.
    pub tick_arrays: u8,
    /// The base mint's transfer-hook accounts appended last: `(mint side 0 / 1, count)`.
    pub transfer_hook: Option<(u8, u8)>,
}

/// `Swap::Placeholder { data: [zero_for_one] }`.
pub fn placeholder(zero_for_one: bool) -> Swap {
    Swap::Placeholder { data: Some(vec![zero_for_one as u8]) }
}

/// The `Swap` this build emits: [`placeholder`] by default; `Swap::Carpenter` with the
/// `upstream-variant` feature.
#[cfg(not(feature = "upstream-variant"))]
pub fn swap_variant(zero_for_one: bool, _counts: SwapCounts) -> Swap {
    placeholder(zero_for_one)
}

/// The `Swap` this build emits: `Swap::Carpenter` with the counts; the transfer-hook accounts are
/// one `TransferHookA` (mint0) or `TransferHookB` (mint1) slice.
#[cfg(feature = "upstream-variant")]
pub fn swap_variant(zero_for_one: bool, counts: SwapCounts) -> Swap {
    use jupiter_amm_interface::{AccountsType, RemainingAccountsInfo, RemainingAccountsSlice};
    let remaining_accounts_info = counts.transfer_hook.filter(|(_, n)| *n > 0).map(|(side, length)| RemainingAccountsInfo {
        slices: vec![RemainingAccountsSlice {
            accounts_type: if side == 0 { AccountsType::TransferHookA } else { AccountsType::TransferHookB },
            length,
        }],
    });
    Swap::Carpenter { zero_for_one, hook_accounts: counts.hook_accounts, tick_arrays: counts.tick_arrays, remaining_accounts_info }
}

/// Read `zero_for_one` back out of a [`placeholder`] (or, with `upstream-variant`, a
/// `Swap::Carpenter`).
pub fn direction_of(swap: &Swap) -> Result<bool, AmmError> {
    match swap {
        Swap::Placeholder { data: Some(d) } if d.len() == 1 && d[0] <= 1 => Ok(d[0] == 1),
        Swap::Placeholder { data } => Err(AmmError::Custom(format!("carpenter-amm: Placeholder data must be one byte (zero_for_one), got {data:?}"))),
        #[cfg(feature = "upstream-variant")]
        Swap::Carpenter { zero_for_one, .. } => Ok(*zero_for_one),
        other => Err(AmmError::Custom(format!("carpenter-amm: only Swap::Placeholder or Swap::Carpenter is encoded, got {other:?}"))),
    }
}

/// The Anchor / Borsh data of `swap(zero_for_one, amount_specified, sqrt_price_limit_x64,
/// amount_limit, hook_data)`:
///
/// ```text
/// discriminator [8] ‖ zero_for_one u8 ‖ amount_specified i128 LE ‖ sqrt_price_limit_x64 u128 LE
///                   ‖ amount_limit u64 LE ‖ hook_data (u32 LE length 0)
/// ```
///
/// Exact-in only: `amount_specified = -in_amount`, the price limit is [`no_limit`] in the swap
/// direction, and `amount_limit` (the minimum out) is `min_out`. Exact-out would need `+amount` and a
/// maximum in, which neither variant carries; `supports_exact_out()` is false for the same reason.
pub fn swap_instruction_data(swap: &Swap, in_amount: u64, min_out: u64) -> Result<Vec<u8>, AmmError> {
    let zero_for_one = direction_of(swap)?;
    let amount_specified: i128 = -i128::from(in_amount);
    let mut data = Vec::with_capacity(8 + 1 + 16 + 16 + 8 + 4);
    data.extend_from_slice(&disc::amm::SWAP);
    data.push(zero_for_one as u8);
    data.extend_from_slice(&amount_specified.to_le_bytes());
    data.extend_from_slice(&no_limit(zero_for_one).to_le_bytes());
    data.extend_from_slice(&min_out.to_le_bytes());
    data.extend_from_slice(&0u32.to_le_bytes());
    Ok(data)
}
