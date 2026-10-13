//! Shared test fixtures: synthetic pools as account maps at their real PDAs, and an
//! `AccountProvider` over the map. The pools are built with the quoters' own `synth` modules, which
//! write the same bytes `initialize_pool` and `modify_liquidity` write.
#![allow(dead_code)]

use std::collections::{BTreeMap, HashMap};

use carpenter_amm::accounts::{bitmap_pda, tick_array_pda};
use carpenter_amm::CarpenterAmm;
use carpenter_quote::clmm::tick_math;
use carpenter_quote::state::{encode_account, encode_bitmap};
use carpenter_quote::{no_limit, PoolAccount};
use carpenter_types::constants as cc;
use carpenter_types::ids::AMM_DEVNET;
use carpenter_types::keys::{SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, WSOL_MINT};
use carpenter_types::layout::Pool;
use floor_quote::synth::{build_pool, floor_pool_defaults, PoolKeys, SynthKind, SynthPool, SynthPosition, FLOOR};
use floor_types::constants as fc;
use floor_types::ids::DEVNET;
use floor_types::layout::FloorPool;
use floor_types::seeds;
use jupiter_amm_interface::{AccountProvider, Amm, AmmContext, FeeMode, KeyedAccount, QuoteParams, SwapMode, SwapParams};
use solana_account::{Account, ReadableAccount};
use solana_pubkey::Pubkey;

pub const RENT_PLENTY: u64 = 10_000_000_000;
/// The supply the synthetic quote mint (token0) reports.
pub const RICH: u64 = u64::MAX / 4;
/// A program-written tick array: the 3,888-byte body, then a 32-byte rent owner.
pub const TICK_ARRAY_ACCOUNT_LEN: usize = 3_920;
pub const TICK_ARRAY_BODY_LEN: usize = 3_888;

pub const JUP: Pubkey = Pubkey::from_str_const("JUP6LkbZbjS1jKKwapdHNy74zcZ3tLUZoi5QNyVTaV4");

pub fn pk(b: [u8; 32]) -> Pubkey {
    Pubkey::new_from_array(b)
}

pub fn amm_id() -> Pubkey {
    pk(AMM_DEVNET.carpenter_amm)
}

pub fn floor_hook() -> Pubkey {
    pk(DEVNET.floor_hook)
}

pub fn spl_token() -> Pubkey {
    pk(SPL_TOKEN_PROGRAM_ID)
}

pub fn token_2022() -> Pubkey {
    pk(TOKEN_2022_PROGRAM_ID)
}

pub fn wsol() -> Pubkey {
    pk(WSOL_MINT)
}

/// SPL Token `Mint` (82 B): no authorities, `supply`, `decimals`, initialized.
pub fn mint_data(supply: u64, decimals: u8) -> Vec<u8> {
    let mut d = vec![0u8; 82];
    d[36..44].copy_from_slice(&supply.to_le_bytes());
    d[44] = decimals;
    d[45] = 1;
    d
}

/// A tick array as the program stores it: the body, then a zero rent owner.
pub fn tick_array_account(body: Vec<u8>) -> Vec<u8> {
    assert_eq!(body.len(), TICK_ARRAY_BODY_LEN);
    let mut d = body;
    d.extend_from_slice(&[0u8; 32]);
    d
}

pub fn account(owner: Pubkey, data: Vec<u8>) -> Account {
    Account { lamports: RENT_PLENTY, data, owner, executable: false, rent_epoch: 0 }
}

/// A synthetic pool and every account a quote reads.
#[derive(Clone)]
pub struct Synth {
    pub key: Pubkey,
    pub bitmap: Pubkey,
    pub mint0: Pubkey,
    pub mint1: Pubkey,
    pub vault0: Pubkey,
    pub vault1: Pubkey,
    pub floor_pool: Option<Pubkey>,
    pub floor_state: Option<FloorPool>,
    /// Every array the pool has, by start index.
    pub arrays: BTreeMap<i32, Pubkey>,
    pub synth: SynthPool,
    pub accounts: HashMap<Pubkey, Account>,
}

impl Synth {
    pub fn keyed_account(&self) -> KeyedAccount {
        KeyedAccount { key: self.key, account: self.accounts[&self.key].clone(), params: None }
    }

    pub fn provider(&self) -> Provider {
        Provider(self.accounts.clone())
    }

    pub fn pool(&self) -> Pool {
        carpenter_quote::state::decode_pool(&self.accounts[&self.key].data).unwrap()
    }

