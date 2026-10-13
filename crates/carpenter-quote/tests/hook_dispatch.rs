//! The `HookQuoter` dispatch: which pools reach a hook quoter, which are refused, and that a hook
//! quoter's records are applied by the AMM's rules. The hook here is a test double; FLOOR's real
//! quoter is tested in `floor-quote`.

use carpenter_quote::clmm::tick_math;
use carpenter_quote::synth::{build_pool, PoolKeys, SynthKind, SynthPool, SynthPosition};
use carpenter_quote::types::constants::{AFTER_SWAP_FLAG, BEFORE_SWAP_FLAG, BEFORE_SWAP_RETURNS_DELTA_FLAG, DYNAMIC_FEE_FLAG};
use carpenter_quote::types::hook::{
    ActionResult, HookRecordV1, HOOK_INTERFACE_VERSION, PHASE_AFTER_ACTIONS, PHASE_AFTER_SWAP, PHASE_BEFORE_SWAP,
};
use carpenter_quote::{
    is_swap_hooked, no_limit, quote_plain, quote_swap, quote_swap_with, AfterSwapIn, HookQuoter, HookSwapCtx, PoolAccount,
    QuoteError, QuoteResult, SwapQuoteParams,
};

const HOOK: [u8; 32] = [0x5A; 32];
const OTHER: [u8; 32] = [0x6B; 32];

fn keys(hook: [u8; 32]) -> PoolKeys {
    PoolKeys {
        pool: [7; 32],
        mint0: [1; 32],
        mint1: [2; 32],
        vault0: [3; 32],
        vault1: [4; 32],
        token_program0: [5; 32],
        token_program1: [6; 32],
        hook,
        hook_authority: if hook == [0; 32] { [0; 32] } else { [9; 32] },
    }
}

fn pool(hook: [u8; 32], flags: u16) -> SynthPool {
    let sqrt = tick_math::get_sqrt_price_at_tick(0).unwrap();
    let kind = if hook == [0; 32] {
        SynthKind::Plain { fee: 3_000 }
    } else {
        SynthKind::Hooked { fee: DYNAMIC_FEE_FLAG, lp_fee: 3_000, flags, max_rounds: 3, callback_accounts: 1 }
    };
    let positions = [SynthPosition { tick_lower: -6_000, tick_upper: 6_000, liquidity: 10_000_000_000_000 }];
    build_pool(&keys(hook), kind, 60, sqrt, 9, 6, &positions).unwrap()
}

fn params(zfo: bool, amount: i128) -> SwapQuoteParams {
    SwapQuoteParams { zero_for_one: zfo, amount_specified: amount, sqrt_price_limit_x64: no_limit(zfo), supply0: 11, supply1: 22 }
}

/// Returns pass-through records, optionally charging a fee on the specified leg in `before_swap`,
/// and records what it was called with.
#[derive(Default)]
struct Recorder {
    id: [u8; 32],
    fee_on_specified: i128,
    refuse_prepare: bool,
    calls: Vec<&'static str>,
    ctx: Option<HookSwapCtx>,
    after: Option<AfterSwapIn>,
}

impl HookQuoter for Recorder {
    fn program_id(&self) -> [u8; 32] {
        self.id
    }
    fn prepare(&mut self, _pool: &PoolAccount) -> QuoteResult<()> {
        self.calls.push("prepare");
        if self.refuse_prepare {
            return Err(QuoteError::QuoteHookStateMissing);
        }
        Ok(())
    }
    fn before_swap(&mut self, ctx: &HookSwapCtx) -> QuoteResult<HookRecordV1> {
        self.calls.push("before_swap");
        self.ctx = Some(*ctx);
        let mut r = HookRecordV1::pass_through(HOOK_INTERFACE_VERSION, PHASE_BEFORE_SWAP, 0);
        r.delta_specified = self.fee_on_specified;
        r.fees = [self.fee_on_specified as u64, 0, 0, 0, 0];
        Ok(r)
    }
    fn after_swap(&mut self, _ctx: &HookSwapCtx, after: &AfterSwapIn) -> QuoteResult<HookRecordV1> {
        self.calls.push("after_swap");
        self.after = Some(*after);
        Ok(HookRecordV1::pass_through(HOOK_INTERFACE_VERSION, PHASE_AFTER_SWAP, 1))
    }
    fn after_actions(&mut self, _: &HookSwapCtx, round: u8, _: &[ActionResult], _: bool, _: u64, _: u64) -> QuoteResult<HookRecordV1> {
        self.calls.push("after_actions");
        Ok(HookRecordV1::pass_through(HOOK_INTERFACE_VERSION, PHASE_AFTER_ACTIONS, round))
    }
}

