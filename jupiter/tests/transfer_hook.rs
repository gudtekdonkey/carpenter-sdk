//! A pool whose base mint is armed with the Token-2022 transfer hook the pool admits
//! (`Pool.base_transfer_hook`): the adapter judges the mint from the bytes `update` read, and a swap
//! carries the hook's accounts, resolved from the hook's own `extra-account-metas` list for the one
//! leg that moves the base.
//!
//! The list used here has the shape `floor_holders` writes — `["holder_book", mint]` writable, a
//! literal `floor_pool` read-only, `["holder", mint, source]` and `["holder", mint, destination]`
//! writable — under a stand-in program id.

mod common;

use carpenter_amm::transfer_hook::{execute_metas, extra_account_metas_pda, hook_account_metas, transfer_hook_of, RawMeta, ResolveErr, EXECUTE_DISCRIMINATOR};
use carpenter_amm::{BaseHook, CarpenterAmm};
use common::*;
use floor_types::constants as fc;
use jupiter_amm_interface::{Amm, SwapMode, SwapParams};
use solana_instruction::AccountMeta;
use solana_pubkey::Pubkey;

/// The hook program's stand-in id (the adapter never executes it).
const HOLDERS: [u8; 32] = [0x71; 32];
const SOURCE: Pubkey = Pubkey::new_from_array([0xA1; 32]);
const DESTINATION: Pubkey = Pubkey::new_from_array([0xA2; 32]);

/// A Token-2022 mint (the SPL base layout, padded to 165, `AccountType::Mint`) carrying one
/// `TransferHook` extension `(authority, program)`, optionally after a leading extension so the TLV
/// walk is exercised past a first entry.
fn t22_mint_with_hook(supply: u64, decimals: u8, lead: Option<(u16, Vec<u8>)>, authority: [u8; 32], program: [u8; 32]) -> Vec<u8> {
    let mut d = mint_data(supply, decimals);
    d.resize(165, 0);
    d.push(1);
    if let Some((ty, v)) = lead {
        d.extend_from_slice(&ty.to_le_bytes());
        d.extend_from_slice(&(v.len() as u16).to_le_bytes());
        d.extend_from_slice(&v);
    }
    d.extend_from_slice(&14u16.to_le_bytes());
    d.extend_from_slice(&64u16.to_le_bytes());
    d.extend_from_slice(&authority);
    d.extend_from_slice(&program);
    d
}

fn seeds(parts: &[&[u8]]) -> [u8; 32] {
    let mut c = [0u8; 32];
    let mut i = 0;
    for p in parts {
        c[i..i + p.len()].copy_from_slice(p);
        i += p.len();
    }
    c
}

fn holders_entries(floor_pool: &Pubkey) -> Vec<RawMeta> {
    vec![
        RawMeta { discriminator: 1, address_config: seeds(&[&[1, 11], b"holder_book", &[3, 1]]), is_signer: false, is_writable: true },
        RawMeta { discriminator: 0, address_config: floor_pool.to_bytes(), is_signer: false, is_writable: false },
        RawMeta { discriminator: 1, address_config: seeds(&[&[1, 6], b"holder", &[3, 1], &[3, 0]]), is_signer: false, is_writable: true },
        RawMeta { discriminator: 1, address_config: seeds(&[&[1, 6], b"holder", &[3, 1], &[3, 2]]), is_signer: false, is_writable: true },
    ]
}

/// The validation account's data as `ExtraAccountMetaList::init::<ExecuteInstruction>` writes it.
fn list_bytes(entries: &[RawMeta]) -> Vec<u8> {
    let mut v = EXECUTE_DISCRIMINATOR.to_vec();
    v.extend_from_slice(&((4 + entries.len() * 35) as u32).to_le_bytes());
    v.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    for e in entries {
        v.push(e.discriminator);
        v.extend_from_slice(&e.address_config);
        v.push(e.is_signer as u8);
        v.push(e.is_writable as u8);
    }
    v
}

/// `floor_pool()` with its base (side 1) armed: the pool records `admitted` (zero = nothing), the base
/// mint's slot is `(authority, program)`, and with `with_list` the validation account holds the list.
fn hooked(admitted: [u8; 32], authority: [u8; 32], program: [u8; 32], with_list: bool) -> Synth {
    let mut s = floor_pool();
    let mut pool = s.pool();
    pool.base_transfer_hook = admitted;
    pool.base_hook_side = 1;
    s.set_pool(pool);
    s.accounts.insert(s.mint1, account(token_2022(), t22_mint_with_hook(fc::SUPPLY, 6, None, authority, program)));
    if with_list {
        let fp = s.floor_pool.unwrap();
        let v = extra_account_metas_pda(&s.mint1, &pk(program));
        s.accounts.insert(v, account(pk(program), list_bytes(&holders_entries(&fp))));
    }
    s
}

