//! Unit tests, one group per `Amm` method, plus the window, the encoder and the crate's dependency
//! and source rules. Every amount the adapter reports is compared with the quoters called directly on
//! the same bytes (`Synth::reference`).

mod common;

use carpenter_amm::accounts::{event_authority_pda, hook_event_authority_pda, tick_array_pda, trader_bucket_pda, window, window_with};
use carpenter_amm::amm::tick_array_body;
use carpenter_amm::{swap_instruction_data, CarpenterAmm, KnownHook, FIXED_METAS, LABEL, MAX_ARRAYS, TRADER_ARRAYS_PER_SIDE};
use carpenter_quote::state::{decode_bitmap, encode_bitmap};
use carpenter_quote::{no_limit, QuoteError};
use carpenter_types::constants as cc;
use carpenter_types::disc;
use carpenter_types::layout::{Layout, Pool};
use common::*;
use floor_quote::FloorError;
use jupiter_amm_interface::{Amm, AmmContext, ClockRef, SwapMode};
use rust_decimal::Decimal;
use solana_clock::Clock;
use solana_pubkey::Pubkey;

fn err_name(e: QuoteError) -> String {
    e.to_string()
}

// ---- from_keyed_account ---------------------------------------------------------------------

#[test]
fn from_keyed_account_classifies_the_hook() {
    let p = CarpenterAmm::from_keyed_account(&plain_pool().keyed_account(), &context()).unwrap();
    assert!(matches!(p.hook(), KnownHook::Inert));
    let f = floor_pool();
    let a = CarpenterAmm::from_keyed_account(&f.keyed_account(), &context()).unwrap();
    match a.hook() {
        KnownHook::Floor { floor_pool, state, cushion_top } => {
            assert_eq!(*floor_pool, f.floor_pool.unwrap(), "the [\"floor_pool\", pool] PDA under floor_hook");
            assert!(state.is_none() && cushion_top.is_none(), "nothing is read before update");
        }
        other => panic!("{other:?}"),
    }
    assert!(!a.is_active(), "never updated");
}

#[test]
fn from_keyed_account_refuses_what_is_not_a_carpenter_pool() {
    // Not owned by carpenter_amm.
    let mut foreign = plain_pool();
    foreign.accounts.get_mut(&foreign.key).unwrap().owner = Pubkey::new_from_array([0xDE; 32]);
    let err = CarpenterAmm::from_keyed_account(&foreign.keyed_account(), &context()).unwrap_err();
    assert!(err.to_string().contains("not carpenter_amm"), "{err}");

    // Another carpenter_amm account (the bitmap) under the pool's key.
    let s = plain_pool();
    let mut ka = s.keyed_account();
    ka.account = s.accounts[&s.bitmap].clone();
    let err = CarpenterAmm::from_keyed_account(&ka, &context()).unwrap_err();
    assert!(err.to_string().contains(Pool::NAME), "{err}");

    // A floor_hook pool whose hook slice is not the four accounts this adapter builds.
    let mut odd = floor_pool();
    let mut pool = odd.pool();
    pool.hook_callback_accounts = 1;
    odd.set_pool(pool);
    let err = CarpenterAmm::from_keyed_account(&odd.keyed_account(), &context()).unwrap_err();
    assert!(err.to_string().contains("hook_callback_accounts is 1, not 4"), "{err}");
}

#[test]
fn an_unknown_swap_hook_constructs_but_is_inactive_and_refused() {
    let mut s = plain_pool();
    let stranger = Pubkey::new_from_array([0xDE; 32]);
    let mut pool = s.pool();
    pool.hook = stranger.to_bytes();
    pool.hook_flags = cc::BEFORE_SWAP_FLAG | cc::AFTER_SWAP_FLAG;
    s.set_pool(pool);
    let amm = built(&s);
    assert!(matches!(amm.hook(), KnownHook::Unknown { hook_program } if *hook_program == stranger));
    assert!(!amm.is_active());
    let err = amm.quote(&s.quote_params(true, 1_000, SwapMode::ExactIn)).unwrap_err();
    assert_eq!(err.to_string(), err_name(QuoteError::QuoteHookNotQuotable));
    let sp = swap_params(&s, true, 1_000, Pubkey::new_unique(), &JUP, true);
    assert!(amm.get_swap_and_account_metas(&sp).is_err(), "crawl mode does not hide an unquotable hook");

    // The same program with no swap flag cannot change a swap: plain.
    pool.hook_flags = cc::BEFORE_INITIALIZE_FLAG;
    s.set_pool(pool);
    let amm = built(&s);
    assert!(matches!(amm.hook(), KnownHook::Inert));
    assert!(amm.is_active());
    assert_eq!(amm.quote(&s.quote_params(true, 1_000_000, SwapMode::ExactIn)).unwrap().out_amount, plain_pool().reference(true, 1_000_000).unwrap().amount_out);
}

// ---- label, program_id, key, get_reserve_mints, provided flags --------------------------------

#[test]
fn identity_and_provided_methods() {
    let s = plain_pool();
    let amm = built(&s);
    assert_eq!(amm.label(), LABEL);
    assert_eq!(LABEL, "Carpenter");
    assert_eq!(amm.program_id(), amm_id());
    assert_eq!(amm.key(), s.key);
    assert_eq!(amm.get_reserve_mints(), vec![s.mint0, s.mint1]);
    assert!(amm.has_dynamic_accounts());
    assert!(!amm.supports_exact_out());
    assert!(!amm.unidirectional());
    assert!(!amm.requires_update_for_reserve_mints());
    assert!(amm.underlying_liquidities().is_none());
}

#[test]
fn program_dependencies_name_the_hook_and_token_2022() {
    let plain = built(&plain_pool());
    assert!(plain.program_dependencies().is_empty(), "a plain SPL pool invokes only the token program");
    let f = built(&floor_pool());
    let deps = f.program_dependencies();
    assert!(deps.contains(&(floor_hook(), "floor_hook".to_string())), "{deps:?}");
    assert!(deps.contains(&(token_2022(), "spl_token_2022".to_string())), "{deps:?}");
}