#[test]
fn a_plain_pool_never_calls_a_quoter() {
    let sp = pool([0; 32], 0);
    let set = sp.tick_array_set().unwrap();
    let mut h = Recorder { id: HOOK, ..Default::default() };
    let with = quote_swap(&sp.account, &set, Some(&mut h), &params(true, -1_000_000)).unwrap();
    let without = quote_plain(&sp.account, &set, &params(true, -1_000_000)).unwrap();
    assert!(h.calls.is_empty());
    assert_eq!((with.amount_in, with.amount_out), (without.amount_in, without.amount_out));
    assert_eq!(bytemuck::bytes_of(&with.pool_after), bytemuck::bytes_of(&without.pool_after));
}

#[test]
fn a_hook_with_no_swap_flag_is_quoted_as_plain_without_its_quoter() {
    let sp = pool(HOOK, 0);
    assert!(!is_swap_hooked(&sp.account.pool));
    let set = sp.tick_array_set().unwrap();
    let q = quote_plain(&sp.account, &set, &params(false, -1_000_000)).unwrap();
    assert!(q.amount_out > 0);
    assert_eq!(q.fee_split0, [0; 5]);
}

#[test]
fn a_swap_hook_without_its_quoter_is_refused_never_quoted_as_plain() {
    for flags in [BEFORE_SWAP_FLAG, AFTER_SWAP_FLAG, BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG] {
        let sp = pool(HOOK, flags);
        let set = sp.tick_array_set().unwrap();
        assert_eq!(quote_plain(&sp.account, &set, &params(true, -1_000)).unwrap_err(), QuoteError::QuoteHookNotQuotable);
        // a quoter for ANOTHER hook program is not this pool's quoter
        let mut other = Recorder { id: OTHER, ..Default::default() };
        assert_eq!(
            quote_swap(&sp.account, &set, Some(&mut other), &params(true, -1_000)).unwrap_err(),
            QuoteError::QuoteHookNotQuotable
        );
        assert!(other.calls.is_empty());
        // and the refusal comes before the prelude, as on chain
        assert_eq!(quote_plain(&sp.account, &set, &params(true, 0)).unwrap_err(), QuoteError::QuoteHookNotQuotable);
    }
}

#[test]
fn quote_swap_with_picks_the_quoter_by_program_id() {
    let sp = pool(HOOK, BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG);
    let set = sp.tick_array_set().unwrap();
    let mut a = Recorder { id: OTHER, ..Default::default() };
    let mut b = Recorder { id: HOOK, ..Default::default() };
    quote_swap_with(&sp.account, &set, &mut [&mut a, &mut b], &params(true, -1_000)).unwrap();
    assert!(a.calls.is_empty());
    assert_eq!(b.calls, ["prepare", "before_swap", "after_swap"]);
    let mut none: [&mut dyn HookQuoter; 0] = [];
    assert_eq!(quote_swap_with(&sp.account, &set, &mut none, &params(true, -1_000)).unwrap_err(), QuoteError::QuoteHookNotQuotable);
}

#[test]
fn prepare_refuses_before_the_prelude() {
    let sp = pool(HOOK, BEFORE_SWAP_FLAG);
    let set = sp.tick_array_set().unwrap();
    let mut h = Recorder { id: HOOK, refuse_prepare: true, ..Default::default() };
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, 0)).unwrap_err(), QuoteError::QuoteHookStateMissing);
    assert_eq!(h.calls, ["prepare"]);
}

#[test]
fn only_the_flagged_callbacks_run_and_they_see_the_amms_arguments() {
    let set_for = |sp: &SynthPool| sp.tick_array_set().unwrap();
    let sp = pool(HOOK, BEFORE_SWAP_FLAG);
    let mut h = Recorder { id: HOOK, ..Default::default() };
    quote_swap(&sp.account, &set_for(&sp), Some(&mut h), &params(true, -1_000)).unwrap();
    assert_eq!(h.calls, ["prepare", "before_swap"]);
    let c = h.ctx.unwrap();
    assert_eq!((c.pool_key, c.zero_for_one, c.amount_specified, c.supply0, c.supply1), ([7; 32], true, -1_000, 11, 22));
    assert_eq!((c.mint0, c.mint1, c.tick_spacing, c.fee, c.pre_tick), ([1; 32], [2; 32], 60, DYNAMIC_FEE_FLAG, 0));

    let sp = pool(HOOK, AFTER_SWAP_FLAG);
    let mut h = Recorder { id: HOOK, ..Default::default() };
    let q = quote_swap(&sp.account, &set_for(&sp), Some(&mut h), &params(true, -1_000)).unwrap();
    assert_eq!(h.calls, ["prepare", "after_swap"]);
    let a = h.after.unwrap();
    assert_eq!((a.post_sqrt_price_x64, a.post_tick, a.swap_delta), (q.sqrt_after_trade, q.tick_after_trade, q.swap_delta));
}