    /// Rewrite the pool account (and the copy the reference quotes from).
    pub fn set_pool(&mut self, pool: Pool) {
        self.synth.account.pool = pool;
        self.accounts.get_mut(&self.key).unwrap().data = encode_account(&pool);
    }

    /// Rewrite the `FloorPool` account (and the reference's copy).
    pub fn set_floor(&mut self, fp: FloorPool) {
        self.floor_state = Some(fp);
        self.accounts.get_mut(&self.floor_pool.unwrap()).unwrap().data = encode_account(&fp);
    }

    pub fn quote_params(&self, zfo: bool, amount: u64, mode: SwapMode) -> QuoteParams {
        let (i, o) = if zfo { (self.mint0, self.mint1) } else { (self.mint1, self.mint0) };
        QuoteParams { amount, input_mint: i, output_mint: o, swap_mode: mode, fee_mode: FeeMode::Normal }
    }

    /// The quoters called directly on the same bytes the adapter reads: the reference every adapter
    /// answer is compared against.
    pub fn reference(&self, zfo: bool, amount: u64) -> Result<floor_quote::SwapQuote, carpenter_quote::QuoteError> {
        let set = self.synth.tick_array_set().unwrap();
        let params = floor_quote::SwapQuoteParams {
            zero_for_one: zfo,
            amount_specified: -(amount as i128),
            sqrt_price_limit_x64: no_limit(zfo),
            supply0: RICH,
            supply1: fc::SUPPLY,
            cushion_top: None,
        };
        floor_quote::quote_swap(&PoolAccount { key: self.key.to_bytes(), pool: self.pool() }, &set, self.floor_state.as_ref(), &params)
    }
}

pub fn swap_params<'a>(s: &Synth, zfo: bool, amount: u64, user: Pubkey, jup: &'a Pubkey, missing_as_default: bool) -> SwapParams<'static, 'a> {
    let (i, o) = if zfo { (s.mint0, s.mint1) } else { (s.mint1, s.mint0) };
    SwapParams {
        swap_mode: SwapMode::ExactIn,
        in_amount: amount,
        out_amount: 0,
        source_mint: i,
        destination_mint: o,
        source_token_account: Pubkey::new_from_array([0xA1; 32]),
        destination_token_account: Pubkey::new_from_array([0xA2; 32]),
        token_transfer_authority: user,
        user,
        payer: user,
        quote_mint_to_referrer: None,
        jupiter_program_id: jup,
        missing_dynamic_accounts_as_default: missing_as_default,
    }
}

/// Construct and settle an adapter the way a router does: `from_keyed_account`, then `update` until
/// the window's arrays have been read (the first pass reads the bitmap, the second the arrays).
pub fn built(s: &Synth) -> CarpenterAmm {
    let mut amm = CarpenterAmm::from_keyed_account(&s.keyed_account(), &context()).expect("from_keyed_account");
    for _ in 0..2 {
        amm.update(s.provider()).expect("update");
    }
    amm
}

pub struct PoolSpec {
    pub mint0: Pubkey,
    pub mint1: Pubkey,
    pub tp0: Pubkey,
    pub tp1: Pubkey,
    pub fee_key: u32,
    pub spacing: u16,
    pub hook: Pubkey,
    pub kind: SynthKind,
    pub sqrt: u128,
    pub positions: Vec<SynthPosition>,
}

/// Build the pool PDA and every account a swap reads, on devnet's `carpenter_amm`.
pub fn install(spec: &PoolSpec, floor: Option<&dyn Fn(&SynthPool) -> (FloorPool, u64, u64)>) -> Synth {
    install_on(amm_id(), spec, floor)
}

