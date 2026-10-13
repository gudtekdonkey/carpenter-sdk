//! [`CarpenterAmm`]: the `Amm` impl. Every required method and every provided one is written out, so
//! no default (`get_accounts_len`'s 32 in particular) is inherited by accident.
//!
//! All decoding happens in `update`; `quote` converts the parameters, calls the quoter and converts
//! the answer back.

use std::collections::{BTreeMap, HashSet};

use carpenter_quote::state::{decode_bitmap, decode_pool, decode_tick_array, TickArraySet};
use carpenter_quote::{no_limit, quote_swap_with, HookQuoter, PoolAccount, QuoteError, SwapQuote, SwapQuoteParams};
use carpenter_types::ids::{AmmClusterIds, AMM_CLUSTERS};
use carpenter_types::keys::TOKEN_2022_PROGRAM_ID;
use carpenter_types::constants::pool_is_paused;
use carpenter_types::layout::{tick_array_len_ok, Pool, TICK_ARRAY_RENT_OWNER_OFFSET};
use floor_quote::state::{decode_cushion_top, decode_floor_pool};
use floor_quote::{FloorError, FloorHookQuoter};
use floor_types::ids::{ClusterIds, CLUSTERS};
use floor_types::layout::FloorPool;
use jupiter_amm_interface::{
    AccountProvider, Amm, AmmContext, AmmError, AmmLabel, ClockRef, KeyedAccount, Quote, QuoteParams, SwapAndAccountMetas,
    SwapMode, SwapParams,
};
use solana_account::ReadableAccount;
use solana_pubkey::Pubkey;

use crate::accounts::{self, Window};
use crate::convert::{direction, is_missing_array, mint_supply, pk, quote_err, raw, to_quote};
use crate::encode::{swap_variant, SwapCounts};
use crate::hook::{transfer_hook_name, Deployment, KnownHook};
use crate::transfer_hook::{self as th, RawMeta};
use crate::{ACTION_POSITIONS_FLOOR, FIXED_METAS, LABEL};

/// The pool's Token-2022 transfer-hook situation, judged from the mint bytes `update` read by the
/// rule the program applies at every transfer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum BaseHook {
    /// No mint of the pool carries an armed transfer hook.
    #[default]
    None,
    /// The base mint is armed with the program the pool admits (`Pool.base_transfer_hook`), its hook
    /// authority renounced: every swap appends that hook's accounts, resolved from `list`.
    Admitted { program: Pubkey, validation: Pubkey, side: u8, list: Vec<RawMeta> },
    /// A mint is armed with a hook the program refuses on its leg (`QuoteHookArmed`,
    /// `TransferHookNotAdmitted`), or with the admitted one whose list cannot be resolved here. The
    /// pool is inactive and quotes are refused with this reason.
    Refused(String),
}

/// The transfer-hook verdict for a pool from its two mints' bytes and, when the base is armed with
/// the admitted program, the bytes of its validation account (`None` when the provider lacks it).
pub fn judge_base_hook(pool: &Pool, mints: [&[u8]; 2], validation: Option<&[u8]>) -> BaseHook {
    let admitted = (pool.base_transfer_hook != [0u8; 32]).then_some((pool.base_transfer_hook, pool.base_hook_side));
    let mut verdict = BaseHook::None;
    for (side, data) in mints.iter().enumerate() {
        let Some((authority, program)) = th::transfer_hook_of(data) else { continue };
        if program == [0u8; 32] {
            continue; // an empty slot: the transfer runs no hook
        }
        match admitted {
            Some((p, s)) if usize::from(s) == side && p == program && authority == [0u8; 32] => {
                let program = pk(&program);
                let mint = if side == 0 { pk(&pool.mint0) } else { pk(&pool.mint1) };
                let validation_key = th::extra_account_metas_pda(&mint, &program);
                let Some(list) = validation.and_then(th::execute_metas) else {
                    return BaseHook::Refused(format!("the base mint's transfer hook {program} has no readable extra-account-metas list at {validation_key}"));
                };
                if let Some(bad) = list.iter().find(|m| !resolvable(m)) {
                    return BaseHook::Refused(format!("{} (entry discriminator {})", th::ResolveErr::Unsupported, bad.discriminator));
                }
                verdict = BaseHook::Admitted { program, validation: validation_key, side: side as u8, list };
            }
            Some((p, s)) if usize::from(s) == side && p == program => {
                return BaseHook::Refused(format!("mint{side}'s transfer hook {} still has a live authority: TransferHookNotAdmitted", pk(&program)));
            }
            Some((_, s)) if usize::from(s) == side => {
                return BaseHook::Refused(format!("mint{side} is armed with {}, not the program this pool admits: TransferHookNotAdmitted", pk(&program)));
            }
            _ => return BaseHook::Refused(format!("mint{side} is armed with transfer hook {} on a leg that admits none: QuoteHookArmed", pk(&program))),
        }
    }
    verdict
}