fn params<'a>(s: &Synth, zfo: bool, amount: u64, user: Pubkey) -> SwapParams<'static, 'a> {
    let mut p = swap_params(s, zfo, amount, user, &JUP, false);
    p.source_token_account = SOURCE;
    p.destination_token_account = DESTINATION;
    p
}

fn holders_pda(seeds: &[&[u8]]) -> Pubkey {
    Pubkey::find_program_address(seeds, &pk(HOLDERS)).0
}

/// What a hooked base leg appends on a FLOOR pool (`floor_pool` is already the hook slice): the book,
/// the sender's record, the receiver's record, the validation list, the program.
fn expected_tail(s: &Synth, sender_account: &Pubkey, receiver_account: &Pubkey) -> Vec<AccountMeta> {
    let mint = s.mint1;
    vec![
        AccountMeta::new(holders_pda(&[b"holder_book", mint.as_ref()]), false),
        AccountMeta::new(holders_pda(&[b"holder", mint.as_ref(), sender_account.as_ref()]), false),
        AccountMeta::new(holders_pda(&[b"holder", mint.as_ref(), receiver_account.as_ref()]), false),
        AccountMeta::new_readonly(extra_account_metas_pda(&mint, &pk(HOLDERS)), false),
        AccountMeta::new_readonly(pk(HOLDERS), false),
    ]
}

#[test]
fn an_admitted_armed_base_appends_its_hook_accounts_for_the_leg_that_moves_it() {
    let s = hooked(HOLDERS, [0u8; 32], HOLDERS, true);
    let validation = extra_account_metas_pda(&s.mint1, &pk(HOLDERS));

    let fresh = CarpenterAmm::from_keyed_account(&s.keyed_account(), &context()).unwrap();
    let keys = fresh.get_accounts_to_update();
    assert_eq!(&keys[..5], &[s.key, s.bitmap, s.floor_pool.unwrap(), s.mint0, s.mint1]);
    assert_eq!(keys.last(), Some(&validation), "the list is asked for last");

    let amm = built(&s);
    match amm.base_hook() {
        BaseHook::Admitted { program, validation: v, side, list } => assert_eq!((*program, *v, *side, list.len()), (pk(HOLDERS), validation, 1, 4)),
        other => panic!("expected Admitted, got {other:?}"),
    }
    assert!(amm.is_active());
    assert!(amm.program_dependencies().iter().any(|(id, n)| *id == pk(HOLDERS) && n == "base_transfer_hook"));
    // floor_holders is named by program, so the parity kit loads `floor_holders.so`.
    assert_eq!(carpenter_amm::transfer_hook_name(&floor_types::ids::FLOOR_HOLDERS_ID), "floor_holders");
    assert_eq!(carpenter_amm::transfer_hook_name(&HOLDERS), "base_transfer_hook");
    let user = Pubkey::new_from_array([0xEE; 32]);

    // A buy moves the base out: vault1 -> the trader's destination, signed by the pool PDA.
    let buy = amm.get_swap_and_account_metas(&params(&s, true, 1_000_000_000, user)).unwrap().account_metas;
    assert_eq!(&buy[buy.len() - 5..], &expected_tail(&s, &s.vault1, &DESTINATION)[..]);
    // A sell moves the base in: the trader's source -> vault1.
    let sell = amm.get_swap_and_account_metas(&params(&s, false, 1_000_000_000_000, user)).unwrap().account_metas;
    assert_eq!(&sell[sell.len() - 5..], &expected_tail(&s, &SOURCE, &s.vault1)[..]);

    // With the upstream variant, the transfer-hook accounts are one TransferHookB slice (the base is
    // mint1) whose length is what was appended after the by-key accounts.
    #[cfg(feature = "upstream-variant")]
    for zfo in [true, false] {
        let amount = if zfo { 1_000_000_000 } else { 1_000_000_000_000 };
        let out = amm.get_swap_and_account_metas(&params(&s, zfo, amount, user)).unwrap();
        let jupiter_amm_interface::Swap::Carpenter { zero_for_one, hook_accounts, tick_arrays, remaining_accounts_info } = out.swap else { panic!() };
        let info = remaining_accounts_info.expect("a transfer-hook slice");
        assert_eq!(info.slices.len(), 1);
        assert_eq!(info.slices[0].accounts_type, jupiter_amm_interface::AccountsType::TransferHookB);
        assert_eq!(info.slices[0].length, 5);
        assert_eq!(zero_for_one, zfo);
        assert_eq!(carpenter_amm::FIXED_METAS + usize::from(hook_accounts) + usize::from(tick_arrays) + 5, out.account_metas.len());
    }

    for metas in [&buy, &sell] {
        let fp = s.floor_pool.unwrap();
        let copies: Vec<&AccountMeta> = metas.iter().filter(|m| m.pubkey == fp).collect();
        assert_eq!(copies.len(), 1, "floor_pool appears once");
        assert!(copies[0].is_writable);
        assert!(amm.get_accounts_len() >= metas.len());
        assert!(metas.len() <= 64);
    }
}