/// [`install`] under the `carpenter_amm` program `amm` (a FLOOR pool's `FloorPool` under `spec.hook`).
pub fn install_on(amm: Pubkey, spec: &PoolSpec, floor: Option<&dyn Fn(&SynthPool) -> (FloorPool, u64, u64)>) -> Synth {
    assert!(spec.mint0 < spec.mint1, "mint0 < mint1");
    let (key, bump) = Pubkey::find_program_address(
        &[seeds::POOL, spec.mint0.as_ref(), spec.mint1.as_ref(), &spec.fee_key.to_le_bytes(), &spec.spacing.to_le_bytes(), spec.hook.as_ref()],
        &amm,
    );
    let (bitmap, bitmap_bump) = Pubkey::find_program_address(&[seeds::BITMAP, key.as_ref()], &amm);
    assert_eq!(bitmap, bitmap_pda(&amm, &key));
    let (vault0, v0b) = Pubkey::find_program_address(&[seeds::VAULT, key.as_ref(), spec.mint0.as_ref()], &amm);
    let (vault1, v1b) = Pubkey::find_program_address(&[seeds::VAULT, key.as_ref(), spec.mint1.as_ref()], &amm);
    let hook_authority = if spec.hook == Pubkey::default() {
        Pubkey::default()
    } else {
        Pubkey::find_program_address(&[seeds::HOOK_AUTHORITY], &spec.hook).0
    };
    let keys = PoolKeys {
        pool: key.to_bytes(),
        mint0: spec.mint0.to_bytes(),
        mint1: spec.mint1.to_bytes(),
        vault0: vault0.to_bytes(),
        vault1: vault1.to_bytes(),
        token_program0: spec.tp0.to_bytes(),
        token_program1: spec.tp1.to_bytes(),
        hook: spec.hook.to_bytes(),
        hook_authority: hook_authority.to_bytes(),
    };
    let mut sp = build_pool(&keys, spec.kind, spec.spacing, spec.sqrt, 9, 6, &spec.positions).expect("synth pool");
    sp.account.pool.bump = bump;
    sp.account.pool.bitmap_bump = bitmap_bump;
    sp.account.pool.vault0_bump = v0b;
    sp.account.pool.vault1_bump = v1b;

    let mut accounts = HashMap::new();
    let (claims0, claims1, floor_pool, floor_state) = match floor {
        Some(make) => {
            let (mut fp, c0, c1) = make(&sp);
            let (fp_key, fp_bump) = Pubkey::find_program_address(&[seeds::FLOOR_POOL, key.as_ref()], &spec.hook);
            fp.bump = fp_bump;
            accounts.insert(fp_key, account(spec.hook, encode_account(&fp)));
            (c0, c1, Some(fp_key), Some(fp))
        }
        None => (0, 0, None, None),
    };
    sp.account.pool.hook_claims0 = claims0;
    sp.account.pool.hook_claims1 = claims1;

    accounts.insert(key, account(amm, encode_account(&sp.account.pool)));
    accounts.insert(bitmap, account(amm, encode_bitmap(&key.to_bytes(), spec.spacing, &sp.words)));
    let mut arrays = BTreeMap::new();
    for a in &sp.arrays {
        let k = tick_array_pda(&amm, &key, a.start_index);
        accounts.insert(k, account(amm, tick_array_account(encode_account(a))));
        arrays.insert(a.start_index, k);
    }
    accounts.insert(spec.mint0, account(spec.tp0, mint_data(RICH, 9)));
    accounts.insert(spec.mint1, account(spec.tp1, mint_data(fc::SUPPLY, 6)));

    Synth { key, bitmap, mint0: spec.mint0, mint1: spec.mint1, vault0, vault1, floor_pool, floor_state, arrays, synth: sp, accounts }
}