/// The bytes a tick-array account is decoded from. A program-written array is 3,920 bytes: the
/// 3,888-byte `TickArray` account body, then a 32-byte rent owner that prices nothing; the body is
/// returned. Any other length is returned whole, so the decoder accepts a bare body and refuses
/// everything else (`QuoteBadAccount("TickArray")`) rather than reading a truncation.
pub fn tick_array_body(data: &[u8]) -> &[u8] {
    if tick_array_len_ok(data.len()) {
        &data[..TICK_ARRAY_RENT_OWNER_OFFSET]
    } else {
        data
    }
}

/// Whether [`th::hook_account_metas`] can resolve an entry with no account data in hand.
fn resolvable(m: &RawMeta) -> bool {
    match m.discriminator {
        0 => return true,
        1 => {}
        d if d >= 128 => {}
        _ => return false,
    }
    let c = &m.address_config;
    let mut i = 0usize;
    while i < c.len() {
        match c[i] {
            0 => return true,
            1 => i += 2 + usize::from(c.get(i + 1).copied().unwrap_or(u8::MAX)),
            2 => i += 3,
            3 => i += 2,
            _ => return false,
        }
    }
    i == c.len()
}

/// Everything one `update` pass decodes, held apart from `self` until the pass has fully succeeded,
/// so a pass that fails part-way never leaves a new price beside old tick arrays.
struct Staged {
    pool: Pool,
    floor: Option<Box<FloorPool>>,
    cushion_top: Option<i32>,
    supply0: u64,
    supply1: u64,
    window: Window,
    set: TickArraySet,
    array_keys: BTreeMap<i32, Pubkey>,
    base_hook: BaseHook,
}

/// The router's clock handle, read by [`CarpenterAmm::is_active`] only (a guardian pause is a time
/// window). No quote reads it.
#[derive(Clone, Default)]
struct RouterClock(ClockRef);

impl RouterClock {
    fn unix_timestamp(&self) -> i64 {
        self.0.unix_timestamp.load(std::sync::atomic::Ordering::Relaxed)
    }
}

impl std::fmt::Debug for RouterClock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RouterClock(unix_timestamp = {})", self.unix_timestamp())
    }
}

/// One Carpenter pool, as a Jupiter `Amm`.
#[derive(Clone, Debug)]
pub struct CarpenterAmm {
    key: Pubkey,
    /// The cluster's program ids, looked up by the pool account's owner.
    deployment: Deployment,
    pool: Pool,
    hook: KnownHook,
    bitmap: Pubkey,
    event_authority: Pubkey,
    /// The tick arrays and bitmap the last settled `update` read.
    set: Option<TickArraySet>,
    /// The PDA of every array in `set`, by start index.
    array_keys: BTreeMap<i32, Pubkey>,
    window: Window,
    supply0: u64,
    supply1: u64,
    base_hook: BaseHook,
    updated: bool,
    clock: RouterClock,
}

impl CarpenterAmm {
    /// The decoded pool, as of the last settled `update` (or `from_keyed_account`).
    pub fn pool(&self) -> &Pool {
        &self.pool
    }

    /// How this pool's hook is classified.
    pub fn hook(&self) -> &KnownHook {
        &self.hook
    }

