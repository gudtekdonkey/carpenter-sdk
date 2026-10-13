//! The extra accounts of a swap whose base mint carries a Token-2022 transfer hook the pool admits.
//!
//! `carpenter_amm` moves at most one armed transfer hook, on the pool's base leg: the program
//! recorded in `Pool.base_transfer_hook` (on side `Pool.base_hook_side`), and only while the mint's
//! hook authority is renounced. The program finds the hook's accounts by key among the swap's
//! remaining accounts, so a swap built here carries the hook program, its
//! `["extra-account-metas", mint]` validation account, and every account that list resolves to for
//! this transfer.
//!
//! The accounts are resolved from the list itself, the way `spl-tlv-account-resolution` resolves
//! them, not from a hard-coded shape. Entry discriminators `0` (a literal key), `1` (a PDA of the
//! hook program) and `128 + i` (a PDA of account `i`) are resolved with the seeds `Literal`,
//! `InstructionData` and `AccountKey`. An `AccountData` seed or a `PubkeyData` entry would need
//! account bytes at routing time; such a list is refused and the pool reported inactive.
//!
//! An account the swap already carries is not appended again; its writability is widened instead
//! (a duplicate `floor_pool` would alias the hook slice, which the program refuses). No signer flag is
//! ever widened.

use solana_instruction::AccountMeta;
use solana_pubkey::Pubkey;

/// `sha256("spl-transfer-hook-interface:execute")[..8]`.
pub const EXECUTE_DISCRIMINATOR: [u8; 8] = [105, 37, 101, 197, 75, 251, 102, 26];

/// The validation account's seed prefix, under the hook program.
pub const EXTRA_ACCOUNT_METAS_SEED: &[u8] = b"extra-account-metas";

/// `ExtensionType::TransferHook` (spl-token-2022, `repr(u16)`).
const EXT_TRANSFER_HOOK: u16 = 14;
/// A Token-2022 mint's `AccountType` byte and where its TLV starts (after the base account length).
const ACCOUNT_TYPE_OFFSET: usize = 165;
const ACCOUNT_TYPE_MINT: u8 = 1;
const META_LEN: usize = 35;

/// A mint's `TransferHook` extension: `(authority, program_id)`, zero = none. `None` when the mint
/// carries no such extension (every classic SPL mint, and a Token-2022 mint without the slot).
pub fn transfer_hook_of(mint_data: &[u8]) -> Option<([u8; 32], [u8; 32])> {
    if mint_data.len() <= ACCOUNT_TYPE_OFFSET || mint_data[ACCOUNT_TYPE_OFFSET] != ACCOUNT_TYPE_MINT {
        return None;
    }
    let mut at = ACCOUNT_TYPE_OFFSET + 1;
    while at + 4 <= mint_data.len() {
        let ty = u16::from_le_bytes([mint_data[at], mint_data[at + 1]]);
        let len = u16::from_le_bytes([mint_data[at + 2], mint_data[at + 3]]) as usize;
        let value = mint_data.get(at + 4..at + 4 + len)?;
        if ty == 0 {
            return None;
        }
        if ty == EXT_TRANSFER_HOOK && len >= 64 {
            return Some((value[..32].try_into().ok()?, value[32..64].try_into().ok()?));
        }
        at += 4 + len;
    }
    None
}

/// `["extra-account-metas", mint]` under `program`.
pub fn extra_account_metas_pda(mint: &Pubkey, program: &Pubkey) -> Pubkey {
    Pubkey::find_program_address(&[EXTRA_ACCOUNT_METAS_SEED, mint.as_ref()], program).0
}

/// One entry of the validation list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RawMeta {
    pub discriminator: u8,
    pub address_config: [u8; 32],
    pub is_signer: bool,
    pub is_writable: bool,
}

/// The `Execute` entry of a validation account's TLV data (`spl-type-length-value` layout:
/// `[discriminator 8][length u32 LE][value]`, the value `count u32 LE` then `count` 35-byte metas).
pub fn execute_metas(data: &[u8]) -> Option<Vec<RawMeta>> {
    let mut at = 0usize;
    while at + 12 <= data.len() {
        let disc: [u8; 8] = data[at..at + 8].try_into().ok()?;
        if disc == [0u8; 8] {
            return None;
        }
        let len = u32::from_le_bytes(data[at + 8..at + 12].try_into().ok()?) as usize;
        let value = data.get(at + 12..(at + 12).checked_add(len)?)?;
        if disc == EXECUTE_DISCRIMINATOR {
            let count = u32::from_le_bytes(value.get(..4)?.try_into().ok()?) as usize;
            let body = value.get(4..4usize.checked_add(count.checked_mul(META_LEN)?)?)?;
            return body
                .chunks_exact(META_LEN)
                .map(|m| {
                    Some(RawMeta {
                        discriminator: m[0],
                        address_config: m[1..33].try_into().ok()?,
                        is_signer: m[33] != 0,
                        is_writable: m[34] != 0,
                    })
                })
                .collect();
        }
        at += 12 + len;
    }
    None
}