fn sorted(a: [u8; 32], b: [u8; 32]) -> (Pubkey, Pubkey) {
    let (a, b) = (pk(a), pk(b));
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

/// A plain 0.3 % pool at spacing 60 around tick 0: a deep range `[-600, 600]` and a wide one
/// `[-60000, 60000]`.
pub fn plain_pool() -> Synth {
    let (mint0, mint1) = sorted([0x11; 32], [0x22; 32]);
    install(
        &PoolSpec {
            mint0,
            mint1,
            tp0: spl_token(),
            tp1: spl_token(),
            fee_key: 3_000,
            spacing: 60,
            hook: Pubkey::default(),
            kind: SynthKind::Plain { fee: 3_000 },
            sqrt: tick_math::get_sqrt_price_at_tick(0).unwrap(),
            positions: vec![
                SynthPosition { tick_lower: -600, tick_upper: 600, liquidity: 5_000_000_000_000 },
                SynthPosition { tick_lower: -60_000, tick_upper: 60_000, liquidity: 1_000_000_000_000 },
            ],
        },
        None,
    )
}

/// A plain pool with `n` disjoint positions, each in its own tick array.
pub fn plain_pool_with_arrays(n: usize) -> Synth {
    let (mint0, mint1) = sorted([0x33; 32], [0x44; 32]);
    let positions = (0..n as i32)
        .map(|i| {
            let lo = (i - (n as i32) / 2) * 3_600 + 60;
            SynthPosition { tick_lower: lo, tick_upper: lo + 600, liquidity: 1_000_000_000_000 }
        })
        .collect();
    install(
        &PoolSpec {
            mint0,
            mint1,
            tp0: spl_token(),
            tp1: spl_token(),
            fee_key: 500,
            spacing: 60,
            hook: Pubkey::default(),
            kind: SynthKind::Plain { fee: 500 },
            sqrt: tick_math::get_sqrt_price_at_tick(100).unwrap(),
            positions,
        },
        None,
    )
}

/// A graduated-shaped FLOOR pool: wSOL / a Token-2022 base, the market at `open + 30`, the floor
/// 200 spacings above `open` (tick-space, quote = token0), a seed position, a ladder, a bid one spacing
/// wide at the floor, an outside LP, and a vault of 22 SOL beside the bid. Skim 250 bps far, creator
/// 25 bps.
pub fn floor_pool() -> Synth {
    floor_pool_on(amm_id(), floor_hook())
}

/// [`floor_pool`] under the `carpenter_amm` program `amm`, hooked by `hook`.
pub fn floor_pool_on(amm: Pubkey, hook: Pubkey) -> Synth {
    let open = 100_020i32;
    let floor_tick = open + 60 * 200;
    let s = 60;
    let f = |t: i32| t.div_euclid(s) * s;
    let seed = SynthPosition { tick_lower: f(open - 6_932), tick_upper: f(open), liquidity: 3_400_000_000_000 };
    let ladder = SynthPosition { tick_lower: f(open - 20_794), tick_upper: f(open - 6_932), liquidity: 5_700_000_000_000 };
    let bid = SynthPosition { tick_lower: floor_tick, tick_upper: floor_tick + s, liquidity: 400_000_000_000_000 };
    let lp = SynthPosition { tick_lower: f(open - 9_000), tick_upper: f(open + 9_000), liquidity: 500_000_000_000 };
    let mut base = [0u8; 32];
    base[0] = 0xF1;
    base[31] = 0xB1;
    let base = pk(base);
    assert!(wsol() < base, "the base mint sorts above wSOL");
    let vault_eth = 22_000_000_000u64;
    let make = move |sp: &SynthPool| -> (FloorPool, u64, u64) {
        let k = &sp.account;
        let mut fp = floor_pool_defaults(k.key, k.pool.mint0, k.pool.mint1, [0x0C; 32]);
        fp.skim_buy_far_bps = 250;
        fp.skim_sell_far_bps = 250;
        fp.creator_buy_bps = 25;
        fp.creator_sell_bps = 25;
        fp.floor_tick = floor_tick;
        fp.vault_eth = vault_eth;
        fp.seed_lower = seed.tick_lower;
        fp.seed_upper = seed.tick_upper;
        fp.seed_liq = seed.liquidity.into();
        fp.ladder_lower = ladder.tick_lower;
        fp.ladder_upper = ladder.tick_upper;
        fp.ladder_liq = ladder.liquidity.into();
        fp.ladder_seeded = 1;
        let (_, ladder_tokens) = carpenter_quote::amounts_for_liquidity(
            k.pool.sqrt_price_x64.get(),
            k.pool.tick_current,
            ladder.tick_lower,
            ladder.tick_upper,
            ladder.liquidity as i128,
        )
        .unwrap();
        fp.ladder_tokens_remaining = ladder_tokens;
        let (bid_eth, _) =
            carpenter_quote::amounts_for_liquidity(k.pool.sqrt_price_x64.get(), k.pool.tick_current, bid.tick_lower, bid.tick_upper, bid.liquidity as i128)
                .unwrap();
        fp.bid_lower = bid.tick_lower;
        fp.bid_liquidity = bid.liquidity.into();
        fp.bid_eth_deployed = bid_eth;
        // The hook's claims cover the vault exactly.
        (fp, vault_eth, 0)
    };
    install_on(
        amm,
        &PoolSpec {
            mint0: wsol(),
            mint1: base,
            tp0: spl_token(),
            tp1: token_2022(),
            fee_key: cc::DYNAMIC_FEE_FLAG,
            spacing: fc::FLOOR_TICK_SPACING,
            hook,
            kind: FLOOR,
            sqrt: tick_math::get_sqrt_price_at_tick(open + 30).unwrap(),
            positions: vec![seed, ladder, bid, lp],
        },
        Some(&make),
    )
}

/// `AccountProvider` over a map.
pub struct Provider(pub HashMap<Pubkey, Account>);

impl AccountProvider for Provider {
    fn get(&self, pubkey: &Pubkey) -> Option<impl ReadableAccount + use<'_>> {
        self.0.get(pubkey).cloned()
    }
}

pub fn context() -> AmmContext {
    AmmContext::default()
}