#[test]
fn a_pass_through_hook_quotes_as_the_plain_walk_at_the_stored_lp_fee() {
    let hooked = pool(HOOK, BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG);
    let plain = pool([0; 32], 0);
    for (zfo, amount) in [(true, -1_000_000i128), (false, -1_000_000), (true, 5_000), (false, 5_000)] {
        let mut h = Recorder { id: HOOK, ..Default::default() };
        let q = quote_swap(&hooked.account, &hooked.tick_array_set().unwrap(), Some(&mut h), &params(zfo, amount)).unwrap();
        let p = quote_plain(&plain.account, &plain.tick_array_set().unwrap(), &params(zfo, amount)).unwrap();
        assert_eq!((q.amount_in, q.amount_out, q.fees, q.sqrt_after), (p.amount_in, p.amount_out, p.fees, p.sqrt_after));
        assert_eq!(q.hook_delta.amount0 | q.hook_delta.amount1, 0);
    }
}

#[test]
fn a_hook_fee_on_the_specified_leg_is_credited_once_and_reported_in_the_split() {
    // a record may carry a delta only on a pool whose flags allow it
    let no_delta = pool(HOOK, BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG);
    let mut h = Recorder { id: HOOK, fee_on_specified: 10_000, ..Default::default() };
    let e = quote_swap(&no_delta.account, &no_delta.tick_array_set().unwrap(), Some(&mut h), &params(true, -1_000_000)).unwrap_err();
    assert_eq!(e.name(), "HookFieldNotAllowed");

    let sp = pool(HOOK, BEFORE_SWAP_FLAG | BEFORE_SWAP_RETURNS_DELTA_FLAG | AFTER_SWAP_FLAG);
    let mut h = Recorder { id: HOOK, fee_on_specified: 10_000, ..Default::default() };
    let q = quote_swap(&sp.account, &sp.tick_array_set().unwrap(), Some(&mut h), &params(true, -1_000_000)).unwrap();
    assert_eq!(q.amount_in, 1_000_000, "exact-in: the trader pays what they named");
    assert_eq!(q.hook_delta.amount0, 10_000);
    assert_eq!(q.pool_after.hook_claims0, 10_000);
    assert_eq!(q.fee_split0, [10_000, 0, 0, 0, 0]);
    assert_eq!(q.fee_split1, [0; 5]);
    // the walk swapped what was left
    let plain = pool([0; 32], 0);
    let p = quote_plain(&plain.account, &plain.tick_array_set().unwrap(), &params(true, -990_000)).unwrap();
    assert_eq!(q.amount_out, p.amount_out);
}

#[test]
fn a_record_the_amm_would_refuse_is_refused() {
    struct Liar;
    impl HookQuoter for Liar {
        fn program_id(&self) -> [u8; 32] {
            HOOK
        }
        fn before_swap(&mut self, _: &HookSwapCtx) -> QuoteResult<HookRecordV1> {
            // the wrong phase for round 0
            Ok(HookRecordV1::pass_through(HOOK_INTERFACE_VERSION, PHASE_AFTER_SWAP, 0))
        }
        fn after_swap(&mut self, _: &HookSwapCtx, _: &AfterSwapIn) -> QuoteResult<HookRecordV1> {
            unreachable!()
        }
    }
    let sp = pool(HOOK, BEFORE_SWAP_FLAG);
    let e = quote_swap(&sp.account, &sp.tick_array_set().unwrap(), Some(&mut Liar), &params(true, -1_000)).unwrap_err();
    assert_eq!(e.name(), "HookFieldNotAllowed");
}