    /// The cluster this pool belongs to: the `carpenter_amm` that owns it and FLOOR's programs beside
    /// it. Every address the adapter derives comes from these ids.
    pub fn deployment(&self) -> &Deployment {
        &self.deployment
    }

    /// [`Amm::from_keyed_account`] over a caller's cluster tables instead of the shipped
    /// `carpenter_types::ids::AMM_CLUSTERS` / `floor_types::ids::CLUSTERS`. The pool account's owner
    /// selects the cluster; an owner in no entry is refused.
    pub fn from_keyed_account_in(
        keyed_account: &KeyedAccount,
        amm_context: &AmmContext,
        amm_clusters: &[AmmClusterIds],
        floor_clusters: &[ClusterIds],
    ) -> Result<Self, AmmError> {
        let owner = keyed_account.account.owner;
        let deployment = Deployment::for_owner_in(&owner, amm_clusters, floor_clusters)
            .ok_or_else(|| AmmError::Custom(format!("account {} is owned by {owner}, not carpenter_amm on any known cluster", keyed_account.key)))?;
        let pool = decode_pool(&keyed_account.account.data).map_err(quote_err)?;
        let key = keyed_account.key;
        let hook = KnownHook::classify_in(&deployment, floor_clusters, &key, &pool).map_err(|e| AmmError::Custom(e.to_string()))?;
        let program = deployment.amm_program();
        Ok(Self {
            key,
            deployment,
            pool,
            hook,
            bitmap: accounts::bitmap_pda(&program, &key),
            event_authority: accounts::event_authority_pda(&program),
            set: None,
            array_keys: BTreeMap::new(),
            window: Window::default(),
            supply0: 0,
            supply1: 0,
            base_hook: BaseHook::None,
            updated: false,
            clock: RouterClock(amm_context.clock_ref.clone()),
        })
    }

    /// The tick-array window the last `update` computed.
    pub fn window(&self) -> &Window {
        &self.window
    }

    /// Whether the last `update` succeeded.
    pub fn is_updated(&self) -> bool {
        self.updated
    }

    /// The transfer-hook verdict of the last settled `update`.
    pub fn base_hook(&self) -> &BaseHook {
        &self.base_hook
    }

    /// `["extra-account-metas", base mint]` under `Pool.base_transfer_hook`, when the pool admits one.
    fn validation_key(pool: &Pool) -> Option<Pubkey> {
        if pool.base_transfer_hook == [0u8; 32] {
            return None;
        }
        let mint = if pool.base_hook_side == 0 { pk(&pool.mint0) } else { pk(&pool.mint1) };
        Some(th::extra_account_metas_pda(&mint, &pk(&pool.base_transfer_hook)))
    }