#[test]
fn get_accounts_len_is_the_upper_bound_never_the_default() {
    let p = built(&plain_pool());
    assert_eq!(p.get_accounts_len(), FIXED_METAS + p.window().starts.len());
    let f = built(&floor_pool());
    assert_eq!(f.get_accounts_len(), FIXED_METAS + 4 + f.window().starts.len() + carpenter_amm::ACTION_POSITIONS_FLOOR);
    assert_ne!(p.get_accounts_len(), 32, "the trait's default leaked through");
}

// ---- get_accounts_to_update / update -----------------------------------------------------------

#[test]
fn accounts_to_update_before_and_after_the_first_update() {
    let f = floor_pool();
    let fresh = CarpenterAmm::from_keyed_account(&f.keyed_account(), &context()).unwrap();
    assert_eq!(fresh.get_accounts_to_update(), vec![f.key, f.bitmap, f.floor_pool.unwrap(), f.mint0, f.mint1]);
    let amm = built(&f);
    let keys = amm.get_accounts_to_update();
    assert_eq!(&keys[..5], &[f.key, f.bitmap, f.floor_pool.unwrap(), f.mint0, f.mint1]);
    let arrays: Vec<Pubkey> = amm.window().starts.iter().map(|s| tick_array_pda(&amm_id(), &f.key, *s)).collect();
    assert_eq!(&keys[5..], &arrays[..]);
    assert_eq!(amm.window().starts, f.arrays.keys().copied().collect::<Vec<_>>(), "fewer than MAX_ARRAYS marked: the window is all of them");

    let p = plain_pool();
    let fresh = CarpenterAmm::from_keyed_account(&p.keyed_account(), &context()).unwrap();
    assert_eq!(fresh.get_accounts_to_update(), vec![p.key, p.bitmap, p.mint0, p.mint1], "no hook account on a plain pool");
}

#[test]
fn update_reads_supplies_and_the_floor_state() {
    let f = floor_pool();
    let amm = built(&f);
    assert!(amm.is_updated());
    assert_eq!(amm.hook().floor_state().map(|s| s.floor_tick), f.floor_state.map(|s| s.floor_tick));
    // The supply feeds the hook callbacks: a different mint1 supply moves a FLOOR sell's quote.
    let mut g = floor_pool();
    let mint1 = g.mint1;
    g.accounts.get_mut(&mint1).unwrap().data = mint_data(floor_types::constants::SUPPLY / 2, 6);
    let a = amm.quote(&f.quote_params(false, 1_000_000_000_000, SwapMode::ExactIn)).unwrap();
    let b = built(&g).quote(&g.quote_params(false, 1_000_000_000_000, SwapMode::ExactIn));
    let r = {
        let set = g.synth.tick_array_set().unwrap();
        let params = floor_quote::SwapQuoteParams {
            zero_for_one: false,
            amount_specified: -1_000_000_000_000,
            sqrt_price_limit_x64: no_limit(false),
            supply0: RICH,
            supply1: floor_types::constants::SUPPLY / 2,
            cushion_top: None,
        };
        floor_quote::quote_swap(&carpenter_quote::PoolAccount { key: g.key.to_bytes(), pool: g.pool() }, &set, g.floor_state.as_ref(), &params)
    };
    match (b, r) {
        (Ok(b), Ok(r)) => assert_eq!((b.in_amount, b.out_amount), (r.amount_in, r.amount_out), "the adapter read mint1's supply"),
        (Err(b), Err(r)) => assert_eq!(b.to_string(), r.to_string()),
        (b, r) => panic!("{b:?} vs {r:?}"),
    }
    assert!(a.out_amount > 0);
}

/// A missing required account fails `update` by name; a missing `floor_pool` does not fail it, but the
/// pool is inactive and quotes refuse with `QuoteHookStateMissing`.
#[test]
fn update_requires_the_pool_bitmap_and_mints_and_reports_a_missing_hook_state() {
    for which in ["bitmap", "mint0", "mint1"] {
        let mut s = plain_pool();
        let k = match which {
            "bitmap" => s.bitmap,
            "mint0" => s.mint0,
            _ => s.mint1,
        };
        s.accounts.remove(&k);
        let mut amm = CarpenterAmm::from_keyed_account(&s.keyed_account(), &context()).unwrap();
        let err = amm.update(s.provider()).unwrap_err();
        assert!(err.to_string().contains(&k.to_string()), "{which}: {err}");
        assert!(!amm.is_active());
    }

    let mut f = floor_pool();
    f.accounts.remove(&f.floor_pool.unwrap());
    let amm = built(&f);
    assert!(amm.is_updated());
    assert!(!amm.is_active());
    let err = amm.quote(&f.quote_params(true, 1_000, SwapMode::ExactIn)).unwrap_err();
    assert_eq!(err.to_string(), err_name(QuoteError::QuoteHookStateMissing));
}

/// `update` is all or nothing: a pass that fails leaves the previous pass's pool and arrays, clears
/// `is_updated`, and quotes refuse until a pass succeeds.
#[test]
fn a_failed_update_deactivates_and_never_mixes_state() {
    let f = floor_pool();
    let mut amm = built(&f);
    let before = *amm.pool();
    let good = amm.quote(&f.quote_params(true, 1_000_000_000, SwapMode::ExactIn)).unwrap();

    let mut late = floor_pool();
    let mut pool = late.pool();
    pool.tick_current += 600;
    pool.sqrt_price_x64 = carpenter_quote::clmm::tick_math::get_sqrt_price_at_tick(pool.tick_current).unwrap().into();
    late.set_pool(pool);
    let some_array = *late.arrays.values().next().unwrap();
    late.accounts.get_mut(&some_array).unwrap().data.truncate(8);
    assert!(amm.update(late.provider()).is_err());
    assert!(!amm.is_active());
    assert_eq!(amm.pool().tick_current, before.tick_current, "the pool decoded first must not outlive the array that failed");
    let err = amm.quote(&f.quote_params(true, 1_000_000_000, SwapMode::ExactIn)).unwrap_err();
    assert!(err.to_string().contains("no settled update"), "{err}");
    assert!(amm.get_swap_and_account_metas(&swap_params(&f, true, 1_000_000_000, Pubkey::default(), &JUP, true)).is_err());

    amm.update(f.provider()).unwrap();
    assert!(amm.is_active());
    assert_eq!(amm.quote(&f.quote_params(true, 1_000_000_000, SwapMode::ExactIn)).unwrap().out_amount, good.out_amount);
}

