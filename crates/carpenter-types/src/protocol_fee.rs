//! The Carpenter Platform fee — Uniswap v4's PROTOCOL fee on Carpenter pools — as pure integer
//! arithmetic, so the AMM program's walk and the off-chain quoter's
//! (`crates/carpenter-quote/src/walk.rs`) run ONE implementation and cannot drift.
//!
//! The rules:
//! - 500 pips by default, ON TOP of the pool's LP fee; v4's composition `p + l − p·l/1e6` stands
//!   ([`compose_swap_fee`]).
//! - EVERY pool pays, FLOOR's included, and every swap pays: the hook's own swaps (the ratchet's
//!   buy, the in-swap nudge) too.
//! - The rate is COPIED into a pool at `initialize_pool` from `AmmConfig`'s current setting and never
//!   written into a live pool. A pool over a mint whose `MintExclusivity` RECORDED a rate at
//!   registration — a FLOOR launch's mint, at `create_launch` — copies THAT rate instead
//!   ([`rate_for_new_pool`]), so a launch graduates at the rate of its creation.
//! - The fee is taken on the SOL / listed-quote LEG on buys AND sells — on a sell, out of what the
//!   seller receives — and a pool of two plain tokens takes it in the input coin
//!   ([`currency_for_roles`], [`fee_currency`]). That departs from v4, which always charges the
//!   INPUT currency; this module is where the departure lives.
//!
//! How a walk charges it (both walks, identically):
//! - fee on the INPUT currency: v4 exactly. Each step runs at `compose_swap_fee(p, lp)`, and the
//!   protocol's share of the step fee is [`input_share`] (the whole fee when the LP fee is 0, else
//!   `floor(gross_in · p / 1e6)` — rounded down, in the LPs' favour, per STEP).
//! - fee on the OUTPUT currency: the steps run at the LP fee alone (the protocol's rate never enters
//!   the curve), and ONE [`output_share`] `floor(gross_out · p / 1e6)` is taken off the walk's whole
//!   output, which stays in the vault as `protocol_fees_owed` of that currency. An exact-out walk
//!   first grosses its target up ([`gross_for_net`]) so the caller receives exactly what it named.
//!
//! No Solana crate, no float, no panic on any input: every function is total.

use crate::constants::{
    MAX_PROTOCOL_FEE, PIPS_DENOMINATOR, PROTOCOL_FEE_ON_CURRENCY0,
    PROTOCOL_FEE_ON_CURRENCY1, PROTOCOL_FEE_ON_INPUT,
};

/// v4 `ProtocolFeeLibrary.getZeroForOneFee` / `getOneForZeroFee`: the 12-bit nibble a direction
/// reads — the LOW nibble for `zero_for_one`, the next for the other way.
pub const fn directional(packed: u32, zero_for_one: bool) -> u32 {
    if zero_for_one {
        packed & 0xfff
    } else {
        (packed >> 12) & 0xfff
    }
}

/// The inverse of [`directional`]: `zero_for_one | one_for_zero << 12`. Each argument is masked to
/// its nibble; validate with [`is_valid`] first.
pub const fn pack(zero_for_one: u32, one_for_zero: u32) -> u32 {
    (zero_for_one & 0xfff) | ((one_for_zero & 0xfff) << 12)
}

/// v4 `ProtocolFeeLibrary.isValidProtocolFee`, with this crate's cap: each direction at most
/// [`MAX_PROTOCOL_FEE`], and nothing above the two nibbles.
pub const fn is_valid(packed: u32) -> bool {
    packed >> 24 == 0 && directional(packed, true) <= MAX_PROTOCOL_FEE && directional(packed, false) <= MAX_PROTOCOL_FEE
}