#[test]
fn an_armed_hook_the_program_refuses_makes_the_pool_inactive_and_quotes_refuse() {
    for (s, word) in [
        (hooked(HOLDERS, [0u8; 32], [0x99; 32], true), "TransferHookNotAdmitted"),
        (hooked(HOLDERS, [0x10; 32], HOLDERS, true), "TransferHookNotAdmitted"),
        (hooked([0u8; 32], [0u8; 32], HOLDERS, true), "QuoteHookArmed"),
    ] {
        let amm = built(&s);
        assert!(matches!(amm.base_hook(), BaseHook::Refused(w) if w.contains(word)), "{word}: {:?}", amm.base_hook());
        assert!(!amm.is_active());
        let err = amm.quote(&s.quote_params(true, 1_000_000_000, SwapMode::ExactIn)).unwrap_err();
        assert!(err.to_string().contains(word), "{word}: {err}");
    }
}

#[test]
fn an_admitted_hook_whose_list_is_absent_or_unresolvable_is_inactive() {
    let amm = built(&hooked(HOLDERS, [0u8; 32], HOLDERS, false));
    assert!(matches!(amm.base_hook(), BaseHook::Refused(w) if w.contains("extra-account-metas")), "{:?}", amm.base_hook());
    assert!(!amm.is_active());

    // An `AccountData` seed needs an account's bytes at routing time and is refused.
    let mut s = hooked(HOLDERS, [0u8; 32], HOLDERS, false);
    let mut entries = holders_entries(&s.floor_pool.unwrap());
    entries[0].address_config = seeds(&[&[4, 0, 0, 8]]);
    let v = extra_account_metas_pda(&s.mint1, &pk(HOLDERS));
    s.accounts.insert(v, account(pk(HOLDERS), list_bytes(&entries)));
    let amm = built(&s);
    assert!(matches!(amm.base_hook(), BaseHook::Refused(_)));
    assert!(!amm.is_active());
}

#[test]
fn an_empty_hook_slot_appends_nothing() {
    let s = hooked(HOLDERS, [0x10; 32], [0u8; 32], false);
    let amm = built(&s);
    assert_eq!(amm.base_hook(), &BaseHook::None);
    assert!(amm.is_active());
    let user = Pubkey::new_from_array([0xEE; 32]);
    let a = amm.get_swap_and_account_metas(&params(&s, true, 1_000_000_000, user)).unwrap().account_metas;
    let p = floor_pool();
    let b = built(&p).get_swap_and_account_metas(&params(&p, true, 1_000_000_000, user)).unwrap().account_metas;
    assert_eq!(a.len(), b.len());
}

#[test]
fn the_mint_tlv_and_the_list_parse_and_resolve() {
    let d = t22_mint_with_hook(1, 6, Some((26, vec![0u8; 33])), [0x10; 32], HOLDERS);
    assert_eq!(transfer_hook_of(&d), Some(([0x10; 32], HOLDERS)));
    assert_eq!(transfer_hook_of(&mint_data(1, 6)), None, "a classic mint has no TLV");
    let fp = Pubkey::new_from_array([0x0F; 32]);
    let entries = holders_entries(&fp);
    let bytes = list_bytes(&entries);
    assert_eq!(bytes.len(), 156);
    assert_eq!(execute_metas(&bytes), Some(entries.clone()));
    let mint = Pubkey::new_from_array([0x5B; 32]);
    let v = extra_account_metas_pda(&mint, &pk(HOLDERS));
    let metas = hook_account_metas(&entries, &pk(HOLDERS), &v, &SOURCE, &mint, &DESTINATION, &JUP, 7).unwrap();
    assert_eq!(metas.len(), 6, "four entries, the list, the program");
    assert_eq!(metas[1], AccountMeta::new_readonly(fp, false));
    assert_eq!(metas[2].pubkey, holders_pda(&[b"holder", mint.as_ref(), SOURCE.as_ref()]));
    let far = [RawMeta { discriminator: 1, address_config: seeds(&[&[3, 9]]), is_signer: false, is_writable: false }];
    assert_eq!(hook_account_metas(&far, &pk(HOLDERS), &v, &SOURCE, &mint, &DESTINATION, &JUP, 7).unwrap_err(), ResolveErr::OutOfRange);
    let pubkey_data = [RawMeta { discriminator: 2, address_config: [0u8; 32], is_signer: false, is_writable: false }];
    assert_eq!(hook_account_metas(&pubkey_data, &pk(HOLDERS), &v, &SOURCE, &mint, &DESTINATION, &JUP, 7).unwrap_err(), ResolveErr::Unsupported);
}