/// A tick array is decoded from its 3,888-byte body; the rent owner after it prices nothing, and any
/// length other than the account (3,920) or the bare body (3,888) is refused.
#[test]
fn tick_arrays_decode_from_their_body() {
    let base = floor_pool();
    let mut other_owner = floor_pool();
    for k in other_owner.arrays.values() {
        let d = &mut other_owner.accounts.get_mut(k).unwrap().data;
        d[TICK_ARRAY_BODY_LEN..].copy_from_slice(&[0x7A; 32]);
    }
    let mut bare = floor_pool();
    for k in bare.arrays.values() {
        bare.accounts.get_mut(k).unwrap().data.truncate(TICK_ARRAY_BODY_LEN);
    }
    let (a, b, c) = (built(&base), built(&other_owner), built(&bare));
    for zfo in [true, false] {
        let qa = a.quote(&base.quote_params(zfo, 5_000_000_000, SwapMode::ExactIn)).unwrap();
        let qb = b.quote(&base.quote_params(zfo, 5_000_000_000, SwapMode::ExactIn)).unwrap();
        let qc = c.quote(&base.quote_params(zfo, 5_000_000_000, SwapMode::ExactIn)).unwrap();
        assert_eq!((qa.in_amount, qa.out_amount, qa.fee_amount), (qb.in_amount, qb.out_amount, qb.fee_amount));
        assert_eq!((qa.in_amount, qa.out_amount, qa.fee_amount), (qc.in_amount, qc.out_amount, qc.fee_amount));
    }
    for len in [TICK_ARRAY_BODY_LEN - 1, TICK_ARRAY_BODY_LEN + 1, TICK_ARRAY_ACCOUNT_LEN + 1] {
        let mut s = floor_pool();
        let k = *s.arrays.values().next().unwrap();
        s.accounts.get_mut(&k).unwrap().data.resize(len, 0);
        let mut amm = CarpenterAmm::from_keyed_account(&s.keyed_account(), &context()).unwrap();
        // One pass computes the window from the bitmap and reads its arrays.
        let err = amm.update(s.provider()).unwrap_err();
        assert!(err.to_string().contains("QuoteBadAccount(TickArray)"), "{len} B: {err}");
    }
    for len in [0usize, 8, TICK_ARRAY_BODY_LEN, TICK_ARRAY_ACCOUNT_LEN - 1, TICK_ARRAY_ACCOUNT_LEN + 1] {
        assert_eq!(tick_array_body(&vec![1u8; len]).len(), len);
    }
    assert_eq!(tick_array_body(&vec![1u8; TICK_ARRAY_ACCOUNT_LEN]).len(), TICK_ARRAY_BODY_LEN);
}

/// An array stored at the PDA of a different start is refused rather than quoted.
#[test]
fn an_array_at_the_wrong_address_is_refused() {
    let mut s = plain_pool();
    let starts: Vec<i32> = s.arrays.keys().copied().collect();
    let (a, b) = (s.arrays[&starts[0]], s.arrays[&starts[1]]);
    let da = s.accounts[&a].data.clone();
    let db = s.accounts[&b].data.clone();
    s.accounts.get_mut(&a).unwrap().data = db;
    s.accounts.get_mut(&b).unwrap().data = da.clone();
    let mut amm = CarpenterAmm::from_keyed_account(&s.keyed_account(), &context()).unwrap();
    let err = amm.update(s.provider()).unwrap_err();
    assert!(err.to_string().contains("is not this pool's array"), "{err}");
    // The same array at two addresses.
    s.accounts.get_mut(&a).unwrap().data = da;
    assert!(amm.update(s.provider()).is_err());
    assert!(!amm.is_active());
}

// ---- quote --------------------------------------------------------------------------------------

/// Every amount the adapter reports is the quoters' on the same bytes, buy and sell, plain and FLOOR,
/// including a sell that triggers the hook's bid re-place.
#[test]
fn quote_delegates_every_amount() {
    let mut replaced = 0;
    for (s, amounts) in [
        (plain_pool(), vec![1_000_000u64, 150_000_000_000, 200_000_000_000]),
        (floor_pool(), vec![100_000_000, 1_000_000_000, 5_000_000_000, 10_000_000_000_000, 100_000_000_000_000]),
    ] {
        let amm = built(&s);
        for zfo in [true, false] {
            for amount in &amounts {
                let want = s.reference(zfo, *amount);
                let got = amm.quote(&s.quote_params(zfo, *amount, SwapMode::ExactIn));
                match (want, got) {
                    (Ok(w), Ok(g)) => {
                        assert_eq!((g.in_amount, g.out_amount), (w.amount_in, w.amount_out), "zfo {zfo} amount {amount}");
                        replaced += usize::from(w.replace.is_some());
                    }
                    (Err(w), Err(g)) => assert_eq!(g.to_string(), w.to_string()),
                    (w, g) => panic!("zfo {zfo} amount {amount}: {w:?} vs {g:?}"),
                }
            }
        }
    }
    assert!(replaced > 0, "at least one row must exercise the hook's actions");
}

