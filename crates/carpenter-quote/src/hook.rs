//! The interface between the AMM's quoter and an approved hook's quoter.
//!
//! On chain the AMM calls a hook program at fixed points of a swap and applies the record each call
//! returns (the hook interface in `carpenter_types::hook`). Off chain nothing can be simulated, so
//! every approved hook whose callbacks change a swap ships a Rust replay of them, and
//! [`crate::quote_swap`] calls that replay at the same points, in the same order, and applies its
//! records with the same rules (`carpenter_types::hook::validate_record`, `HookDeltaAssembly`).
//!
//! | Pool | What [`crate::quote_swap`] does |
//! |---|---|
//! | no hook, or a hook with neither `BEFORE_SWAP_FLAG` nor `AFTER_SWAP_FLAG` | the plain walk; any quoter passed is not called |
//! | a hook with a swap flag, and a quoter that [accepts](HookQuoter::accepts) the pool's `hook` | the round protocol below |
//! | a hook with a swap flag, and no quoter for it | refused: [`QuoteError::QuoteHookNotQuotable`] |
//!
//! The round protocol, in the order the AMM runs it:
//!
//! ```text
//! prepare → prelude (amount ≠ 0, ≤ u64::MAX)
//! → before_swap (round 0, if BEFORE_SWAP_FLAG) → the walk
//! → after_swap (round 1, if AFTER_SWAP_FLAG) → the hook delta is credited once
//! → the record's actions execute (hooks skipped) → after_actions (round 2, 3, …) while
//!   `want_callback`, each round's actions executed in turn, capped by `hook_max_rounds`
//! ```
//!
//! A hook carrying `SETTLES_FLAG` (Uniswap v4's settle) may also mint into the pool's vaults inside a
//! callback, cap the trader's walk with a price limit from `before_swap`, take the unfilled remainder
//! of an exact-in from `after_swap`, and return negative deltas (a payout from its claims). The quoter
//! replays the AMM's side of all four; what the hook minted is reported by [`HookQuoter::settled`].
//!
//! A hook quoter keeps its own state (the hook's account, decoded) and updates it as its program
//! would; whatever it wants to report beyond the AMM's [`crate::SwapQuote`] (its fee breakdown, its
//! post-state) it reads off itself after the quote returns. FLOOR's implementation is
//! `floor_quote::FloorHookQuoter`.
//!
//! A hook quoter must refuse exactly what its program refuses, under the program's error name
//! ([`QuoteError::Hook`]), and must read no clock, randomness, float or network: a quote is a pure
//! function of the account bytes and the parameters.

use carpenter_types::hook::{ActionResult, BalanceDelta, HookRecordV1};

use crate::error::{QuoteError, QuoteResult};
use crate::swap::PoolAccount;

/// What the AMM hands a hook's swap callbacks, plus the pool fields a hook binds its state to. One
/// value per quote, built before the prelude; it does not change between rounds.
#[derive(Clone, Copy, Debug)]
pub struct HookSwapCtx {
    /// The pool account's key.
    pub pool_key: [u8; 32],
    pub zero_for_one: bool,
    /// v4 sign: `< 0` exact-in, `> 0` exact-out.
    pub amount_specified: i128,
    /// `pool.tick_current` before the swap.
    pub pre_tick: i32,
    /// `mint0.supply` / `mint1.supply`, as the AMM reads them into the callback args.
    pub supply0: u64,
    pub supply1: u64,
    /// The hook's claims before the swap (`hook_claims0/1`).
    pub claims0: u64,
    pub claims1: u64,
    pub mint0: [u8; 32],
    pub mint1: [u8; 32],
    pub tick_spacing: u16,
    /// The pool's key fee (`DYNAMIC_FEE_FLAG` on a dynamic-fee pool).
    pub fee: u32,
}

/// The post-walk values `after_swap` receives.
#[derive(Clone, Copy, Debug)]
pub struct AfterSwapIn {
    pub post_sqrt_price_x64: u128,
    pub post_tick: i32,
    /// The walk's raw delta, caller perspective.
    pub swap_delta: BalanceDelta,
    /// What `before_swap` returned (0 when it did not run).
    pub before_delta_specified: i128,
    pub before_delta_unspecified: i128,
}

/// An approved hook's swap callbacks, replayed off-chain. See the module docs for when each is
/// called. Every method returns the record the hook program would return for the same input; the
/// quoter validates it (`validate_record`) exactly as the AMM does.
pub trait HookQuoter {
    /// The hook program this quoter replays (on a hook deployed on several clusters, one of its ids).
    fn program_id(&self) -> [u8; 32];

    /// Whether this quoter replays the hook program `hook`. A pool is quoted through this quoter
    /// only when this returns true for its `hook` field. The default is `program_id() == hook`; a
    /// hook deployed under a different id on each cluster overrides it to accept each of its ids.
    fn accepts(&self, hook: &[u8; 32]) -> bool {
        self.program_id() == *hook
    }

    /// Called once, before the prelude, on a pool this quoter replays. Refuse here a quote the
    /// quoter cannot answer at all — typically [`QuoteError::QuoteHookStateMissing`] when the hook's
    /// account was never supplied. The default accepts.
    fn prepare(&mut self, pool: &PoolAccount) -> QuoteResult<()> {
        let _ = pool;
        Ok(())
    }

    /// Round 0, when the pool sets `BEFORE_SWAP_FLAG`.
    fn before_swap(&mut self, ctx: &HookSwapCtx) -> QuoteResult<HookRecordV1>;

    /// Round 1, when the pool sets `AFTER_SWAP_FLAG`. A record with `want_callback` set is followed
    /// by its actions and then [`HookQuoter::after_actions`].
    fn after_swap(&mut self, ctx: &HookSwapCtx, after: &AfterSwapIn) -> QuoteResult<HookRecordV1>;

    /// Round 2 and later, after the previous round's actions executed. `results` are those actions'
    /// results in order (empty when the round requested none); each carries the pool's price and tick
    /// after that action. `results_carry_fees` is false when a `ModifyPosition` result's `fees0/1`
    /// are not the position's real accrued fees (the quoter cannot know them: no `Position` account
    /// in a quote). `claims0/1` are the hook's claims after those actions. The default refuses, for
    /// a hook that never sets `want_callback`.
    fn after_actions(
        &mut self,
        ctx: &HookSwapCtx,
        round: u8,
        results: &[ActionResult],
        results_carry_fees: bool,
        claims0: u64,
        claims1: u64,
    ) -> QuoteResult<HookRecordV1> {
        let _ = (ctx, round, results, results_carry_fees, claims0, claims1);
        Err(QuoteError::QuoteActionNotSupported)
    }

    /// `SETTLES_FLAG` only: what the hook minted (or deposited) into the pool's two vaults during the
    /// callback it has just answered, `[vault0, vault1]`. Read once after every callback of a pool
    /// whose hook settles, and credited to the hook's claims as the AMM credits a vault increase. The
    /// default is `[0, 0]` (a hook that never settles; never read for one).
    fn settled(&mut self) -> [u64; 2] {
        [0, 0]
    }
}