/// v4 `ProtocolFeeLibrary.calculateSwapFee`: `p + l − p·l / 1e6`, the rate a step is charged when the
/// protocol fee rides the input. `p` is one direction's pips (masked to 12 bits), `l` the LP fee.
/// 500 on a 2,500 LP fee is 2,999, not 3,000: the protocol takes its share first and the LP fee
/// applies to the remainder.
pub const fn compose_swap_fee(protocol_fee: u32, lp_fee: u32) -> u32 {
    let p = (protocol_fee & 0xfff) as u64;
    let l = (lp_fee & 0x00ff_ffff) as u64;
    (p + l - (p * l) / PIPS_DENOMINATOR as u64) as u32
}

/// The currency (0 / 1) a swap in this direction pays its protocol fee in, from the pool's
/// `protocol_fee_currency` code. Any code but the two currency codes is the v4 input rule — a zeroed
/// pool reads [`PROTOCOL_FEE_ON_INPUT`].
pub const fn fee_currency(protocol_fee_currency: u8, zero_for_one: bool) -> u8 {
    if protocol_fee_currency == PROTOCOL_FEE_ON_CURRENCY0 {
        0
    } else if protocol_fee_currency == PROTOCOL_FEE_ON_CURRENCY1 {
        1
    } else if zero_for_one {
        0
    } else {
        1
    }
}

/// Whether this swap's protocol fee is charged on its OUTPUT (the listed quote is what it receives).
pub const fn on_output(protocol_fee_currency: u8, zero_for_one: bool) -> bool {
    let input = if zero_for_one { 0 } else { 1 };
    fee_currency(protocol_fee_currency, zero_for_one) != input
}

/// What one walk charges, resolved once before its first step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WalkFee {
    /// The rate every step runs at: v4's composed rate when the fee rides the input, the LP fee
    /// alone otherwise (and the LP fee alone whenever the direction's protocol pips are 0).
    pub swap_fee: u32,
    /// The direction's protocol pips when they ride the INPUT (the per-step share), else 0.
    pub input_pips: u32,
    /// The direction's protocol pips when they ride the OUTPUT (one share off the whole output),
    /// else 0.
    pub output_pips: u32,
    /// The currency (0 / 1) the protocol fee is booked in (`protocol_fees_owed0` / `owed1`).
    pub currency: u8,
}

/// Resolve a walk's rates: the pool's packed fee and fee-currency code, the direction, and the LP fee
/// the walk was handed (a hook override, or `pool.lp_fee`).
pub const fn walk_fee(packed: u32, protocol_fee_currency: u8, zero_for_one: bool, lp_fee: u32) -> WalkFee {
    let p = directional(packed, zero_for_one);
    let currency = fee_currency(protocol_fee_currency, zero_for_one);
    if p == 0 {
        WalkFee { swap_fee: lp_fee, input_pips: 0, output_pips: 0, currency }
    } else if on_output(protocol_fee_currency, zero_for_one) {
        WalkFee { swap_fee: lp_fee, input_pips: 0, output_pips: p, currency }
    } else {
        WalkFee { swap_fee: compose_swap_fee(p, lp_fee), input_pips: p, output_pips: 0, currency }
    }
}

/// v4 `Pool.swap`'s per-step split of the step fee when the protocol fee rides the input: the WHOLE
/// fee when the LP fee is 0 (`swap_fee == protocol`), else `floor(gross_in · p / 1e6)`, rounded down
/// to favour LPs. `None` when the share would exceed the fee — impossible for a fee `compute_swap`
/// charged at `compose_swap_fee(p, lp)` (it charges `fee ≥ gross_in · swap_fee / 1e6` on every
/// branch and `swap_fee ≥ p`), so a `None` is a mis-derived composition and the caller must fail
/// rather than round it away (v4's `unchecked` subtraction would underflow there).
pub const fn input_share(step_fee: u64, gross_in: u64, swap_fee: u32, protocol_pips: u32) -> Option<u64> {
    if protocol_pips == 0 {
        return Some(0);
    }
    let share = if swap_fee == protocol_pips {
        step_fee
    } else {
        ((gross_in as u128) * (protocol_pips as u128) / PIPS_DENOMINATOR as u128) as u64
    };
    if share <= step_fee {
        Some(share)
    } else {
        None
    }
}