/// `fee_mint` is the input mint; a FLOOR buy's `fee_amount` is the LP fee plus the hook's takes (all
/// in the quote currency, the input); a FLOOR sell's is the LP fee alone (the hook's takes came out of
/// `out_amount`); `fee_pct = fee_amount / in_amount`.
#[test]
fn quote_fee_triple() {
    let f = floor_pool();
    let amm = built(&f);
    let buy = amm.quote(&f.quote_params(true, 1_000_000_000, SwapMode::ExactIn)).unwrap();
    let r = f.reference(true, 1_000_000_000).unwrap();
    assert_eq!(buy.fee_mint, f.mint0);
    assert!(r.fees.skim != 0 && r.fees.creator != 0, "the row must pay hook fees to mean anything");
    assert_eq!(buy.fee_amount, r.fees.lp + r.fees.hook_total());
    assert_eq!(buy.fee_pct, Decimal::from(buy.fee_amount) / Decimal::from(buy.in_amount));

    let sell = amm.quote(&f.quote_params(false, 1_000_000_000_000, SwapMode::ExactIn)).unwrap();
    let r = f.reference(false, 1_000_000_000_000).unwrap();
    assert_eq!(sell.fee_mint, f.mint1);
    assert_ne!(r.fees.skim, 0);
    assert_eq!(sell.fee_amount, r.fees.lp);

    let p = plain_pool();
    let q = built(&p).quote(&p.quote_params(false, 1_000_000, SwapMode::ExactIn)).unwrap();
    let r = p.reference(false, 1_000_000).unwrap();
    assert_eq!((q.fee_amount, q.fee_mint), (r.fees.lp, p.mint1));
}

/// The Carpenter Platform fee is counted once, and only when charged in the input currency: a FLOOR
/// buy and a plain pool either way, never a FLOOR sell (it came off the output).
#[test]
fn quote_fee_triple_counts_the_carpenter_platform_fee_on_its_side() {
    let with_fee = |mut s: Synth, packed: u32, currency: u8| {
        let mut pool = s.pool();
        pool.protocol_fee = packed;
        pool.protocol_fee_currency = currency;
        s.set_pool(pool);
        s
    };
    let f = with_fee(floor_pool(), cc::PROTOCOL_FEE_DEFAULT_PACKED, cc::PROTOCOL_FEE_ON_CURRENCY0);
    let amm = built(&f);
    let buy = amm.quote(&f.quote_params(true, 1_000_000_000, SwapMode::ExactIn)).unwrap();
    let r = f.reference(true, 1_000_000_000).unwrap();
    assert_ne!(r.fees.protocol, 0);
    assert_eq!(buy.fee_amount, r.fees.lp + r.fees.hook_total() + r.fees.protocol);
    let sell = amm.quote(&f.quote_params(false, 1_000_000_000_000, SwapMode::ExactIn)).unwrap();
    let r = f.reference(false, 1_000_000_000_000).unwrap();
    assert_ne!(r.fees.protocol, 0);
    assert_eq!(sell.fee_amount, r.fees.lp);
    assert_eq!(sell.out_amount, r.amount_out);

    let p = with_fee(plain_pool(), cc::PROTOCOL_FEE_DEFAULT_PACKED, cc::PROTOCOL_FEE_ON_INPUT);
    let pamm = built(&p);
    for zfo in [true, false] {
        let q = pamm.quote(&p.quote_params(zfo, 1_000_000, SwapMode::ExactIn)).unwrap();
        let r = p.reference(zfo, 1_000_000).unwrap();
        assert_ne!(r.fees.protocol, 0);
        assert_eq!(q.fee_amount, r.fees.lp + r.fees.protocol, "zfo {zfo}");
    }
}

#[test]
fn quote_refuses_exact_out_and_a_foreign_pair() {
    for s in [floor_pool(), plain_pool()] {
        let amm = built(&s);
        let err = amm.quote(&s.quote_params(true, 1_000, SwapMode::ExactOut)).unwrap_err();
        assert_eq!(err.to_string(), err_name(FloorError::ExactOutNotSupported.into()));
        let mut sp = swap_params(&s, true, 1_000, Pubkey::default(), &JUP, false);
        sp.swap_mode = SwapMode::ExactOut;
        let err = amm.get_swap_and_account_metas(&sp).err().expect("exact-out metas refused");
        assert!(err.to_string().contains("ExactOutNotSupported"), "{err}");

        let mut qp = s.quote_params(true, 1_000, SwapMode::ExactIn);
        qp.output_mint = Pubkey::new_unique();
        assert!(amm.quote(&qp).unwrap_err().to_string().contains("not this pool's pair"));
    }
}

/// The context's clock is never read: two contexts a million slots apart quote byte-identically, and
/// the source never names the field.
#[test]
fn quote_reads_no_clock() {
    let s = floor_pool();
    let quotes: Vec<_> = [0u64, 1_000_000]
        .into_iter()
        .map(|slot| {
            let clock = Clock { slot, epoch_start_timestamp: 0, epoch: slot / 432_000, leader_schedule_epoch: 0, unix_timestamp: 1_700_000_000 + slot as i64 };
            let ctx = AmmContext { clock_ref: ClockRef::default() };
            ctx.clock_ref.update(clock);
            let mut amm = CarpenterAmm::from_keyed_account(&s.keyed_account(), &ctx).unwrap();
            amm.update(s.provider()).unwrap();
            amm.update(s.provider()).unwrap();
            let q = amm.quote(&s.quote_params(true, 1_000_000_000, SwapMode::ExactIn)).unwrap();
            (q.in_amount, q.out_amount, q.fee_amount, q.fee_mint, q.fee_pct)
        })
        .collect();
    assert_eq!(quotes[0], quotes[1]);
}

// ---- get_swap_and_account_metas ----------------------------------------------------------------