    /// One `update` pass, decoded into locals. Every `?` returns before `self` has changed.
    fn stage(&self, provider: &impl AccountProvider) -> Result<Staged, AmmError> {
        let pool_acc = provider.try_get(&self.key)?;
        let pool = decode_pool(pool_acc.data()).map_err(quote_err)?;
        drop(pool_acc);
        let bitmap_acc = provider.try_get(&self.bitmap)?;
        let words = decode_bitmap(bitmap_acc.data(), &raw(&self.key), pool.tick_spacing).map_err(quote_err)?;
        drop(bitmap_acc);

        // The hook account is optional: without it a FLOOR pool is inactive and quotes refuse with
        // `QuoteHookStateMissing`. The cushion top is read from the same bytes as the `FloorPool`.
        let (floor, cushion_top) = match &self.hook {
            KnownHook::Floor { floor_pool, .. } => match provider.get(floor_pool) {
                Some(acc) => {
                    let fp = decode_floor_pool(acc.data()).map_err(quote_err)?;
                    (Some(Box::new(fp)), decode_cushion_top(acc.data()))
                }
                None => (None, None),
            },
            _ => (None, None),
        };

        let mint0_data = provider.try_get(&pk(&pool.mint0))?.data().to_vec();
        let mint1_data = provider.try_get(&pk(&pool.mint1))?.data().to_vec();
        let supply0 = mint_supply(&mint0_data).ok_or_else(|| AmmError::Custom("carpenter-amm: mint0 is not a mint".into()))?;
        let supply1 = mint_supply(&mint1_data).ok_or_else(|| AmmError::Custom("carpenter-amm: mint1 is not a mint".into()))?;
        let validation = Self::validation_key(&pool).and_then(|k| provider.get(&k).map(|a| a.data().to_vec()));
        let base_hook = judge_base_hook(&pool, [&mint0_data, &mint1_data], validation.as_deref());

        let window = accounts::window(&pool, &words, floor.as_deref()).map_err(quote_err)?;
        let mut arrays = Vec::with_capacity(window.starts.len());
        let mut array_keys = BTreeMap::new();
        for start in &window.starts {
            let key = accounts::tick_array_pda(&self.deployment.amm_program(), &self.key, *start);
            // An array the provider does not carry is left out; with the bitmap supplied, a walk
            // that reaches it is refused by name rather than read as empty.
            if let Some(acc) = provider.get(&key) {
                let array = decode_tick_array(tick_array_body(acc.data())).map_err(quote_err)?;
                if array.start_index != *start || array.pool != raw(&self.key) {
                    return Err(AmmError::Custom(format!("carpenter-amm: tick array {key} is not this pool's array at {start}")));
                }
                arrays.push(array);
                array_keys.insert(*start, key);
            }
        }
        // `TickArraySet::from_accounts` without the second decode: the same constructor, the same
        // checks, with the bitmap's words.
        let set = TickArraySet::new(&raw(&self.key), pool.tick_spacing, arrays, Some(words)).map_err(quote_err)?;
        Ok(Staged { pool, floor, cushion_top, supply0, supply1, window, set, array_keys, base_hook })
    }

    /// The exact-in quote for `amount` of `input_mint`, plus FLOOR's hook total on the quote leg (0
    /// on any other pool). The only call site of the quoter in this crate: `quote` and
    /// `get_swap_and_account_metas` both come through here. Refused until an `update` has settled.
    fn quote_exact_in(&self, input_mint: &Pubkey, output_mint: &Pubkey, amount: u64) -> Result<(SwapQuote, u64, bool), AmmError> {
        if !self.updated {
            return Err(AmmError::Custom("carpenter-amm: no settled update(): the last pass failed or none has run".into()));
        }
        if let BaseHook::Refused(why) = &self.base_hook {
            return Err(AmmError::Custom(format!("carpenter-amm: {why}")));
        }
        let zero_for_one = direction(&self.pool, input_mint, output_mint)?;
        let set = self.set.as_ref().ok_or_else(|| AmmError::Custom("carpenter-amm: no tick arrays read".into()))?;
        let account = PoolAccount { key: raw(&self.key), pool: self.pool };
        let params = SwapQuoteParams {
            zero_for_one,
            amount_specified: -i128::from(amount),
            sqrt_price_limit_x64: no_limit(zero_for_one),
            supply0: self.supply0,
            supply1: self.supply1,
        };
        match &self.hook {
            KnownHook::Floor { state, cushion_top, .. } => {
                // `classify` made this a FLOOR pool only with the owning cluster's FLOOR ids.
                let cluster = self.deployment.floor.ok_or_else(|| AmmError::Custom("carpenter-amm: no FLOOR deployment on this cluster".into()))?;
                let mut floor_q = FloorHookQuoter::for_cluster(state.as_deref().copied(), *cushion_top, &cluster);
                let mut quoters: [&mut dyn HookQuoter; 1] = [&mut floor_q];
                let q = quote_swap_with(&account, set, &mut quoters, &params).map_err(quote_err)?;
                Ok((q, floor_q.taken.total(), zero_for_one))
            }
            // A plain pool needs no quoter; an unknown swap hook finds none and is refused with
            // `QuoteHookNotQuotable`.
            KnownHook::Inert | KnownHook::Unknown { .. } => {
                let q = quote_swap_with(&account, set, &mut [], &params).map_err(quote_err)?;
                Ok((q, 0, zero_for_one))
            }
        }
    }

    fn exact_out_refused() -> AmmError {
        quote_err(QuoteError::from(FloorError::ExactOutNotSupported))
    }
}