/// The share taken off a walk's whole OUTPUT when the fee rides it: `floor(gross_out · p / 1e6)`.
/// Always `≤ gross_out` (p < 1e6), so the caller's net output never goes negative.
pub const fn output_share(gross_out: u64, protocol_pips: u32) -> u64 {
    ((gross_out as u128) * (protocol_pips as u128) / PIPS_DENOMINATOR as u128) as u64
}

/// An exact-out walk whose fee rides the output must walk to a GROSS output `G` whose net
/// `G − output_share(G, p)` is exactly `net`. This is the least such `G` (the trader's input is then
/// the least it can be), or `None` if it does not fit a `u64`.
///
/// `f(G) = G − floor(G·p/1e6)` never decreases and rises by 0 or 1 per unit of `G`, so it takes every
/// value, and the least `G` with `f(G) ≥ net` has `f(G) == net` exactly. The search starts at
/// `ceil(net·1e6 / (1e6 − p))`, where `f ≥ net` already holds, and steps down while the unit below
/// still reaches `net` — at most twice for any `p ≤ 0xfff` (a zero step of `f` happens at most once in
/// every `1e6/p ≥ 244` units); the bound of four is a proof obligation checked exhaustively by the
/// unit tests below, not a limit a real input meets.
pub const fn gross_for_net(net: u64, protocol_pips: u32) -> Option<u64> {
    if protocol_pips == 0 || net == 0 {
        return Some(net);
    }
    let p = protocol_pips as u128;
    if p >= PIPS_DENOMINATOR as u128 {
        return None;
    }
    let d = PIPS_DENOMINATOR as u128;
    let n = net as u128;
    // ceil(n·d / (d − p)); n < 2^64 and d < 2^20, so n·d fits u128
    let mut g = (n * d + (d - p) - 1) / (d - p);
    let mut i = 0;
    while i < 4 && g > n {
        let below = g - 1;
        if below - below * p / d >= n {
            g = below;
        } else {
            break;
        }
        i += 1;
    }
    if g > u64::MAX as u128 {
        None
    } else {
        Some(g as u64)
    }
}

/// The fee-currency code a pool is born with, from the two mints' ROLES at `initialize_pool`
/// (`MintRole::ListedQuote` = wSOL or a `Listed` roster mint) and which side, if either, IS wSOL
/// (`keys::WSOL_MINT`). In order:
/// 1. a pool with a wSOL side → the wSOL side. wSOL is always a listed quote on the AMM, so a
///    wSOL / base pool answers what rule 2 would; what this rule decides is the pool whose OTHER side
///    is a listed quote too (e.g. a wSOL / tokenized-stock route pool), where the fee is taken on the
///    SOL leg, so the treasury never collects the other quote mint.
/// 2. exactly ONE listed quote → that side (a FLOOR pool: its quote).
/// 3. neither listed, or two listed quotes of which neither is wSOL → the v4 input rule.
///    Two non-SOL quotes (a stock / USDC pool) is the one shape still on the input coin: such a pool
///    has no SOL leg, and no listed quote outranks another.
pub const fn currency_for_roles(listed_quote0: bool, listed_quote1: bool, wsol0: bool, wsol1: bool) -> u8 {
    match (wsol0, wsol1) {
        (true, false) => return PROTOCOL_FEE_ON_CURRENCY0,
        (false, true) => return PROTOCOL_FEE_ON_CURRENCY1,
        _ => {}
    }
    match (listed_quote0, listed_quote1) {
        (true, false) => PROTOCOL_FEE_ON_CURRENCY0,
        (false, true) => PROTOCOL_FEE_ON_CURRENCY1,
        _ => PROTOCOL_FEE_ON_INPUT,
    }
}