/// The metas are the `swap` instruction's in order, the tick arrays exactly the quote's
/// `all_arrays()` at this amount, and the swap is `Placeholder([zero_for_one])`.
#[test]
fn metas_are_the_instruction_order_and_exactly_the_quoted_arrays() {
    let user = Pubkey::new_from_array([0x55; 32]);
    let mut positions_seen = 0usize;
    for (s, amounts, hooked) in [
        (plain_pool(), vec![1_000_000u64, 200_000_000_000], false),
        (floor_pool(), vec![1_000_000_000u64, 1_000_000_000_000, 100_000_000_000_000], true),
    ] {
        let amm = built(&s);
        for zfo in [true, false] {
            for amount in &amounts {
                let sp = swap_params(&s, zfo, *amount, user, &JUP, false);
                let out = amm.get_swap_and_account_metas(&sp).unwrap();
                let m = &out.account_metas;
                #[cfg(not(feature = "upstream-variant"))]
                assert_eq!(out.swap, jupiter_amm_interface::Swap::Placeholder { data: Some(vec![zfo as u8]) });
                assert_eq!((m[0].pubkey, m[0].is_signer, m[0].is_writable), (user, true, false), "trader");
                assert_eq!((m[1].pubkey, m[1].is_writable), (s.key, true));
                assert_eq!((m[2].pubkey, m[2].is_writable), (s.bitmap, true));
                assert_eq!((m[3].pubkey, m[3].is_writable), (sp.source_token_account, true));
                assert_eq!((m[4].pubkey, m[4].is_writable), (sp.destination_token_account, true));
                assert_eq!((m[5].pubkey, m[5].is_writable), (s.vault0, true));
                assert_eq!((m[6].pubkey, m[6].is_writable), (s.vault1, true));
                assert_eq!((m[7].pubkey, m[7].is_writable), (s.mint0, false));
                assert_eq!((m[8].pubkey, m[8].is_writable), (s.mint1, false));
                assert_eq!(m[9].pubkey, spl_token());
                assert_eq!(m[10].pubkey, if hooked { token_2022() } else { spl_token() });
                assert_eq!(m[11].pubkey, if hooked { floor_hook() } else { amm_id() }, "hook_program, or the program id for none");
                assert_eq!(m[12].pubkey, event_authority_pda(&amm_id()));
                assert_eq!(m[13].pubkey, amm_id());
                assert!(m[..FIXED_METAS].iter().skip(1).all(|x| !x.is_signer));
                let mut i = FIXED_METAS;
                if hooked {
                    assert_eq!((m[i].pubkey, m[i].is_writable), (s.floor_pool.unwrap(), true), "floor_pool, writable");
                    let bucket = Pubkey::find_program_address(&[b"trader_ref", user.as_ref(), s.mint0.as_ref()], &floor_hook()).0;
                    assert_eq!(bucket, trader_bucket_pda(&floor_hook(), &user, &s.mint0));
                    assert_eq!((m[i + 1].pubkey, m[i + 1].is_writable, m[i + 1].is_signer), (bucket, true, false));
                    // The callbacks' `#[event_cpi]` pair, read-only.
                    let hook_ea = Pubkey::find_program_address(&[b"__event_authority"], &floor_hook()).0;
                    assert_eq!(hook_ea, hook_event_authority_pda(&floor_hook()));
                    assert_eq!((m[i + 2].pubkey, m[i + 2].is_writable, m[i + 2].is_signer), (hook_ea, false, false), "the hook's event authority");
                    assert_eq!((m[i + 3].pubkey, m[i + 3].is_writable, m[i + 3].is_signer), (floor_hook(), false, false), "floor_hook");
                    i += 4;
                }
                let reference = s.reference(zfo, *amount).unwrap();
                let mut want: Vec<Pubkey> = reference.all_arrays().iter().map(|st| tick_array_pda(&amm_id(), &s.key, *st)).collect();
                assert!(!want.is_empty());
                // Then the hook authority's position for every distinct ModifyPosition salt, derived
                // here from the seeds rather than through the adapter's helper.
                let owner = Pubkey::new_from_array(s.pool().hook_authority);
                let mut salts = Vec::new();
                for a in &reference.replace_actions {
                    if let carpenter_types::hook::HookAction::ModifyPosition { salt, .. } = a {
                        if !salts.contains(salt) {
                            salts.push(*salt);
                        }
                    }
                }
                positions_seen += salts.len();
                want.extend(salts.iter().map(|salt| {
                    Pubkey::find_program_address(&[b"position", s.key.as_ref(), owner.as_ref(), &salt.to_le_bytes()], &amm_id()).0
                }));
                assert_eq!(m[i..].iter().map(|x| x.pubkey).collect::<Vec<_>>(), want, "zfo {zfo} amount {amount}");
                // The variant's counts delimit exactly this layout: the hook slice, then the
                // by-key accounts, and no transfer-hook slice on these mints.
                #[cfg(feature = "upstream-variant")]
                assert_eq!(
                    out.swap,
                    jupiter_amm_interface::Swap::Carpenter {
                        zero_for_one: zfo,
                        hook_accounts: if hooked { 4 } else { 0 },
                        tick_arrays: want.len() as u8,
                        remaining_accounts_info: None,
                    },
                    "zfo {zfo} amount {amount}"
                );
                assert!(m[i..].iter().all(|x| x.is_writable && !x.is_signer));
                assert!(amm.get_accounts_len() >= m.len());
                assert!(m.len() <= FIXED_METAS + 4 + MAX_ARRAYS + carpenter_amm::ACTION_POSITIONS_FLOOR);
            }
        }
    }
    // The large FLOOR sell reaches the bid and re-places it: its position must have been listed.
    assert!(positions_seen > 0, "no case exercised a ModifyPosition action");
}

/// A sell that triggers the bid re-place carries the actions' arrays as well as the trader's.
#[test]
fn metas_carry_the_hook_actions_arrays() {
    let f = floor_pool();
    let amm = built(&f);
    let amount = 100_000_000_000_000u64;
    let r = f.reference(false, amount).unwrap();
    assert!(r.replace.is_some() && !r.action_arrays.is_empty(), "this row must trigger the re-place");
    let out = amm.get_swap_and_account_metas(&swap_params(&f, false, amount, Pubkey::new_unique(), &JUP, false)).unwrap();
    let keys: Vec<Pubkey> = out.account_metas.iter().map(|m| m.pubkey).collect();
    for st in &r.action_arrays {
        assert!(keys.contains(&tick_array_pda(&amm_id(), &f.key, *st)), "action array {st} missing");
    }
}