impl Amm for CarpenterAmm {
    /// Look the cluster up by the pool account's owner (an owner that is `carpenter_amm` on no
    /// known cluster is refused), decode the pool and classify its hook against that cluster's FLOOR
    /// ids. The context's clock handle is kept for `is_active`
    /// alone (a guardian pause ends at a unix time); no quote reads it, so a quote stays a function of
    /// account bytes alone.
    fn from_keyed_account(keyed_account: &KeyedAccount, amm_context: &AmmContext) -> Result<Self, AmmError> {
        Self::from_keyed_account_in(keyed_account, amm_context, AMM_CLUSTERS, CLUSTERS)
    }

    fn label(&self) -> AmmLabel {
        LABEL
    }

    fn program_id(&self) -> Pubkey {
        self.deployment.amm_program()
    }

    fn key(&self) -> Pubkey {
        self.key
    }

    fn get_reserve_mints(&self) -> Vec<Pubkey> {
        vec![pk(&self.pool.mint0), pk(&self.pool.mint1)]
    }

    /// The pool, its bitmap, FLOOR's `floor_pool` on a FLOOR pool, both mints (their supplies are
    /// hook-callback inputs), the window's tick arrays, and last the base transfer hook's validation
    /// account when the pool admits one. Before the first `update` the window is empty: the first
    /// pass reads the bitmap, the second the arrays it marks.
    fn get_accounts_to_update(&self) -> Vec<Pubkey> {
        let mut keys = vec![self.key, self.bitmap];
        if let KnownHook::Floor { floor_pool, .. } = &self.hook {
            keys.push(*floor_pool);
        }
        keys.push(pk(&self.pool.mint0));
        keys.push(pk(&self.pool.mint1));
        let program = self.deployment.amm_program();
        keys.extend(self.window.starts.iter().map(|s| accounts::tick_array_pda(&program, &self.key, *s)));
        if let Some(v) = Self::validation_key(&self.pool) {
            keys.push(v);
        }
        keys
    }

    /// Decode everything a quote reads. The pool, the bitmap and both mints are required
    /// (`AccountNotFound` otherwise); `floor_pool` and the tick arrays are read when present. The
    /// window is recomputed from the fresh bitmap and hook state, and its arrays are read from the
    /// same provider.
    ///
    /// All or nothing: a pass that fails leaves the previous pass's state untouched and clears
    /// `is_updated`, so `is_active()` is false and `quote` refuses until a pass succeeds.
    fn update(&mut self, account_provider: impl AccountProvider) -> Result<(), AmmError> {
        let staged = match self.stage(&account_provider) {
            Ok(s) => s,
            Err(e) => {
                self.updated = false;
                return Err(e);
            }
        };
        let Staged { pool, floor, cushion_top: top, supply0, supply1, window, set, array_keys, base_hook } = staged;
        self.pool = pool;
        if let KnownHook::Floor { state, cushion_top, .. } = &mut self.hook {
            *state = floor;
            *cushion_top = top;
        }
        self.supply0 = supply0;
        self.supply1 = supply1;
        self.window = window;
        self.set = Some(set);
        self.array_keys = array_keys;
        self.base_hook = base_hook;
        self.updated = true;
        Ok(())
    }

    /// Exact-in only. `amount_specified = -amount`, no price limit, the direction from the mints;
    /// the amounts are the quoter's, and the fee triple is [`to_quote`]'s mapping.
    fn quote(&self, quote_params: &QuoteParams) -> Result<Quote, AmmError> {
        if quote_params.swap_mode == SwapMode::ExactOut {
            return Err(Self::exact_out_refused());
        }
        let (q, hook_total, zero_for_one) = self.quote_exact_in(&quote_params.input_mint, &quote_params.output_mint, quote_params.amount)?;
        Ok(to_quote(&q, hook_total, zero_for_one, &self.pool, self.hook.floor_state()))
    }