/// The packed rate `initialize_pool` copies into a new
/// pool. `recorded0` / `recorded1` are the rates each side's `MintExclusivity` recorded at
/// registration (`None` where the side has no entry, or an entry that recorded nothing); with none
/// recorded the pool takes `current`, the config's setting now — every pool that is not a launch's.
/// Both recorded (two base mints exclusive to the one hook — never a FLOOR pool): each direction
/// takes the LOWER of the two, so no pool charges a rate either of its mints was not registered under
/// and neither registration's terms are exceeded.
pub const fn rate_for_new_pool(current: u32, recorded0: Option<u32>, recorded1: Option<u32>) -> u32 {
    match (recorded0, recorded1) {
        (None, None) => current,
        (Some(r), None) | (None, Some(r)) => r,
        (Some(a), Some(b)) => {
            let (a0, b0) = (directional(a, true), directional(b, true));
            let (a1, b1) = (directional(a, false), directional(b, false));
            pack(if a0 < b0 { a0 } else { b0 }, if a1 < b1 { a1 } else { b1 })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_rate_wins_over_the_current_setting() {
        let now = pack(1_000, 1_000);
        assert_eq!(rate_for_new_pool(now, None, None), now, "no record: today's setting (a direct pool)");
        assert_eq!(rate_for_new_pool(now, None, Some(pack(500, 500))), pack(500, 500), "a launch's mint: its creation's rate");
        assert_eq!(rate_for_new_pool(now, Some(0), None), 0, "a recorded 0 is a rate, not an absence");
        assert_eq!(rate_for_new_pool(0, None, Some(pack(700, 300))), pack(700, 300), "a fall is not taken either");
        assert_eq!(rate_for_new_pool(now, Some(pack(200, 900)), Some(pack(600, 100))), pack(200, 100), "both: the lower per direction");
    }
    use crate::constants::{PROTOCOL_FEE_DEFAULT, PROTOCOL_FEE_DEFAULT_PACKED, PROTOCOL_FEE_ON_CURRENCY0, PROTOCOL_FEE_ON_CURRENCY1};

    #[test]
    fn the_default_is_500_both_ways_and_within_v4s_max() {
        assert_eq!(PROTOCOL_FEE_DEFAULT, 500);
        assert_eq!(MAX_PROTOCOL_FEE, 1_000, "v4 ProtocolFeeLibrary.MAX_PROTOCOL_FEE");
        assert_eq!(PROTOCOL_FEE_DEFAULT_PACKED, 2_048_500);
        assert_eq!(pack(500, 500), PROTOCOL_FEE_DEFAULT_PACKED);
        assert_eq!(directional(PROTOCOL_FEE_DEFAULT_PACKED, true), 500);
        assert_eq!(directional(PROTOCOL_FEE_DEFAULT_PACKED, false), 500);
        assert!(is_valid(PROTOCOL_FEE_DEFAULT_PACKED));
        // FLOOR's 0.25 % LP fee: 2,999, not 3,000 (the protocol's share comes off first)
        assert_eq!(compose_swap_fee(500, 2_500), 2_999);
        assert_eq!(compose_swap_fee(500, 0), 500, "no LP fee: the composed rate IS the protocol's");
        assert_eq!(compose_swap_fee(0, 2_500), 2_500);
    }

    #[test]
    fn the_cap_is_inclusive_and_nothing_above_the_nibbles_is_valid() {
        assert!(is_valid(pack(1_000, 1_000)));
        assert!(is_valid(pack(0, 0)), "0 is a setting");
        assert!(is_valid(pack(1_000, 0)));
        assert!(!is_valid(pack(1_001, 0)));
        assert!(!is_valid(pack(0, 1_001)));
        assert!(!is_valid(pack(4_095, 4_095)));
        assert!(!is_valid(1 << 24), "a bit above the two nibbles");
    }

    #[test]
    fn v4s_nibble_order() {
        let packed = pack(1_000, 2_000);
        assert_eq!(packed & 0xfff, 1_000, "getZeroForOneFee = self & 0xfff");
        assert_eq!((packed >> 12) & 0xfff, 2_000, "getOneForZeroFee = self >> 12");
        assert_eq!(directional(packed, true), 1_000);
        assert_eq!(directional(packed, false), 2_000);
    }

    #[test]
    fn the_quote_side_rule_and_the_input_fallback() {
        // a FLOOR pool: quote = currency 0
        assert_eq!(currency_for_roles(true, false, false, false), PROTOCOL_FEE_ON_CURRENCY0);
        assert_eq!(fee_currency(PROTOCOL_FEE_ON_CURRENCY0, true), 0, "a buy pays in the quote it puts in");
        assert_eq!(fee_currency(PROTOCOL_FEE_ON_CURRENCY0, false), 0, "a sell pays in the quote it receives");
        assert!(!on_output(PROTOCOL_FEE_ON_CURRENCY0, true));
        assert!(on_output(PROTOCOL_FEE_ON_CURRENCY0, false));
        // quote = currency 1
        assert_eq!(currency_for_roles(false, true, false, false), PROTOCOL_FEE_ON_CURRENCY1);
        assert_eq!(fee_currency(PROTOCOL_FEE_ON_CURRENCY1, true), 1);
        assert_eq!(fee_currency(PROTOCOL_FEE_ON_CURRENCY1, false), 1);
        assert!(on_output(PROTOCOL_FEE_ON_CURRENCY1, true));
        assert!(!on_output(PROTOCOL_FEE_ON_CURRENCY1, false));
        // two plain tokens, or two NON-SOL quotes: the input coin (v4)
        for code in [currency_for_roles(false, false, false, false), currency_for_roles(true, true, false, false), PROTOCOL_FEE_ON_INPUT, 7] {
            assert_eq!(fee_currency(code, true), 0);
            assert_eq!(fee_currency(code, false), 1);
            assert!(!on_output(code, true) && !on_output(code, false));
        }
    }

    /// On a wSOL / listed-quote route pool (both sides listed) the fee side is
    /// the SOL leg — on the buy (SOL in) AND the sell (SOL out) — never the stock.
    #[test]
    fn a_pool_with_a_wsol_side_takes_the_fee_in_sol_even_when_both_sides_are_listed() {
        // wSOL is currency 0, the stock (listed too) currency 1
        let c = currency_for_roles(true, true, true, false);
        assert_eq!(c, PROTOCOL_FEE_ON_CURRENCY0);
        assert_eq!(fee_currency(c, true), 0, "a route buy (SOL in) pays in SOL, off the input");
        assert_eq!(fee_currency(c, false), 0, "a route sell (stock in) pays in SOL, off the output");
        assert!(!on_output(c, true) && on_output(c, false));
        // and mirrored: wSOL as currency 1
        let c = currency_for_roles(true, true, false, true);
        assert_eq!(c, PROTOCOL_FEE_ON_CURRENCY1);
        assert_eq!((fee_currency(c, true), fee_currency(c, false)), (1, 1));
        // a wSOL / base pool answers what the one-listed-side rule answers
        assert_eq!(currency_for_roles(true, false, true, false), currency_for_roles(true, false, false, false));
        assert_eq!(currency_for_roles(false, true, false, true), currency_for_roles(false, true, false, false));
        // all 16 inputs: a wSOL side always wins; otherwise one listed side; otherwise the input
        for bits in 0u8..16 {
            let (l0, l1, w0, w1) = (bits & 1 != 0, bits & 2 != 0, bits & 4 != 0, bits & 8 != 0);
            let want = if w0 != w1 {
                if w0 {
                    PROTOCOL_FEE_ON_CURRENCY0
                } else {
                    PROTOCOL_FEE_ON_CURRENCY1
                }
            } else if l0 != l1 {
                if l0 {
                    PROTOCOL_FEE_ON_CURRENCY0
                } else {
                    PROTOCOL_FEE_ON_CURRENCY1
                }
            } else {
                PROTOCOL_FEE_ON_INPUT
            };
            assert_eq!(currency_for_roles(l0, l1, w0, w1), want, "listed ({l0}, {l1}) wsol ({w0}, {w1})");
        }
    }

    #[test]
    fn walk_fee_composes_only_on_the_input() {
        let d = PROTOCOL_FEE_DEFAULT_PACKED;
        // FLOOR buy (quote in): composed
        let buy = walk_fee(d, PROTOCOL_FEE_ON_CURRENCY0, true, 2_500);
        assert_eq!(buy, WalkFee { swap_fee: 2_999, input_pips: 500, output_pips: 0, currency: 0 });
        // FLOOR sell (quote out): the steps run at the LP fee; 500 off the output
        let sell = walk_fee(d, PROTOCOL_FEE_ON_CURRENCY0, false, 2_500);
        assert_eq!(sell, WalkFee { swap_fee: 2_500, input_pips: 0, output_pips: 500, currency: 0 });
        // a pool at 0: nothing, on either side
        let old = walk_fee(0, PROTOCOL_FEE_ON_INPUT, false, 3_000);
        assert_eq!(old, WalkFee { swap_fee: 3_000, input_pips: 0, output_pips: 0, currency: 1 });
        // a plain pool, input rule, both ways
        assert_eq!(walk_fee(d, PROTOCOL_FEE_ON_INPUT, false, 3_000).input_pips, 500);
        assert_eq!(walk_fee(d, PROTOCOL_FEE_ON_INPUT, false, 3_000).currency, 1);
    }

    #[test]
    fn input_share_is_v4s_split() {
        // the whole fee when LP is 0
        assert_eq!(input_share(37, 1_000_000, 500, 500), Some(37));
        // floor(gross · p / 1e6) otherwise
        assert_eq!(input_share(2_999, 1_000_000, 2_999, 500), Some(500));
        assert_eq!(input_share(5, 1_999, 2_999, 500), Some(0), "under 2,000 units in, the share floors to 0");
        // no protocol: nothing
        assert_eq!(input_share(2_500, 1_000_000, 2_500, 0), Some(0));
        // an impossible composition is refused, never clamped
        assert_eq!(input_share(1, 1_000_000, 2_999, 500), None);
    }

    #[test]
    fn output_share_floors() {
        assert_eq!(output_share(1_000_000, 500), 500);
        assert_eq!(output_share(1_999, 500), 0);
        assert_eq!(output_share(2_000, 500), 1);
        assert_eq!(output_share(u64::MAX, 1_000), ((u64::MAX as u128) * 1_000 / 1_000_000) as u64);
        assert_eq!(output_share(123, 0), 0);
    }

    /// `gross_for_net` against a brute-force search, every `net` in a window and every `p` that can
    /// be stored (0..=0xfff): the least `G` with `G − floor(G·p/1e6) ≥ net`, and that `G` nets EXACTLY.
    #[test]
    fn gross_for_net_is_the_least_gross_and_nets_exactly() {
        let f = |g: u64, p: u32| g - output_share(g, p);
        for p in (0..=0xfffu32).step_by(7).chain([500, 1_000, 0xfff]) {
            for net in (0..3_000u64).chain([999_999, 1_000_000, 1_000_001, 123_456_789]) {
                let g = gross_for_net(net, p).expect("fits");
                assert_eq!(f(g, p), net, "p={p} net={net}: nets exactly");
                if g > 0 {
                    assert!(f(g - 1, p) < net, "p={p} net={net}: the least such gross");
                }
            }
        }
        // near u64::MAX the gross-up no longer fits
        assert_eq!(gross_for_net(u64::MAX, 500), None);
        assert_eq!(gross_for_net(u64::MAX, 0), Some(u64::MAX));
    }

}