/// `missing_dynamic_accounts_as_default` replaces only a missing-array refusal; any other refusal
/// propagates either way.
#[test]
fn crawl_mode_defaults_only_a_missing_array() {
    let mut s = plain_pool();
    let far = s.arrays[&-61_200];
    s.accounts.remove(&far);
    let amm = built(&s);
    assert!(amm.is_active());
    let huge = u64::MAX / 8;
    assert_eq!(amm.quote(&s.quote_params(true, huge, SwapMode::ExactIn)).unwrap_err().to_string(), "TickArrayMissing(-61200)");
    assert!(amm.get_swap_and_account_metas(&swap_params(&s, true, huge, Pubkey::default(), &JUP, false)).is_err());
    let out = amm.get_swap_and_account_metas(&swap_params(&s, true, huge, Pubkey::default(), &JUP, true)).unwrap();
    assert_eq!(out.account_metas.len(), FIXED_METAS);
    // A swap that stays clear of the missing array is unaffected.
    let full = built(&plain_pool());
    let amount = 100_000_000_000u64;
    assert_eq!(
        amm.quote(&s.quote_params(true, amount, SwapMode::ExactIn)).unwrap().out_amount,
        full.quote(&s.quote_params(true, amount, SwapMode::ExactIn)).unwrap().out_amount
    );

    let p = plain_pool();
    let amm = built(&p);
    for crawl in [false, true] {
        let err = amm.get_swap_and_account_metas(&swap_params(&p, true, 0, Pubkey::default(), &JUP, crawl)).err().unwrap();
        assert_eq!(err.to_string(), err_name(QuoteError::AmountZero), "crawl {crawl}");
    }
    let mut wrong = floor_pool();
    let mut fp = wrong.floor_state.unwrap();
    fp.amm_pool = [0xAB; 32];
    wrong.set_floor(fp);
    let amm = built(&wrong);
    for crawl in [false, true] {
        let err = amm.get_swap_and_account_metas(&swap_params(&wrong, true, 1_000_000_000, Pubkey::default(), &JUP, crawl)).err().unwrap();
        assert_eq!(err.to_string(), err_name(FloorError::WrongPool.into()), "crawl {crawl}");
    }
}

/// The account count of every row, plus an allowance for the router's own accounts, stays inside a
/// transaction's 64 account locks; so does the largest list the knobs allow.
#[test]
fn lock_budget_stays_inside_64() {
    const ROUTER_OWN_EST: usize = 10;
    const _: () = assert!(FIXED_METAS + 4 + MAX_ARRAYS + carpenter_amm::ACTION_POSITIONS_FLOOR + ROUTER_OWN_EST <= 64);
    let user = Pubkey::new_unique();
    for (s, amounts) in [(plain_pool(), vec![1_000_000u64, 200_000_000_000]), (floor_pool(), vec![1_000_000_000, 100_000_000_000_000])] {
        let amm = built(&s);
        for zfo in [true, false] {
            for amount in &amounts {
                let n = amm.get_swap_and_account_metas(&swap_params(&s, zfo, *amount, user, &JUP, false)).unwrap().account_metas.len();
                assert!(n + ROUTER_OWN_EST <= 64, "{n} metas");
            }
        }
    }
}

// ---- is_active ----------------------------------------------------------------------------------

#[test]
fn is_active_gate() {
    assert!(built(&plain_pool()).is_active());
    assert!(built(&floor_pool()).is_active());

    let mut dry = plain_pool();
    let mut pool = dry.pool();
    pool.liquidity = 0u128.into();
    dry.set_pool(pool);
    assert!(!built(&dry).is_active(), "no in-range liquidity");

    let mut paused = plain_pool();
    let mut pool = paused.pool();
    pool.paused = 1;
    paused.set_pool(pool);
    assert!(!built(&paused).is_active(), "the program refuses swaps on a paused pool");
    assert_eq!(built(&paused).quote(&paused.quote_params(true, 1_000, SwapMode::ExactIn)).unwrap_err().to_string(), "PoolPaused");

    let mut wrong = floor_pool();
    let mut fp = wrong.floor_state.unwrap();
    fp.amm_pool = [0xAB; 32];
    wrong.set_floor(fp);
    let amm = built(&wrong);
    assert!(!amm.is_active(), "FloorPool bound to another pool");
    assert_eq!(amm.quote(&wrong.quote_params(true, 1_000, SwapMode::ExactIn)).unwrap_err().to_string(), err_name(FloorError::WrongPool.into()));

    let mut unbound = floor_pool();
    let mut fp = unbound.floor_state.unwrap();
    fp.pool_bound = 0;
    unbound.set_floor(fp);
    assert!(!built(&unbound).is_active());

    // A launch still in its bonding sale: floor_hook's before_swap refuses every swap (NotSeeded),
    // and so does the FLOOR quoter, by that name; the adapter is inactive and passes the refusal on
    // rather than answering a swap the program would fail.
    let mut bonding = floor_pool();
    let mut fp = bonding.floor_state.unwrap();
    fp.ladder_seeded = 0;
    bonding.set_floor(fp);
    let amm = built(&bonding);
    assert!(!amm.is_active(), "an unseeded FLOOR pool");
    for zfo in [true, false] {
        assert_eq!(amm.quote(&bonding.quote_params(zfo, 1_000_000, SwapMode::ExactIn)).unwrap_err().to_string(), err_name(FloorError::NotSeeded.into()));
        let user = Pubkey::new_from_array([0x55; 32]);
        assert!(amm.get_swap_and_account_metas(&swap_params(&bonding, zfo, 1_000_000, user, &JUP, true)).is_err());
    }
}