    /// The `swap` accounts with exactly the tick arrays the quote at `in_amount` reads
    /// (`SwapQuote::all_arrays`: the trader's walk and every hook action's), the hook positions its
    /// `ModifyPosition` actions modify, and `Swap::Placeholder { data: [zero_for_one] }` (with the
    /// `upstream-variant` feature, `Swap::Carpenter` with the same layout's counts).
    ///
    /// `missing_dynamic_accounts_as_default` is honoured only for `TickArrayMissing` /
    /// `HookActionAccountMissing` (a walk that left the window): the fixed accounts are returned with
    /// no arrays. Every other refusal is returned whatever the flag says.
    fn get_swap_and_account_metas(&self, swap_params: &SwapParams) -> Result<SwapAndAccountMetas, AmmError> {
        if swap_params.swap_mode == SwapMode::ExactOut {
            return Err(Self::exact_out_refused());
        }
        let program = self.deployment.amm_program();
        let quoted = self.quote_exact_in(&swap_params.source_mint, &swap_params.destination_mint, swap_params.in_amount);
        let (starts, positions, zero_for_one, amount_out) = match quoted {
            Ok((q, _, zfo)) => (q.all_arrays(), accounts::action_positions(&program, &self.key, &self.pool, &q.hook_actions), zfo, q.amount_out),
            Err(e) if swap_params.missing_dynamic_accounts_as_default && is_missing_array(&e) => {
                let zfo = direction(&self.pool, &swap_params.source_mint, &swap_params.destination_mint)?;
                (Vec::new(), Vec::new(), zfo, 0)
            }
            Err(e) => return Err(e),
        };
        // The quoter reads only arrays `update` cached, so every start has a key here; the PDA is
        // derived as a fallback rather than substituting a default key.
        let tick_arrays: Vec<Pubkey> = starts
            .into_iter()
            .map(|start| self.array_keys.get(&start).copied().unwrap_or_else(|| accounts::tick_array_pda(&program, &self.key, start)))
            .collect();
        let mut account_metas = accounts::swap_metas(
            &program,
            &self.key,
            &self.pool,
            &self.bitmap,
            &self.event_authority,
            &self.hook,
            &swap_params.token_transfer_authority,
            &swap_params.source_token_account,
            &swap_params.destination_token_account,
            &tick_arrays,
            &positions,
        );
        // An admitted base transfer hook: its accounts for the one base transfer of this swap. Base
        // in is `source -> vault` by the trader for `in_amount`; base out is `vault -> destination`
        // by the pool PDA for the quoted output.
        let before_transfer_hook = account_metas.len();
        if let BaseHook::Admitted { program, validation, side, list } = &self.base_hook {
            let in_side = if zero_for_one { 0u8 } else { 1u8 };
            let (mint, vault) = if *side == 0 { (pk(&self.pool.mint0), pk(&self.pool.vault0)) } else { (pk(&self.pool.mint1), pk(&self.pool.vault1)) };
            let (source, destination, authority, amount) = if in_side == *side {
                (swap_params.source_token_account, vault, swap_params.token_transfer_authority, swap_params.in_amount)
            } else {
                (vault, swap_params.destination_token_account, self.key, amount_out)
            };
            let hook = th::hook_account_metas(list, program, validation, &source, &mint, &destination, &authority, amount)
                .map_err(|e| AmmError::Custom(e.to_string()))?;
            th::merge_metas(&mut account_metas, hook);
        }
        let count = |n: usize, what: &str| u8::try_from(n).map_err(|_| AmmError::Custom(format!("carpenter-amm: {n} {what} do not fit a u8 count")));
        let counts = SwapCounts {
            hook_accounts: if self.hook.calls_back() { self.pool.hook_callback_accounts } else { 0 },
            tick_arrays: count(tick_arrays.len() + positions.len(), "tick arrays and positions")?,
            transfer_hook: match &self.base_hook {
                BaseHook::Admitted { side, .. } => Some((*side, count(account_metas.len() - before_transfer_hook, "transfer-hook accounts")?)),
                _ => None,
            },
        };
        Ok(SwapAndAccountMetas { swap: swap_variant(zero_for_one, counts), account_metas })
    }

    /// `true`: the number of tick arrays a swap carries depends on the amount, and on a FLOOR pool
    /// on the hook's actions.
    fn has_dynamic_accounts(&self) -> bool {
        true
    }