/// Why a list cannot be resolved off-chain; the pool is then inactive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResolveErr {
    /// A seed kind or entry discriminator this resolver does not read (`AccountData`, `PubkeyData`,
    /// or one the format does not have), or seeds that run past their 32 bytes.
    Unsupported,
    /// An index past the accounts resolved so far, or instruction data past its end.
    OutOfRange,
    /// No bump makes the seeds a valid PDA.
    NoAddress,
}

impl core::fmt::Display for ResolveErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            ResolveErr::Unsupported => "the transfer hook's extra-account-metas list uses a seed or entry kind this adapter does not resolve",
            ResolveErr::OutOfRange => "the transfer hook's extra-account-metas list names an account or data past its range",
            ResolveErr::NoAddress => "the transfer hook's extra-account-metas list resolves to no valid PDA",
        })
    }
}

/// The accounts a hooked transfer appends — every resolved entry (de-escalated against the
/// `Execute` accounts, as `add_extra_accounts_for_execute_cpi` does), then the validation account,
/// then the hook program — for one transfer `source → destination` of `amount` by `authority`.
#[allow(clippy::too_many_arguments)]
pub fn hook_account_metas(
    list: &[RawMeta],
    hook_program: &Pubkey,
    validation: &Pubkey,
    source: &Pubkey,
    mint: &Pubkey,
    destination: &Pubkey,
    authority: &Pubkey,
    amount: u64,
) -> Result<Vec<AccountMeta>, ResolveErr> {
    let mut ix_data = EXECUTE_DISCRIMINATOR.to_vec();
    ix_data.extend_from_slice(&amount.to_le_bytes());
    let mut exec: Vec<AccountMeta> = vec![
        AccountMeta::new_readonly(*source, false),
        AccountMeta::new_readonly(*mint, false),
        AccountMeta::new_readonly(*destination, false),
        AccountMeta::new_readonly(*authority, false),
        AccountMeta::new_readonly(*validation, false),
    ];
    for m in list {
        let key = resolve(m, &ix_data, hook_program, &exec)?;
        let mut meta = AccountMeta { pubkey: key, is_signer: m.is_signer, is_writable: m.is_writable };
        let prior: Vec<&AccountMeta> = exec.iter().filter(|e| e.pubkey == key).collect();
        if !prior.is_empty() {
            meta.is_signer &= prior.iter().any(|e| e.is_signer);
            meta.is_writable &= prior.iter().any(|e| e.is_writable);
        }
        exec.push(meta);
    }
    let mut out: Vec<AccountMeta> = exec.split_off(5);
    out.push(AccountMeta::new_readonly(*validation, false));
    out.push(AccountMeta::new_readonly(*hook_program, false));
    Ok(out)
}

fn resolve(m: &RawMeta, ix_data: &[u8], hook_program: &Pubkey, exec: &[AccountMeta]) -> Result<Pubkey, ResolveErr> {
    let program = match m.discriminator {
        0 => return Ok(Pubkey::new_from_array(m.address_config)),
        1 => *hook_program,
        d if d >= 128 => exec.get((d - 128) as usize).ok_or(ResolveErr::OutOfRange)?.pubkey,
        _ => return Err(ResolveErr::Unsupported),
    };
    let c = &m.address_config;
    let mut seeds: Vec<Vec<u8>> = Vec::new();
    let mut i = 0usize;
    while i < c.len() {
        match c[i] {
            0 => break,
            1 => {
                let len = *c.get(i + 1).ok_or(ResolveErr::Unsupported)? as usize;
                seeds.push(c.get(i + 2..i + 2 + len).ok_or(ResolveErr::Unsupported)?.to_vec());
                i += 2 + len;
            }
            2 => {
                let (index, length) = (*c.get(i + 1).ok_or(ResolveErr::Unsupported)? as usize, *c.get(i + 2).ok_or(ResolveErr::Unsupported)? as usize);
                seeds.push(ix_data.get(index..index + length).ok_or(ResolveErr::OutOfRange)?.to_vec());
                i += 3;
            }
            3 => {
                let index = *c.get(i + 1).ok_or(ResolveErr::Unsupported)? as usize;
                seeds.push(exec.get(index).ok_or(ResolveErr::OutOfRange)?.pubkey.to_bytes().to_vec());
                i += 2;
            }
            // 4 = AccountData: an account's bytes at routing time, which the adapter does not hold.
            _ => return Err(ResolveErr::Unsupported),
        }
    }
    let refs: Vec<&[u8]> = seeds.iter().map(|s| s.as_slice()).collect();
    Pubkey::try_find_program_address(&refs, &program).map(|(k, _)| k).ok_or(ResolveErr::NoAddress)
}

/// Append `hook` metas to a swap's metas: a key already present is widened to the list's
/// writability, a new one is appended (see the module docs). A signer flag is never widened:
/// the router cannot sign for an account the swap does not already sign with, and a list that
/// demands one is a transfer the chain would refuse anyway.
pub fn merge_metas(metas: &mut Vec<AccountMeta>, hook: Vec<AccountMeta>) {
    for h in hook {
        match metas.iter_mut().find(|m| m.pubkey == h.pubkey) {
            Some(m) => m.is_writable |= h.is_writable,
            None => metas.push(h),
        }
    }
}