/// A guardian pause is a time window: the pool is inactive while `guardian_paused_until` is ahead of
/// the router's clock and active again once the clock passes it, the clock read live through the
/// context's handle. Quotes do not depend on the clock either way.
#[test]
fn is_active_follows_the_guardian_window_on_the_routers_clock() {
    let mut s = plain_pool();
    let mut pool = s.pool();
    pool.guardian_paused_until = 1_700_000_100;
    s.set_pool(pool);
    let ctx = AmmContext { clock_ref: ClockRef::default() };
    let at = |t: i64| ctx.clock_ref.update(Clock { slot: 1, epoch_start_timestamp: 0, epoch: 0, leader_schedule_epoch: 0, unix_timestamp: t });
    at(1_700_000_000);
    let mut amm = CarpenterAmm::from_keyed_account(&s.keyed_account(), &ctx).unwrap();
    amm.update(s.provider()).unwrap();
    amm.update(s.provider()).unwrap();
    assert!(!amm.is_active(), "inside the guardian window");
    let q0 = amm.quote(&s.quote_params(true, 1_000_000, SwapMode::ExactIn)).unwrap();
    at(1_700_000_099);
    assert!(!amm.is_active(), "one second before it ends");
    at(1_700_000_100);
    assert!(amm.is_active(), "the window lapsed on its own");
    let q1 = amm.quote(&s.quote_params(true, 1_000_000, SwapMode::ExactIn)).unwrap();
    assert_eq!((q0.in_amount, q0.out_amount, q0.fee_amount), (q1.in_amount, q1.out_amount, q1.fee_amount), "quotes never read the clock");
    // a clone shares the router's clock handle
    let clone = amm.clone();
    at(1_700_000_000);
    assert!(!clone.is_active());
}

// ---- the window ---------------------------------------------------------------------------------

#[test]
fn the_window_is_capped_and_a_walk_past_it_refuses_by_name() {
    let s = plain_pool_with_arrays(MAX_ARRAYS + 8);
    let amm = built(&s);
    let w = amm.window();
    assert!(w.starts.len() <= MAX_ARRAYS && !w.floor_span_too_wide);
    assert!(w.starts.contains(&0));
    assert!(amm.is_active());
    assert!(amm.quote(&s.quote_params(true, 1_000_000, SwapMode::ExactIn)).is_ok());
    let err = amm.quote(&s.quote_params(true, u64::MAX / 8, SwapMode::ExactIn)).unwrap_err();
    assert!(err.to_string().starts_with("TickArrayMissing("), "{err}");
}

/// When the cap binds, the trader's arrays are admitted nearest first across both sides.
#[test]
fn the_window_admits_trader_arrays_nearest_first() {
    let s = plain_pool_with_arrays(MAX_ARRAYS + 8);
    let pool = s.pool();
    let words = decode_bitmap(&s.accounts[&s.bitmap].data, &s.key.to_bytes(), pool.tick_spacing).unwrap();
    let w = window_with(&pool, &words, None, 4, 3).unwrap();
    assert_eq!(w.starts, vec![-7_200, -3_600, 0, 3_600]);
    let w = window(&pool, &words, None).unwrap();
    assert_eq!(w, window_with(&pool, &words, None, MAX_ARRAYS, TRADER_ARRAYS_PER_SIDE).unwrap());
    assert_eq!(w.starts, vec![-10_800, -7_200, -3_600, 0, 3_600, 7_200, 10_800]);
}

/// A FLOOR pool whose own positions span more than `MAX_ARRAYS` arrays is inactive.
#[test]
fn a_floor_span_wider_than_the_window_is_inactive() {
    let mut f = floor_pool();
    let mut fp = f.floor_state.unwrap();
    fp.ladder_lower = fp.ladder_upper - 3_600 * 20;
    f.set_floor(fp);
    let mut words = decode_bitmap(&f.accounts[&f.bitmap].data, &f.key.to_bytes(), 60).unwrap();
    for i in 1..=20 {
        let start = fp.ladder_upper.div_euclid(3_600) * 3_600 - 3_600 * i;
        let bit = carpenter_quote::clmm::bitmap::array_bit(start, 60).unwrap();
        carpenter_quote::clmm::bitmap::set_initialized(&mut words, bit, true).unwrap();
    }
    f.accounts.get_mut(&f.bitmap).unwrap().data = encode_bitmap(&f.key.to_bytes(), 60, &words);
    let amm = built(&f);
    assert!(amm.window().floor_span_too_wide);
    assert!(!amm.is_active());
}

// ---- the encoder --------------------------------------------------------------------------------

#[test]
fn the_encoder_writes_the_native_swap_data() {
    use jupiter_amm_interface::Swap;
    for zfo in [true, false] {
        let d = swap_instruction_data(&Swap::Placeholder { data: Some(vec![zfo as u8]) }, 1_000_000_007, 42).unwrap();
        assert_eq!(&d[..8], &disc::amm::SWAP);
        assert_ne!(&d[..8], &disc::amm::SWAP_FROM_BALANCE);
        assert_eq!(d[8], zfo as u8);
        assert_eq!(i128::from_le_bytes(d[9..25].try_into().unwrap()), -1_000_000_007);
        assert_eq!(u128::from_le_bytes(d[25..41].try_into().unwrap()), no_limit(zfo));
        assert_eq!(u64::from_le_bytes(d[41..49].try_into().unwrap()), 42);
        assert_eq!(u32::from_le_bytes(d[49..53].try_into().unwrap()), 0);
        assert_eq!(d.len(), 53);
    }
    assert!(swap_instruction_data(&Swap::Placeholder { data: None }, 1, 0).is_err());
    assert!(swap_instruction_data(&Swap::Placeholder { data: Some(vec![2]) }, 1, 0).is_err());
    assert!(swap_instruction_data(&Swap::Saber, 1, 0).is_err());
}