    /// `false`: the mints are in the pool account `from_keyed_account` reads.
    fn requires_update_for_reserve_mints(&self) -> bool {
        false
    }

    /// `false`. FLOOR pools refuse exact-out on chain (`ExactOutNotSupported`), and the
    /// swap encodings carry exact-in arguments only. Plain pools are exact-in here as well.
    fn supports_exact_out(&self) -> bool {
        false
    }

    /// `false`: both directions trade.
    fn unidirectional(&self) -> bool {
        false
    }

    /// The programs a swap invokes, for the test kit: the pool's known hook program, Token-2022 when
    /// either side uses it, and the base transfer hook the pool admits (`floor_holders` by name,
    /// any other as `base_transfer_hook`).
    fn program_dependencies(&self) -> Vec<(Pubkey, String)> {
        let mut deps = Vec::new();
        if self.hook.calls_back() {
            if let Some(name) = crate::hook::known_hook_name(&self.pool.hook) {
                deps.push((pk(&self.pool.hook), name.to_string()));
            }
        }
        if self.pool.token_program0 == TOKEN_2022_PROGRAM_ID || self.pool.token_program1 == TOKEN_2022_PROGRAM_ID {
            deps.push((pk(&TOKEN_2022_PROGRAM_ID), "spl_token_2022".to_string()));
        }
        if self.pool.base_transfer_hook != [0u8; 32] {
            deps.push((pk(&self.pool.base_transfer_hook), transfer_hook_name(&self.pool.base_transfer_hook).to_string()));
        }
        deps
    }

    /// An upper bound on the accounts a swap carries in the current state: the fixed accounts, the
    /// hook slice, every array in the window, the hook position an in-swap action modifies, and the
    /// base transfer hook's entries plus its list and program. `get_swap_and_account_metas` selects from exactly this set.
    fn get_accounts_len(&self) -> usize {
        let slice = if self.hook.calls_back() { usize::from(self.pool.hook_callback_accounts) } else { 0 };
        let transfer_hook = match &self.base_hook {
            BaseHook::Admitted { list, .. } => list.len() + 2,
            _ => 0,
        };
        // The positions an in-swap action may modify: FLOOR's in-swap actions are the bid re-place,
        // which modifies the one bid position.
        let positions = if self.hook.calls_back() { ACTION_POSITIONS_FLOOR } else { 0 };
        FIXED_METAS + slice + self.window.starts.len() + positions + transfer_hook
    }

    /// `None`: the pool is its own liquidity.
    fn underlying_liquidities(&self) -> Option<HashSet<Pubkey>> {
        None
    }

    /// Whether this adapter can answer for the pool at all (not a market-health check). False when:
    /// no `update` has settled; the pool is paused — the authority's flag, or a guardian pause whose
    /// `guardian_paused_until` is still ahead of the router's clock — or has no in-range liquidity; the hook is a swap
    /// hook this crate has no quoter for (`QuoteHookNotQuotable`); a FLOOR pool's `floor_pool` was not
    /// supplied (`QuoteHookStateMissing`), is not bound to this pool or is not seeded yet
    /// (`ladder_seeded != 1`: the quoter refuses every swap with `NotSeeded`, as the program does);
    /// FLOOR's position span is wider
    /// than the window; or a mint carries a transfer hook the program would refuse.
    fn is_active(&self) -> bool {
        let paused = pool_is_paused(self.pool.paused, self.pool.guardian_paused_until, self.clock.unix_timestamp());
        if !self.updated || paused || self.pool.liquidity.get() == 0 || self.window.floor_span_too_wide {
            return false;
        }
        if matches!(self.base_hook, BaseHook::Refused(_)) {
            return false;
        }
        match &self.hook {
            KnownHook::Inert => true,
            KnownHook::Unknown { .. } => false,
            KnownHook::Floor { state, .. } => match state {
                Some(fp) => fp.amm_pool == raw(&self.key) && fp.pool_bound == 1 && fp.ladder_seeded == 1,
                None => false,
            },
        }
    }
}