/// A hook that refuses every swap in `before_swap` (an ungraduated pool, for example): the refusal
/// reaches the caller as `QuoteError::Hook(name)` with the hook program's own error name, for either
/// direction and any amount, exact-out included (the AMM forwards an exact-out swap to the hook; the
/// hook decides). A prelude refusal (`AmountZero`) still comes first, as on chain.
#[test]
fn a_before_swap_refusal_surfaces_under_the_hooks_name() {
    struct Refuser {
        calls: u32,
    }
    impl HookQuoter for Refuser {
        fn program_id(&self) -> [u8; 32] {
            HOOK
        }
        fn before_swap(&mut self, _: &HookSwapCtx) -> QuoteResult<HookRecordV1> {
            self.calls += 1;
            Err(QuoteError::Hook("NotSeeded"))
        }
        fn after_swap(&mut self, _: &HookSwapCtx, _: &AfterSwapIn) -> QuoteResult<HookRecordV1> {
            unreachable!("before_swap refused")
        }
    }
    let sp = pool(HOOK, BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG);
    let set = sp.tick_array_set().unwrap();
    let mut h = Refuser { calls: 0 };
    let mut n = 0;
    for zfo in [true, false] {
        for amount in [-1i128, -1_000, -1_000_000_000, 1_000] {
            let e = quote_swap(&sp.account, &set, Some(&mut h), &params(zfo, amount)).unwrap_err();
            assert_eq!(e, QuoteError::Hook("NotSeeded"), "zfo {zfo} amount {amount}");
            assert_eq!(e.name(), "NotSeeded");
            n += 1;
        }
    }
    assert_eq!(h.calls, n);
    let e = quote_swap_with(&sp.account, &set, &mut [&mut h], &params(true, -1_000)).unwrap_err();
    assert_eq!(e.name(), "NotSeeded");
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, 0)).unwrap_err(), QuoteError::AmountZero);
    assert_eq!(h.calls, n + 1, "the prelude refused before the hook was called");
}

/// On chain the pause check precedes every callback: a paused hooked pool is `PoolPaused`, and its
/// quoter is never called, not even `prepare`.
#[test]
fn a_paused_hooked_pool_is_refused_before_any_callback() {
    let mut sp = pool(HOOK, BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG);
    sp.account.pool.paused = 1;
    let set = sp.tick_array_set().unwrap();
    let mut h = Recorder { id: HOOK, refuse_prepare: true, ..Default::default() };
    assert_eq!(quote_swap(&sp.account, &set, Some(&mut h), &params(true, -1_000)).unwrap_err(), QuoteError::PoolPaused);
    assert!(h.calls.is_empty(), "{:?}", h.calls);
    // an unknown swap hook on a paused pool: the chain's answer, not the quoter's
    assert_eq!(quote_swap(&sp.account, &set, None, &params(true, -1_000)).unwrap_err(), QuoteError::PoolPaused);
}

/// A hook deployed under a different program id on each cluster: one quoter that accepts each of
/// its ids is called for a pool hooked by any of them, and for no other hook.
#[test]
fn a_quoter_that_accepts_several_program_ids_is_used_for_each_and_no_other() {
    struct MultiCluster {
        ids: [[u8; 32]; 2],
        inner: Recorder,
    }
    impl HookQuoter for MultiCluster {
        fn program_id(&self) -> [u8; 32] {
            self.ids[0]
        }
        fn accepts(&self, hook: &[u8; 32]) -> bool {
            self.ids.contains(hook)
        }
        fn before_swap(&mut self, ctx: &HookSwapCtx) -> QuoteResult<HookRecordV1> {
            self.inner.before_swap(ctx)
        }
        fn after_swap(&mut self, ctx: &HookSwapCtx, after: &AfterSwapIn) -> QuoteResult<HookRecordV1> {
            self.inner.after_swap(ctx, after)
        }
    }
    let flags = BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG;
    for hook in [HOOK, OTHER] {
        let sp = pool(hook, flags);
        let set = sp.tick_array_set().unwrap();
        let mut m = MultiCluster { ids: [HOOK, OTHER], inner: Recorder::default() };
        quote_swap(&sp.account, &set, Some(&mut m), &params(true, -1_000)).unwrap();
        assert_eq!(m.inner.calls, ["before_swap", "after_swap"]);
        let mut m = MultiCluster { ids: [HOOK, OTHER], inner: Recorder::default() };
        quote_swap_with(&sp.account, &set, &mut [&mut m], &params(true, -1_000)).unwrap();
        assert_eq!(m.inner.calls, ["before_swap", "after_swap"]);
    }
    let third = pool([0x7C; 32], flags);
    let mut m = MultiCluster { ids: [HOOK, OTHER], inner: Recorder::default() };
    assert_eq!(
        quote_swap(&third.account, &third.tick_array_set().unwrap(), Some(&mut m), &params(true, -1_000)).unwrap_err(),
        QuoteError::QuoteHookNotQuotable
    );
    assert!(m.inner.calls.is_empty());
}