/// The upstream variant encodes to exactly the bytes of the `Placeholder` it replaces, whatever its
/// counts: the counts delimit accounts and are not instruction data.
#[cfg(feature = "upstream-variant")]
#[test]
fn the_upstream_variant_encodes_like_the_placeholder() {
    use carpenter_amm::encode::{direction_of, swap_variant, SwapCounts};
    use jupiter_amm_interface::{AccountsType, RemainingAccountsInfo, RemainingAccountsSlice, Swap};
    for zfo in [true, false] {
        for counts in [
            SwapCounts::default(),
            SwapCounts { hook_accounts: 2, tick_arrays: 5, transfer_hook: None },
            SwapCounts { hook_accounts: 2, tick_arrays: 3, transfer_hook: Some((1, 5)) },
        ] {
            let v = swap_variant(zfo, counts);
            let Swap::Carpenter { zero_for_one, hook_accounts, tick_arrays, remaining_accounts_info } = &v else { panic!("{v:?}") };
            assert_eq!((*zero_for_one, *hook_accounts, *tick_arrays), (zfo, counts.hook_accounts, counts.tick_arrays));
            let want_info = counts.transfer_hook.map(|(side, length)| RemainingAccountsInfo {
                slices: vec![RemainingAccountsSlice { accounts_type: if side == 0 { AccountsType::TransferHookA } else { AccountsType::TransferHookB }, length }],
            });
            assert_eq!(remaining_accounts_info, &want_info);
            assert_eq!(direction_of(&v).unwrap(), zfo);
            assert_eq!(
                swap_instruction_data(&v, 1_000_000_007, 42).unwrap(),
                swap_instruction_data(&Swap::Placeholder { data: Some(vec![zfo as u8]) }, 1_000_000_007, 42).unwrap()
            );
        }
    }
    // A transfer-hook slice of length 0 is no slice.
    let Swap::Carpenter { remaining_accounts_info, .. } = swap_variant(true, SwapCounts { transfer_hook: Some((0, 0)), ..Default::default() }) else { unreachable!() };
    assert_eq!(remaining_accounts_info, None);
}

// ---- dependency and source rules ----------------------------------------------------------------

fn crate_root() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn sources() -> Vec<(String, String)> {
    std::fs::read_dir(crate_root().join("src"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("rs"))
        .map(|p| (p.display().to_string(), std::fs::read_to_string(&p).unwrap()))
        .collect()
}

/// No HTTP client, async runtime or RPC client in the library's resolved graph (normal and build
/// dependencies, reached from this crate), and no socket, file or timer in the source. The router's
/// clock is held in exactly one place (`from_keyed_account` keeps the context's handle) and read in
/// exactly one (`is_active`, for the guardian pause window): never by `quote`.
/// Dev-dependencies are excluded: the parity kit in `tests/parity.rs` carries an RPC snapshotter.
#[test]
fn network_free() {
    // `upstream/check.sh` substitutes the interface with `--config` patches; the same patches are
    // passed here (one per line) so the resolved graph matches the build under test.
    let mut cmd = std::process::Command::new(env!("CARGO"));
    cmd.args(["metadata", "--format-version", "1", "--locked", "--offline"]).current_dir(crate_root());
    for c in std::env::var("CARPENTER_AMM_CARGO_CONFIG").unwrap_or_default().lines().filter(|l| !l.is_empty()) {
        cmd.args(["--config", c]);
    }
    let out = cmd.output().expect("cargo metadata");
    assert!(out.status.success(), "cargo metadata: {}", String::from_utf8_lossy(&out.stderr));
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    let names_by_id: std::collections::HashMap<&str, &str> = meta["packages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["id"].as_str().unwrap(), p["name"].as_str().unwrap()))
        .collect();
    let nodes: std::collections::HashMap<&str, &serde_json::Value> =
        meta["resolve"]["nodes"].as_array().unwrap().iter().map(|n| (n["id"].as_str().unwrap(), n)).collect();
    let root = meta["resolve"]["root"].as_str().expect("resolve.root");
    let mut seen = std::collections::HashSet::from([root]);
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        for dep in nodes[id]["deps"].as_array().unwrap() {
            let not_dev = dep["dep_kinds"].as_array().unwrap().iter().any(|k| k["kind"].as_str() != Some("dev"));
            let pkg = dep["pkg"].as_str().unwrap();
            if not_dev && seen.insert(pkg) {
                stack.push(pkg);
            }
        }
    }
    let names: Vec<&str> = seen.iter().map(|id| names_by_id[id]).collect();
    assert!(names.contains(&"jupiter-amm-interface") && !names.contains(&"litesvm"), "the walk must cover normal deps only");
    for banned in ["reqwest", "ureq", "tokio", "hyper", "solana-client", "solana-rpc-client", "solana-rpc-client-api", "async-std"] {
        assert!(!names.contains(&banned), "{banned} is in the resolved dependency graph");
    }
    for (path, src) in sources() {
        for banned in ["std::net", "std::fs", "SystemTime", "Instant", "std::process"] {
            assert!(!src.contains(banned), "{path} mentions {banned}");
        }
        let (handles, reads) = (src.matches("clock_ref").count(), src.matches("self.clock").count());
        if path.ends_with("amm.rs") {
            assert_eq!((handles, reads), (1, 1), "{path}: the clock handle is kept once and read once");
            let body = |name: &str| {
                let at = src.find(&format!("fn {name}(")).unwrap_or_else(|| panic!("{path}: no fn {name}"));
                let rest = &src[at..];
                let end = rest[1..].find("\n    fn ").map(|e| e + 1).unwrap_or(rest.len());
                rest[..end].to_string()
            };
            assert!(body("is_active").contains("self.clock"), "{path}: is_active reads the clock");
            assert!(body("from_keyed_account_in").contains("amm_context.clock_ref"), "{path}: from_keyed_account(_in) keeps the handle");
        } else {
            assert_eq!((handles, reads), (0, 0), "{path} touches the router's clock");
        }
    }
}

/// The adapter links no math crate and names none of their modules: every amount is the quoters'.
#[test]
fn the_manifest_links_no_math_crate() {
    let manifest = std::fs::read_to_string(crate_root().join("Cargo.toml")).unwrap();
    for dep in ["floor-math", "carpenter-clmm-math"] {
        let declared = manifest.lines().any(|l| l.trim_start().starts_with(&format!("{dep} ")) || l.trim_start().starts_with(&format!("{dep}=")));
        assert!(!declared, "Cargo.toml declares {dep}");
    }
    for (path, src) in sources() {
        for module in ["tick_math", "swap_math", "liquidity_math", "full_math", "floor_math::", "sqrt_price_math"] {
            assert!(!src.contains(module), "{path} names {module}");
        }
    }
}
