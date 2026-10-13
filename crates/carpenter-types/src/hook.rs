//! The hook interface (frozen wire format, version 1).
//!
//! Instruction data to a hook is `discriminator ‖ borsh(Args)` (discriminators in
//! [`crate::disc::callback`]); the hook's decision comes back as `borsh(HookRecordV1)` in
//! return data (≤ 1,024 B). Hook-initiated AMM instructions return `borsh(HookOpResultV1)`.
//!
//! Three parts:
//! 1. The interface types, byte-for-byte. Field order IS the wire format; never reorder.
//! 2. PROVISIONAL Args for the reserved v4 phases (the frozen interface defines none). FLOOR sets
//!    none of those flags.
//! 3. The pure record rules — strict decode and echo, the fee-override rule, the delta and fee
//!    field permissions, the action rules, no deltas in `after_actions`, and the round rules —
//!    and the two pieces of `Hooks.sol` arithmetic a hook
//!    author must be able to reproduce (the specified-side flip check, `Hooks.sol:273-278`, and
//!    the hook-delta assembly, `Hooks.sol:305-313`). The AMM maps [`RecordError`] to its own
//!    error codes; nothing here knows about accounts, CPIs or return-data program ids.

use borsh::{BorshDeserialize, BorshSerialize};

use crate::constants::{
    AFTER_SWAP_RETURNS_DELTA_FLAG, BEFORE_SWAP_FLAG, BEFORE_SWAP_RETURNS_DELTA_FLAG, MAX_LP_FEE,
    REQUESTS_ACTIONS_FLAG, SETTLES_FLAG,
};

// =========================================================================================
// Interface constants
// =========================================================================================

pub const HOOK_INTERFACE_VERSION: u8 = 1;
pub const MAX_ACTIONS_PER_RECORD: usize = 8;
pub const MAX_ROUNDS_CAP: u8 = 4;
pub const MAX_HOOK_DATA: usize = 64;
/// Solana return data is at most 1,024 bytes.
pub const MAX_RETURN_DATA: usize = 1_024;

// phase: one value per callback. round: before_swap 0, after_swap 1, the k-th after_actions
// 1 + k; 0 for every other phase.
pub const PHASE_BEFORE_INITIALIZE: u8 = 0;
pub const PHASE_AFTER_INITIALIZE: u8 = 1;
pub const PHASE_BEFORE_SWAP: u8 = 2;
pub const PHASE_AFTER_SWAP: u8 = 3;
pub const PHASE_AFTER_ACTIONS: u8 = 4;
pub const PHASE_BEFORE_ADD_LIQUIDITY: u8 = 5;
pub const PHASE_AFTER_ADD_LIQUIDITY: u8 = 6;
pub const PHASE_BEFORE_REMOVE_LIQUIDITY: u8 = 7;
pub const PHASE_AFTER_REMOVE_LIQUIDITY: u8 = 8;
pub const PHASE_BEFORE_DONATE: u8 = 9;
pub const PHASE_AFTER_DONATE: u8 = 10;
/// One past the last phase.
pub const PHASE_COUNT: u8 = 11;

// `HookRecordV1.fees` indices. The LP fee is not in the split: it is the AMM's own figure.
/// The skim, GROSS — what the trader paid, before the platform's cut is carved out of it.
pub const FEE_SKIM: usize = 0;
/// The creator fee.
pub const FEE_CREATOR: usize = 1;
/// The holder tax: up to 10 % of the quote leg a side (`floor_types::constants::MAX_HOLDER_BPS`).
pub const FEE_HOLDER: usize = 2;
/// The platform's cut, already CONTAINED in `FEE_SKIM`: never added to the
/// other three, and never more than the skim.
pub const FEE_PLATFORM: usize = 3;
/// The platform fee: charged ON TOP (in `fees_charged`), never inside the skim.
pub const FEE_TEAM: usize = 4;
/// `[skim, creator, holder, platform, team]`.
pub type FeeSplit = [u64; 5]; // [skim, creator, holder, platform, team]

/// The callback discriminator for a phase. `None` for an unknown phase.
pub const fn callback_discriminator(phase: u8) -> Option<[u8; 8]> {
    use crate::disc::callback as c;
    Some(match phase {
        PHASE_BEFORE_INITIALIZE => c::BEFORE_INITIALIZE,
        PHASE_AFTER_INITIALIZE => c::AFTER_INITIALIZE,
        PHASE_BEFORE_SWAP => c::BEFORE_SWAP,
        PHASE_AFTER_SWAP => c::AFTER_SWAP,
        PHASE_AFTER_ACTIONS => c::AFTER_ACTIONS,
        PHASE_BEFORE_ADD_LIQUIDITY => c::BEFORE_ADD_LIQUIDITY,
        PHASE_AFTER_ADD_LIQUIDITY => c::AFTER_ADD_LIQUIDITY,
        PHASE_BEFORE_REMOVE_LIQUIDITY => c::BEFORE_REMOVE_LIQUIDITY,
        PHASE_AFTER_REMOVE_LIQUIDITY => c::AFTER_REMOVE_LIQUIDITY,
        PHASE_BEFORE_DONATE => c::BEFORE_DONATE,
        PHASE_AFTER_DONATE => c::AFTER_DONATE,
        _ => return None,
    })
}

/// The fixed round index of a callback. `after_actions` rounds start at 2 and are the
/// caller's to count, so this returns the FIRST one (2) for that phase.
pub const fn first_round_of(phase: u8) -> u8 {
    match phase {
        PHASE_AFTER_SWAP => 1,
        PHASE_AFTER_ACTIONS => 2,
        _ => 0,
    }
}

// =========================================================================================
// Interface types
// =========================================================================================

/// 40 B.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PoolSnapshot {
    pub sqrt_price_x64: u128,
    pub tick: i32,
    pub liquidity: u128,
    pub lp_fee_pips: u32,
}

/// Caller perspective; negative = pays the pool (v4). In `ActionResult` and
/// `HookOpResultV1`, the hook-claims perspective: positive = credited.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BalanceDelta {
    pub amount0: i128,
    pub amount1: i128,
}

/// v4 sign: `amount_specified < 0` is exact-in.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SwapParams {
    pub zero_for_one: bool,
    pub amount_specified: i128,
    pub sqrt_price_limit_x64: u128,
}

/// The first three bytes of EVERY Args struct, reserved phases included. A hook that only
/// needs to echo (the pass-through mock) decodes this prefix and nothing else.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ArgsHeader {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
}

impl ArgsHeader {
    /// Read the prefix of any Args bytes (the 8-byte discriminator already stripped).
    pub fn peek(args: &[u8]) -> Option<Self> {
        match args {
            [version, phase, round, ..] => Some(Self { version: *version, phase: *phase, round: *round }),
            _ => None,
        }
    }
}

/// 153 B.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct BeforeInitializeArgs {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    pub pool: [u8; 32],
    pub sender: [u8; 32],
    pub mint0: [u8; 32],
    pub mint1: [u8; 32],
    pub fee: u32,
    pub tick_spacing: u16,
    pub sqrt_price_x64: u128,
}

/// 176 B + `hook_data` (≤ 64).
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct BeforeSwapArgs {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    pub pool: [u8; 32],
    pub sender: [u8; 32],
    pub params: SwapParams,
    pub pre: PoolSnapshot,
    pub supply0: u64,
    pub supply1: u64,
    /// Pre-swap.
    pub hook_claims0: u64,
    /// Pre-swap.
    pub hook_claims1: u64,
    pub hook_data: Vec<u8>,
}

/// 280 B + `hook_data` (≤ 64).
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AfterSwapArgs {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    pub pool: [u8; 32],
    pub sender: [u8; 32],
    pub params: SwapParams,
    pub pre: PoolSnapshot,
    pub post: PoolSnapshot,
    pub swap_delta: BalanceDelta,
    pub before_delta_specified: i128,
    pub before_delta_unspecified: i128,
    pub supply0: u64,
    pub supply1: u64,
    /// Pre-swap: no hook delta credited yet.
    pub hook_claims0: u64,
    /// Pre-swap: no hook delta credited yet.
    pub hook_claims1: u64,
    pub hook_data: Vec<u8>,
}

/// 111 B + 70 B per result.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AfterActionsArgs {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    pub pool: [u8; 32],
    pub post: PoolSnapshot,
    pub supply0: u64,
    pub supply1: u64,
    /// Post-action.
    pub hook_claims0: u64,
    /// Post-action.
    pub hook_claims1: u64,
    pub results: Vec<ActionResult>,
}

/// 70 B.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ActionResult {
    pub index: u8,
    /// The executed [`HookAction`]'s Borsh tag (0 `ModifyPosition`, 1 `Swap`).
    pub kind: u8,
    /// Hook-claims perspective: + credited.
    pub delta: BalanceDelta,
    pub fees0: u64,
    pub fees1: u64,
    pub post_sqrt_price_x64: u128,
    pub post_tick: i32,
}

/// The decision record. Fixed part 86 B with a fee override, 82 B without; worst case 358 B
///
/// `fees` sits in the fixed part at offset 38 (42 with an override), before the actions, so a
/// decoder that wants only the split never walks the `Vec`. The strict decode refuses a record of
/// any other length, so a hook and the AMM must agree on the five-slot split.
///
/// FLOOR's largest real record: the in-swap bid re-place's `after_swap`, `[ModifyPosition BID -L
/// (33 B), Swap nudge (34 B)]` = 82 + 67 = 149 B. A record of one `Swap` action alone is 116 B.
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct HookRecordV1 {
    pub version: u8,
    /// Echo of `Args.phase`, a `PHASE_*` value.
    pub phase: u8,
    /// Echo of `Args.round`.
    pub round: u8,
    /// `after_swap` / `after_actions` only.
    pub want_callback: bool,
    /// `before_swap` only; ≤ 1,000,000.
    pub fee_override_pips: Option<u32>,
    /// `before_swap` only (`BEFORE_SWAP_RETURNS_DELTA`).
    pub delta_specified: i128,
    /// `before_swap` / `after_swap` (matching `*_RETURNS_DELTA`); 0 in `after_actions`.
    pub delta_unspecified: i128,
    /// bit0 ask exhausted, bit1 ladder exhausted.
    pub signals: u8,
    /// `[skim, creator, holder, platform, team]` ([`FEE_SKIM`] … [`FEE_TEAM`]): the breakdown of the
    /// delta this record takes, in that delta's units — `delta_specified` in `before_swap`,
    /// `delta_unspecified` in `after_swap`, nothing anywhere else.
    /// `skim + creator + holder + team ≤ |delta|`; `platform ≤ skim` (it is the cut carved out of
    /// the skim, not a charge of its own). `team` is the platform fee, a CHARGE on top.
    /// All zero when the hook took nothing.
    pub fees: FeeSplit,
    /// ≤ 8; `after_swap` / `after_actions` only; `REQUESTS_ACTIONS`.
    pub actions: Vec<HookAction>,
    /// `before_swap` only, `SETTLES` only: a price the trader's walk may not pass —
    /// the walk runs to the TIGHTER of this and the trader's own limit; 0 = none. ⛔ NOT part of the
    /// Borsh body: it is an OPTIONAL 16-byte little-endian TAIL after `actions`, written by
    /// [`encode_record`] only when non-zero and read by [`decode_record`], so every record a hook
    /// without `SETTLES` returns is byte-identical to the record before the field existed, and the
    /// strict decode still refuses any other trailing length.
    #[borsh(skip)]
    pub sqrt_price_limit_x64: u128,
}

impl HookRecordV1 {
    /// The record a hook returns when it has nothing to say: echo, no fields.
    pub fn pass_through(version: u8, phase: u8, round: u8) -> Self {
        Self { version, phase, round, ..Self::default() }
    }

    /// The fee split, `[skim, creator, holder, platform, team]`.
    pub fn fees(&self) -> FeeSplit {
        self.fees
    }

    /// skim + creator + holder + team (the platform cut is inside the skim; the team fee on top).
    /// What the trader was charged by this record. `None` on overflow — a record no honest hook can
    /// produce.
    pub fn fees_charged(&self) -> Option<u64> {
        self.fees[FEE_SKIM].checked_add(self.fees[FEE_CREATOR])?.checked_add(self.fees[FEE_HOLDER])?.checked_add(self.fees[FEE_TEAM])
    }

    /// The delta `fees` breaks down: `delta_specified` from `before_swap`, `delta_unspecified` from
    /// `after_swap`, and `None` from every other phase (which may carry no fees).
    pub fn fee_delta(&self) -> Option<i128> {
        match self.phase {
            PHASE_BEFORE_SWAP => Some(self.delta_specified),
            PHASE_AFTER_SWAP => Some(self.delta_unspecified),
            _ => None,
        }
    }
}

/// An action the AMM executes on the hook's behalf.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookAction {
    /// Tag 0, 33 B.
    ModifyPosition { salt: u64, tick_lower: i32, tick_upper: i32, liquidity_delta: i128 },
    /// Tag 1, 34 B.
    Swap { zero_for_one: bool, amount_specified: i128, sqrt_price_limit_x64: u128 },
}

impl HookAction {
    pub const KIND_MODIFY_POSITION: u8 = 0;
    pub const KIND_SWAP: u8 = 1;

    /// The Borsh tag, reported back as `ActionResult.kind`.
    pub const fn kind(&self) -> u8 {
        match self {
            HookAction::ModifyPosition { .. } => Self::KIND_MODIFY_POSITION,
            HookAction::Swap { .. } => Self::KIND_SWAP,
        }
    }
}

/// What every hook-initiated AMM instruction that takes a pool returns. 105 B.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HookOpResultV1 {
    pub version: u8,
    /// Hook-claims perspective.
    pub delta: BalanceDelta,
    pub fees0: u64,
    pub fees1: u64,
    pub post: PoolSnapshot,
    pub hook_claims0: u64,
    pub hook_claims1: u64,
}

// ---- sizes -----------------------------------------------------------------------------

pub const POOL_SNAPSHOT_LEN: usize = 40;
pub const BALANCE_DELTA_LEN: usize = 32;
pub const SWAP_PARAMS_LEN: usize = 33;
pub const BEFORE_INITIALIZE_ARGS_LEN: usize = 153;
/// Without `hook_data` bytes (its 4-byte length prefix included).
pub const BEFORE_SWAP_ARGS_BASE_LEN: usize = 176;
/// Without `hook_data` bytes (its 4-byte length prefix included).
pub const AFTER_SWAP_ARGS_BASE_LEN: usize = 280;
/// Without results (the 4-byte length prefix included).
pub const AFTER_ACTIONS_ARGS_BASE_LEN: usize = 111;
pub const ACTION_RESULT_LEN: usize = 70;
/// `fees`: five `u64`s.
pub const FEE_SPLIT_LEN: usize = 40;
/// Byte offset of `fees` in a record with no fee override (`None` is one byte); 42 with one.
pub const RECORD_FEES_OFFSET_NO_OVERRIDE: usize = 38;
/// Byte offset of `fees` in a record with a fee override (`Some` is five bytes).
pub const RECORD_FEES_OFFSET_WITH_OVERRIDE: usize = 42;
/// The fixed fields (38, `None` override) + `fees` (40) + the actions' 4-byte length prefix.
pub const RECORD_BASE_LEN: usize = 82;
/// The same with a `Some` override (+4).
pub const RECORD_BASE_LEN_WITH_OVERRIDE: usize = 86;
pub const MODIFY_POSITION_ACTION_LEN: usize = 33;
pub const SWAP_ACTION_LEN: usize = 34;
/// 86 + 8 × 34 = 358. The cap that matters is [`MAX_RETURN_DATA`].
pub const RECORD_MAX_LEN: usize = RECORD_BASE_LEN_WITH_OVERRIDE + MAX_ACTIONS_PER_RECORD * SWAP_ACTION_LEN;
pub const HOOK_OP_RESULT_LEN: usize = 105;

// =========================================================================================
// PROVISIONAL: Args for the reserved v4 phases. Not part of the frozen interface.
// Each starts with version/phase/round like every frozen Args. FLOOR sets none of these flags;
// The AMM's `call_simple` is their first caller and may reshape them.
// =========================================================================================

/// PROVISIONAL. `after_initialize` (phase 1).
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AfterInitializeArgs {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    pub pool: [u8; 32],
    pub sender: [u8; 32],
    pub mint0: [u8; 32],
    pub mint1: [u8; 32],
    pub fee: u32,
    pub tick_spacing: u16,
    pub sqrt_price_x64: u128,
    pub tick: i32,
}

/// PROVISIONAL. v4 `ModifyLiquidityParams`, with the position salt.
#[derive(BorshSerialize, BorshDeserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ModifyLiquidityParams {
    pub salt: u64,
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub liquidity_delta: i128,
}

/// PROVISIONAL. `before_add_liquidity` (5) / `before_remove_liquidity` (7).
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct BeforeModifyLiquidityArgs {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    pub pool: [u8; 32],
    pub sender: [u8; 32],
    pub params: ModifyLiquidityParams,
    pub pre: PoolSnapshot,
    pub hook_data: Vec<u8>,
}

/// PROVISIONAL. `after_add_liquidity` (6) / `after_remove_liquidity` (8).
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct AfterModifyLiquidityArgs {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    pub pool: [u8; 32],
    pub sender: [u8; 32],
    pub params: ModifyLiquidityParams,
    pub post: PoolSnapshot,
    /// Caller perspective (v4).
    pub delta: BalanceDelta,
    pub fees_accrued: BalanceDelta,
    pub hook_data: Vec<u8>,
}

/// PROVISIONAL. `before_donate` (9) / `after_donate` (10).
#[derive(BorshSerialize, BorshDeserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct DonateArgs {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    pub pool: [u8; 32],
    pub sender: [u8; 32],
    pub amount0: u64,
    pub amount1: u64,
    pub hook_data: Vec<u8>,
}

// =========================================================================================
// Record rules (pure)
// =========================================================================================

/// Why a record was refused. The AMM maps each to its error code of the same name
/// (`HookBadReturn`, `HookFieldNotAllowed`, `HookRoundsExceeded`, `HookDeltaExceedsSwapAmount`,
/// `FeeInvalid`, `HookFeesInconsistent`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecordError {
    /// Undecodable, trailing bytes, too long, or a version/phase/round mismatch.
    BadReturn,
    /// A field set from a phase or flag set that may not set it, or more than 8 actions.
    FieldNotAllowed,
    /// A fee override above `MAX_LP_FEE`.
    FeeTooLarge,
    /// A record that asks for a callback round beyond the pool's `max_rounds`.
    RoundsExceeded,
    /// The specified delta flips the swap's direction (`Hooks.sol:273-278`).
    DeltaExceedsSwapAmount,
    /// Checked arithmetic overflowed (i128 deltas; the u64 fee sum).
    Overflow,
    /// `fees` claims more than the delta it explains
    /// (`skim + creator + holder > |delta|`), or a platform cut larger than the skim.
    FeesInconsistent,
}

/// What the AMM sent, and what the pool allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecordExpect {
    pub version: u8,
    pub phase: u8,
    pub round: u8,
    /// `pool.hook_flags`.
    pub flags: u16,
    /// `pool.hook_max_rounds`.
    pub max_rounds: u8,
}

/// Strict Borsh decode. Refuses an empty buffer, anything over 1,024 B, trailing bytes,
/// an invalid bool or enum tag, and more than 8 actions — all as `BadReturn`, except the action
/// count, which is `FieldNotAllowed`.
pub fn decode_record(bytes: &[u8]) -> Result<HookRecordV1, RecordError> {
    if bytes.is_empty() || bytes.len() > MAX_RETURN_DATA {
        return Err(RecordError::BadReturn);
    }
    // The optional price-limit tail: exactly 0 or 16 bytes may follow the body, and a written tail is
    // never 0 (`encode_record` omits a zero), so one record has one encoding. The body ends in a
    // length-prefixed Vec, so a strict decode of the whole and of all-but-16 cannot both succeed. The
    // strict decode lives in its own non-inlined frame (`decode_body`) to keep this function's SBF
    // stack frame small.
    let rec = match decode_body(bytes) {
        Some(rec) => rec,
        None => {
            let split = bytes.len().checked_sub(RECORD_PRICE_LIMIT_TAIL_LEN).ok_or(RecordError::BadReturn)?;
            let mut rec = decode_body(&bytes[..split]).ok_or(RecordError::BadReturn)?;
            let mut tail = [0u8; RECORD_PRICE_LIMIT_TAIL_LEN];
            tail.copy_from_slice(&bytes[split..]);
            let limit = u128::from_le_bytes(tail);
            if limit == 0 {
                return Err(RecordError::BadReturn);
            }
            rec.sqrt_price_limit_x64 = limit;
            rec
        }
    };
    if rec.actions.len() > MAX_ACTIONS_PER_RECORD {
        return Err(RecordError::FieldNotAllowed);
    }
    Ok(rec)
}

/// The length of [`HookRecordV1::sqrt_price_limit_x64`]'s optional tail.
pub const RECORD_PRICE_LIMIT_TAIL_LEN: usize = 16;

/// The strict Borsh decode of a record body (every byte consumed), or `None`.
#[inline(never)]
fn decode_body(bytes: &[u8]) -> Option<HookRecordV1> {
    borsh::from_slice::<HookRecordV1>(bytes).ok()
}

/// The bytes a hook returns: the Borsh body, then the price-limit tail iff it is non-zero. A hook
/// that never sets the limit may keep `borsh::to_vec`, which writes the same bytes.
pub fn encode_record(rec: &HookRecordV1) -> Vec<u8> {
    let mut out = borsh::to_vec(rec).unwrap_or_default();
    if rec.sqrt_price_limit_x64 != 0 {
        out.extend_from_slice(&rec.sqrt_price_limit_x64.to_le_bytes());
    }
    out
}

/// Strict Borsh decode of a [`HookOpResultV1`] (the hook side).
pub fn decode_op_result(bytes: &[u8]) -> Result<HookOpResultV1, RecordError> {
    let r = borsh::from_slice::<HookOpResultV1>(bytes).map_err(|_| RecordError::BadReturn)?;
    if r.version != HOOK_INTERFACE_VERSION {
        return Err(RecordError::BadReturn);
    }
    Ok(r)
}

/// The record rules for one decoded record: echo identity, the fee override, the delta and fee
/// field permissions, the actions (flag, phase, count), no deltas in `after_actions`, and the
/// round budget. Program-id and return-data checks are the AMM's; the flip checks need amounts and are
/// [`apply_specified_delta`] / [`HookDeltaAssembly`].
pub fn validate_record(rec: &HookRecordV1, expect: &RecordExpect) -> Result<(), RecordError> {
    // echo identity
    if rec.version != expect.version || rec.phase != expect.phase || rec.round != expect.round {
        return Err(RecordError::BadReturn);
    }
    let phase = rec.phase;
    let callback_phase = phase == PHASE_AFTER_SWAP || phase == PHASE_AFTER_ACTIONS;
    if (rec.want_callback || !rec.actions.is_empty()) && !callback_phase {
        return Err(RecordError::FieldNotAllowed);
    }
    // fee override
    if let Some(fee) = rec.fee_override_pips {
        if phase != PHASE_BEFORE_SWAP {
            return Err(RecordError::FieldNotAllowed);
        }
        if fee > MAX_LP_FEE {
            return Err(RecordError::FeeTooLarge);
        }
    }
    // deltas (after_actions may carry none)
    let f = expect.flags;
    let settles = f & SETTLES_FLAG != 0;
    // `SETTLES`: an `after_swap` may also take the specified side — the unfilled remainder of an
    // exact-in, which the hook fills along its own curve. Its bound needs the swap's amounts and is
    // the AMM's (the program's `settle_specified`; `carpenter_quote` replays it).
    let after_specified_ok = phase == PHASE_AFTER_SWAP && settles && f & AFTER_SWAP_RETURNS_DELTA_FLAG != 0;
    if rec.delta_specified != 0
        && !(phase == PHASE_BEFORE_SWAP && f & BEFORE_SWAP_RETURNS_DELTA_FLAG != 0)
        && !after_specified_ok
    {
        return Err(RecordError::FieldNotAllowed);
    }
    // `SETTLES`: the walk's price limit comes from `before_swap` alone.
    if rec.sqrt_price_limit_x64 != 0 && !(phase == PHASE_BEFORE_SWAP && settles) {
        return Err(RecordError::FieldNotAllowed);
    }
    if rec.delta_unspecified != 0 {
        let ok = (phase == PHASE_BEFORE_SWAP && f & BEFORE_SWAP_RETURNS_DELTA_FLAG != 0)
            || (phase == PHASE_AFTER_SWAP && f & AFTER_SWAP_RETURNS_DELTA_FLAG != 0);
        if !ok {
            return Err(RecordError::FieldNotAllowed);
        }
    }
    // fees: the split explains ONE delta — `delta_specified` in `before_swap`,
    // `delta_unspecified` in `after_swap` — under the same flag that allows that delta. It may
    // not claim more than the delta, and the platform cut lives inside the skim. The team slot
    // is a charge, so it counts toward the delta.
    if rec.fees != [0; 5] {
        let delta = match phase {
            PHASE_BEFORE_SWAP if f & BEFORE_SWAP_RETURNS_DELTA_FLAG != 0 => rec.delta_specified,
            PHASE_AFTER_SWAP if f & AFTER_SWAP_RETURNS_DELTA_FLAG != 0 => rec.delta_unspecified,
            _ => return Err(RecordError::FieldNotAllowed),
        };
        let charged = rec.fees_charged().ok_or(RecordError::Overflow)?;
        if u128::from(charged) > delta.unsigned_abs() || rec.fees[FEE_PLATFORM] > rec.fees[FEE_SKIM] {
            return Err(RecordError::FeesInconsistent);
        }
    }
    // actions
    if !rec.actions.is_empty() {
        if f & REQUESTS_ACTIONS_FLAG == 0 || rec.actions.len() > MAX_ACTIONS_PER_RECORD {
            return Err(RecordError::FieldNotAllowed);
        }
    }
    // `want_callback` books an `after_actions` round, and that phase is gated by
    // `REQUESTS_ACTIONS` (`hook_dispatch::phase_flag`). Without the flag the next callback would
    // be unflagged: calling it would invoke a phase the hook did not register, and skipping it
    // would silently drop the requested round. Refuse it.
    if rec.want_callback && f & REQUESTS_ACTIONS_FLAG == 0 {
        return Err(RecordError::FieldNotAllowed);
    }
    // rounds: a record that wants another callback must leave room for it.
    if rec.want_callback && (rec.round as u16) + 1 >= expect.max_rounds as u16 {
        return Err(RecordError::RoundsExceeded);
    }
    Ok(())
}

/// The override the swap actually uses. Applied only when `BEFORE_SWAP` is set and the
/// pool's key fee is dynamic (v4 ignores an override on a static pool); validity was checked
/// by [`validate_record`].
pub fn effective_fee_override(rec: &HookRecordV1, flags: u16, pool_fee_is_dynamic: bool) -> Option<u32> {
    if rec.phase == PHASE_BEFORE_SWAP && flags & BEFORE_SWAP_FLAG != 0 && pool_fee_is_dynamic {
        rec.fee_override_pips
    } else {
        None
    }
}

/// `Hooks.sol:273-278`: `amount_to_swap = amount_specified + delta_specified`, refused if
/// it flips exact-in into exact-out (or back). Exactly 0 is allowed and skips the walk.
pub fn apply_specified_delta(amount_specified: i128, delta_specified: i128) -> Result<i128, RecordError> {
    let exact_in = amount_specified < 0;
    let to_swap = amount_specified.checked_add(delta_specified).ok_or(RecordError::Overflow)?;
    let flipped = if exact_in { to_swap > 0 } else { to_swap < 0 };
    if flipped {
        return Err(RecordError::DeltaExceedsSwapAmount);
    }
    Ok(to_swap)
}

/// `Hooks.sol:305-313`: the ONE hook delta, from the specified and unspecified parts
/// (`before_swap`'s `delta_specified`; `before_swap` + `after_swap`'s `delta_unspecified`).
pub struct HookDeltaAssembly;

impl HookDeltaAssembly {
    pub fn assemble(amount_specified: i128, zero_for_one: bool, specified: i128, unspecified: i128) -> BalanceDelta {
        if specified == 0 && unspecified == 0 {
            return BalanceDelta::default();
        }
        if (amount_specified < 0) == zero_for_one {
            BalanceDelta { amount0: specified, amount1: unspecified }
        } else {
            BalanceDelta { amount0: unspecified, amount1: specified }
        }
    }

    /// The fee split per currency, for `Swapped.fee_split0 / fee_split1`. Each swap
    /// record explains one delta (`fee_delta`), so `before_swap`'s split lands on the SPECIFIED
    /// currency and `after_swap`'s on the unspecified one — the same mapping `assemble` gives the
    /// deltas (`Hooks.sol:305-313`). Nothing is added, so nothing can overflow: the two records
    /// never share a currency. A missing record contributes zeros.
    pub fn assemble_fees(amount_specified: i128, zero_for_one: bool, before: FeeSplit, after: FeeSplit) -> (FeeSplit, FeeSplit) {
        if (amount_specified < 0) == zero_for_one {
            (before, after)
        } else {
            (after, before)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::*;

    // The FLOOR hook's own numbers, as FIXTURES: this crate is the AMM's and links no mechanism, so
    // `floor_types::constants::{FLOOR_HOOK_FLAGS, FLOOR_MAX_ROUNDS}` are spelled here as literals —
    // `0x60CC` is `0x20CC | REQUESTS_ACTIONS_FLAG`, three rounds — and `floor_types` pins the real
    // ones to these values (`floor_flags_are_0x60cc`). A record from ANY hook with returns-delta
    // and actions bits set would do; FLOOR's is the one that exists.
    const FLOOR_HOOK_FLAGS: u16 = 0x60CC;
    const FLOOR_MAX_ROUNDS: u8 = 3;
    // Likewise the round-trip fixtures below: `SIGNAL_*`, `SALT_LADDER`, `LP_FEE_PIPS` and `SUPPLY` are
    // FLOOR's values, spelled as the literals the mechanism defines them as. Any bytes would round-trip.
    const SIGNAL_ASK_EXHAUSTED: u8 = 1 << 0;
    const SIGNAL_LADDER_EXHAUSTED: u8 = 1 << 1;
    const SALT_LADDER: u64 = 100;
    const LP_FEE_PIPS: u32 = 2_500;
    const SUPPLY: u64 = 1_000_000_000_000_000;

    fn len<T: BorshSerialize>(v: &T) -> usize {
        borsh::to_vec(v).unwrap().len()
    }

    #[test]
    fn sizes_match_the_interface() {
        assert_eq!(len(&PoolSnapshot::default()), POOL_SNAPSHOT_LEN);
        assert_eq!(len(&BalanceDelta::default()), BALANCE_DELTA_LEN);
        assert_eq!(len(&SwapParams::default()), SWAP_PARAMS_LEN);
        assert_eq!(len(&ArgsHeader::default()), 3);
        assert_eq!(len(&BeforeInitializeArgs::default()), BEFORE_INITIALIZE_ARGS_LEN);
        assert_eq!(len(&BeforeSwapArgs::default()), BEFORE_SWAP_ARGS_BASE_LEN);
        assert_eq!(len(&BeforeSwapArgs { hook_data: vec![7; MAX_HOOK_DATA], ..Default::default() }), 176 + 64);
        assert_eq!(len(&AfterSwapArgs::default()), AFTER_SWAP_ARGS_BASE_LEN);
        assert_eq!(len(&AfterActionsArgs::default()), AFTER_ACTIONS_ARGS_BASE_LEN);
        assert_eq!(len(&ActionResult::default()), ACTION_RESULT_LEN);
        let eight = AfterActionsArgs { results: vec![ActionResult::default(); 8], ..Default::default() };
        assert_eq!(len(&eight), 671);
        assert_eq!(8 + len(&eight), 679);
        assert_eq!(len(&HookOpResultV1::default()), HOOK_OP_RESULT_LEN);
    }

    #[test]
    fn record_sizes_and_worst_case_fit_return_data() {
        let base = HookRecordV1::default();
        assert_eq!(len(&base), RECORD_BASE_LEN);
        assert_eq!(RECORD_BASE_LEN, 82, "42 + the 40-B five-slot fee split");
        let with_fee = HookRecordV1 { fee_override_pips: Some(2_500), ..Default::default() };
        assert_eq!(len(&with_fee), RECORD_BASE_LEN_WITH_OVERRIDE);
        assert_eq!(RECORD_BASE_LEN_WITH_OVERRIDE, 86);
        let mp = HookAction::ModifyPosition { salt: 1, tick_lower: -60, tick_upper: 60, liquidity_delta: -5 };
        let sw = HookAction::Swap { zero_for_one: true, amount_specified: -1, sqrt_price_limit_x64: 1 };
        assert_eq!(len(&mp), MODIFY_POSITION_ACTION_LEN);
        assert_eq!(len(&sw), SWAP_ACTION_LEN);
        let worst = HookRecordV1 { fee_override_pips: Some(MAX_LP_FEE), actions: vec![sw; 8], ..Default::default() };
        assert_eq!(len(&worst), RECORD_MAX_LEN);
        assert_eq!(RECORD_MAX_LEN, 358, "86 + 8 × 34");
        assert!(RECORD_MAX_LEN <= MAX_RETURN_DATA);
        // FLOOR's largest real record: after_swap, the re-place's [ModifyPosition, Swap], no
        // override = 82 + 33 + 34 = 149 B, the fee split included.
        let floor = HookRecordV1 { phase: PHASE_AFTER_SWAP, round: 1, want_callback: true, actions: vec![mp, sw], ..Default::default() };
        assert_eq!(len(&floor), 149);
        // A one-Swap record: 76 + the 40-B split.
        let one_swap = HookRecordV1 { phase: PHASE_AFTER_SWAP, round: 1, want_callback: true, actions: vec![sw], ..Default::default() };
        assert_eq!(len(&one_swap), 116);
        // The split is at a fixed offset, before the actions, whatever the action count.
        let fees = [11u64, 22, 33, 4, 5];
        let fee_bytes = borsh::to_vec(&fees).unwrap();
        assert_eq!(fee_bytes.len(), FEE_SPLIT_LEN);
        let bytes = borsh::to_vec(&HookRecordV1 { fees, actions: vec![sw; 3], ..Default::default() }).unwrap();
        let at = RECORD_FEES_OFFSET_NO_OVERRIDE;
        assert_eq!(&bytes[at..at + FEE_SPLIT_LEN], &fee_bytes[..]);
        assert_eq!(at + FEE_SPLIT_LEN + 4, RECORD_BASE_LEN, "fees, then the actions' length prefix");
        let bytes = borsh::to_vec(&HookRecordV1 { fees, fee_override_pips: Some(1), actions: vec![sw; 3], ..Default::default() }).unwrap();
        let at = RECORD_FEES_OFFSET_WITH_OVERRIDE;
        assert_eq!(&bytes[at..at + FEE_SPLIT_LEN], &fee_bytes[..]);
        assert_eq!(at + FEE_SPLIT_LEN + 4, RECORD_BASE_LEN_WITH_OVERRIDE);
    }

    #[test]
    fn borsh_round_trips_every_type() {
        let rec = HookRecordV1 {
            version: 1,
            phase: PHASE_AFTER_SWAP,
            round: 1,
            want_callback: true,
            fee_override_pips: None,
            delta_specified: 0,
            delta_unspecified: -12_345_678_901_234_567_890_123,
            signals: SIGNAL_ASK_EXHAUSTED | SIGNAL_LADDER_EXHAUSTED,
            fees: [u64::MAX, 1, 0, u64::MAX - 1, 7],
            actions: vec![
                HookAction::Swap { zero_for_one: true, amount_specified: -50_000_000, sqrt_price_limit_x64: MIN_SQRT_PRICE_X64 },
                HookAction::ModifyPosition { salt: SALT_LADDER, tick_lower: -20_760, tick_upper: -6_960, liquidity_delta: i128::MIN },
            ],
            sqrt_price_limit_x64: 0,
        };
        assert_eq!(decode_record(&borsh::to_vec(&rec).unwrap()).unwrap(), rec);

        let snap = PoolSnapshot { sqrt_price_x64: MAX_SQRT_PRICE_X64, tick: MIN_TICK, liquidity: u128::MAX, lp_fee_pips: LP_FEE_PIPS };
        let args = AfterSwapArgs {
            version: 1,
            phase: PHASE_AFTER_SWAP,
            round: 1,
            pool: [1; 32],
            sender: [2; 32],
            params: SwapParams { zero_for_one: false, amount_specified: -1, sqrt_price_limit_x64: 3 },
            pre: snap,
            post: PoolSnapshot { tick: MAX_TICK, ..snap },
            swap_delta: BalanceDelta { amount0: i128::MAX, amount1: i128::MIN },
            before_delta_specified: 4,
            before_delta_unspecified: -4,
            supply0: 5,
            supply1: SUPPLY,
            hook_claims0: 6,
            hook_claims1: 7,
            hook_data: vec![9; 64],
        };
        let bytes = borsh::to_vec(&args).unwrap();
        assert_eq!(borsh::from_slice::<AfterSwapArgs>(&bytes).unwrap(), args);
        assert_eq!(ArgsHeader::peek(&bytes), Some(ArgsHeader { version: 1, phase: PHASE_AFTER_SWAP, round: 1 }));

        let op = HookOpResultV1 { version: 1, delta: BalanceDelta { amount0: -1, amount1: 2 }, fees0: 3, fees1: 4, post: snap, hook_claims0: 5, hook_claims1: 6 };
        assert_eq!(decode_op_result(&borsh::to_vec(&op).unwrap()).unwrap(), op);

        let ba = BeforeInitializeArgs { version: 1, phase: 0, round: 0, pool: [3; 32], sender: [4; 32], mint0: [5; 32], mint1: [6; 32], fee: DYNAMIC_FEE_FLAG, tick_spacing: 60, sqrt_price_x64: 1 << 64 };
        assert_eq!(borsh::from_slice::<BeforeInitializeArgs>(&borsh::to_vec(&ba).unwrap()).unwrap(), ba);
    }

    #[test]
    fn every_args_struct_starts_with_version_phase_round() {
        let hdr = |b: Vec<u8>| ArgsHeader::peek(&b).unwrap();
        let want = ArgsHeader { version: 1, phase: 7, round: 9 };
        assert_eq!(hdr(borsh::to_vec(&BeforeInitializeArgs { version: 1, phase: 7, round: 9, ..Default::default() }).unwrap()), want);
        assert_eq!(hdr(borsh::to_vec(&BeforeSwapArgs { version: 1, phase: 7, round: 9, ..Default::default() }).unwrap()), want);
        assert_eq!(hdr(borsh::to_vec(&AfterSwapArgs { version: 1, phase: 7, round: 9, ..Default::default() }).unwrap()), want);
        assert_eq!(hdr(borsh::to_vec(&AfterActionsArgs { version: 1, phase: 7, round: 9, ..Default::default() }).unwrap()), want);
        assert_eq!(hdr(borsh::to_vec(&AfterInitializeArgs { version: 1, phase: 7, round: 9, ..Default::default() }).unwrap()), want);
        assert_eq!(hdr(borsh::to_vec(&BeforeModifyLiquidityArgs { version: 1, phase: 7, round: 9, ..Default::default() }).unwrap()), want);
        assert_eq!(hdr(borsh::to_vec(&AfterModifyLiquidityArgs { version: 1, phase: 7, round: 9, ..Default::default() }).unwrap()), want);
        assert_eq!(hdr(borsh::to_vec(&DonateArgs { version: 1, phase: 7, round: 9, ..Default::default() }).unwrap()), want);
        assert_eq!(ArgsHeader::peek(&[1, 2]), None);
    }

    #[test]
    fn decode_is_strict() {
        let ok = borsh::to_vec(&HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0)).unwrap();
        assert!(decode_record(&ok).is_ok());
        let mut trailing = ok.clone();
        trailing.push(0);
        assert_eq!(decode_record(&trailing), Err(RecordError::BadReturn));
        assert_eq!(decode_record(&ok[..ok.len() - 1]), Err(RecordError::BadReturn));
        assert_eq!(decode_record(&[]), Err(RecordError::BadReturn));
        let mut bad_bool = ok.clone();
        bad_bool[3] = 2; // want_callback
        assert_eq!(decode_record(&bad_bool), Err(RecordError::BadReturn));
        let mut bad_tag = borsh::to_vec(&HookRecordV1 {
            actions: vec![HookAction::Swap { zero_for_one: false, amount_specified: 0, sqrt_price_limit_x64: 0 }],
            ..Default::default()
        })
        .unwrap();
        let tag_at = RECORD_BASE_LEN;
        assert_eq!(bad_tag[tag_at], 1);
        bad_tag[tag_at] = 2;
        assert_eq!(decode_record(&bad_tag), Err(RecordError::BadReturn));
        let nine = HookRecordV1 {
            phase: PHASE_AFTER_SWAP,
            actions: vec![HookAction::Swap { zero_for_one: false, amount_specified: 0, sqrt_price_limit_x64: 0 }; 9],
            ..Default::default()
        };
        assert_eq!(decode_record(&borsh::to_vec(&nine).unwrap()), Err(RecordError::FieldNotAllowed));
        assert_eq!(decode_record(&vec![0u8; MAX_RETURN_DATA + 1]), Err(RecordError::BadReturn));
    }

    fn expect(phase: u8, round: u8) -> RecordExpect {
        RecordExpect { version: 1, phase, round, flags: FLOOR_HOOK_FLAGS, max_rounds: FLOOR_MAX_ROUNDS }
    }

    fn swap_action() -> HookAction {
        HookAction::Swap { zero_for_one: true, amount_specified: -1, sqrt_price_limit_x64: 1 }
    }

    #[test]
    fn o1_identity_must_echo() {
        let e = expect(PHASE_BEFORE_SWAP, 0);
        assert_eq!(validate_record(&HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0), &e), Ok(()));
        assert_eq!(validate_record(&HookRecordV1::pass_through(2, PHASE_BEFORE_SWAP, 0), &e), Err(RecordError::BadReturn));
        assert_eq!(validate_record(&HookRecordV1::pass_through(1, PHASE_AFTER_SWAP, 0), &e), Err(RecordError::BadReturn));
        assert_eq!(validate_record(&HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 1), &e), Err(RecordError::BadReturn));
    }

    #[test]
    fn o1_want_callback_and_actions_only_from_after_swap_or_after_actions() {
        for phase in 0..PHASE_COUNT {
            let round = first_round_of(phase);
            let e = expect(phase, round);
            let want = HookRecordV1 { want_callback: true, ..HookRecordV1::pass_through(1, phase, round) };
            let acts = HookRecordV1 { actions: vec![swap_action()], ..HookRecordV1::pass_through(1, phase, round) };
            if phase == PHASE_AFTER_SWAP || phase == PHASE_AFTER_ACTIONS {
                assert_eq!(validate_record(&acts, &e), Ok(()), "phase {phase}");
            } else {
                assert_eq!(validate_record(&want, &e), Err(RecordError::FieldNotAllowed), "phase {phase}");
                assert_eq!(validate_record(&acts, &e), Err(RecordError::FieldNotAllowed), "phase {phase}");
            }
        }
    }

    #[test]
    fn o3_fee_override_only_from_before_swap_and_capped() {
        let rec = |phase, round, fee| HookRecordV1 { fee_override_pips: Some(fee), ..HookRecordV1::pass_through(1, phase, round) };
        assert_eq!(validate_record(&rec(PHASE_BEFORE_SWAP, 0, MAX_LP_FEE), &expect(PHASE_BEFORE_SWAP, 0)), Ok(()));
        assert_eq!(validate_record(&rec(PHASE_BEFORE_SWAP, 0, MAX_LP_FEE + 1), &expect(PHASE_BEFORE_SWAP, 0)), Err(RecordError::FeeTooLarge));
        assert_eq!(validate_record(&rec(PHASE_AFTER_SWAP, 1, 2500), &expect(PHASE_AFTER_SWAP, 1)), Err(RecordError::FieldNotAllowed));
        let r = rec(PHASE_BEFORE_SWAP, 0, 2500);
        assert_eq!(effective_fee_override(&r, FLOOR_HOOK_FLAGS, true), Some(2500));
        assert_eq!(effective_fee_override(&r, FLOOR_HOOK_FLAGS, false), None, "static pool ignores it");
        assert_eq!(effective_fee_override(&r, FLOOR_HOOK_FLAGS & !BEFORE_SWAP_FLAG, true), None);
    }

    #[test]
    fn o4_deltas_follow_phase_and_flags() {
        let flags_no_delta = FLOOR_HOOK_FLAGS & !(BEFORE_SWAP_RETURNS_DELTA_FLAG | AFTER_SWAP_RETURNS_DELTA_FLAG);
        let with = |phase, round, s, u| HookRecordV1 { delta_specified: s, delta_unspecified: u, ..HookRecordV1::pass_through(1, phase, round) };
        // before_swap: both allowed with BEFORE_SWAP_RETURNS_DELTA
        assert_eq!(validate_record(&with(PHASE_BEFORE_SWAP, 0, 5, 6), &expect(PHASE_BEFORE_SWAP, 0)), Ok(()));
        let mut e = expect(PHASE_BEFORE_SWAP, 0);
        e.flags = flags_no_delta;
        assert_eq!(validate_record(&with(PHASE_BEFORE_SWAP, 0, 5, 0), &e), Err(RecordError::FieldNotAllowed));
        assert_eq!(validate_record(&with(PHASE_BEFORE_SWAP, 0, 0, 5), &e), Err(RecordError::FieldNotAllowed));
        // after_swap: unspecified only, with AFTER_SWAP_RETURNS_DELTA
        assert_eq!(validate_record(&with(PHASE_AFTER_SWAP, 1, 0, 5), &expect(PHASE_AFTER_SWAP, 1)), Ok(()));
        assert_eq!(validate_record(&with(PHASE_AFTER_SWAP, 1, 5, 0), &expect(PHASE_AFTER_SWAP, 1)), Err(RecordError::FieldNotAllowed));
        let mut e = expect(PHASE_AFTER_SWAP, 1);
        e.flags = flags_no_delta;
        assert_eq!(validate_record(&with(PHASE_AFTER_SWAP, 1, 0, 5), &e), Err(RecordError::FieldNotAllowed));
        // after_actions carries none
        assert_eq!(validate_record(&with(PHASE_AFTER_ACTIONS, 2, 1, 0), &expect(PHASE_AFTER_ACTIONS, 2)), Err(RecordError::FieldNotAllowed));
        assert_eq!(validate_record(&with(PHASE_AFTER_ACTIONS, 2, 0, 1), &expect(PHASE_AFTER_ACTIONS, 2)), Err(RecordError::FieldNotAllowed));
        // before_initialize carries none
        assert_eq!(validate_record(&with(PHASE_BEFORE_INITIALIZE, 0, 0, 1), &expect(PHASE_BEFORE_INITIALIZE, 0)), Err(RecordError::FieldNotAllowed));
    }

    #[test]
    fn o4_fees_explain_one_delta_under_its_flag() {
        // FLOOR's own shapes: a buy at before_swap (delta_specified), a sell at after_swap
        // (delta_unspecified); skim 200 of which the platform's cut is 5, creator 100, holder 0.
        let split = [200u64, 100, 0, 5, 0];
        let buy = HookRecordV1 { delta_specified: 300, fees: split, ..HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0) };
        assert_eq!(validate_record(&buy, &expect(PHASE_BEFORE_SWAP, 0)), Ok(()));
        assert_eq!(buy.fees(), split);
        assert_eq!(buy.fees_charged(), Some(300));
        assert_eq!(buy.fee_delta(), Some(300));
        let sell = HookRecordV1 { delta_unspecified: 300, fees: split, ..HookRecordV1::pass_through(1, PHASE_AFTER_SWAP, 1) };
        assert_eq!(validate_record(&sell, &expect(PHASE_AFTER_SWAP, 1)), Ok(()));
        assert_eq!(sell.fee_delta(), Some(300));
        // a split may be SMALLER than the delta (a hook that takes something it does not classify)
        let under = HookRecordV1 { delta_specified: 301, ..buy.clone() };
        assert_eq!(validate_record(&under, &expect(PHASE_BEFORE_SWAP, 0)), Ok(()));
        // …but never larger, and the platform cut never exceeds the skim
        let over = HookRecordV1 { delta_specified: 299, ..buy.clone() };
        assert_eq!(validate_record(&over, &expect(PHASE_BEFORE_SWAP, 0)), Err(RecordError::FeesInconsistent));
        let cut_too_big = HookRecordV1 { fees: [200, 100, 0, 201, 0], ..buy.clone() };
        assert_eq!(validate_record(&cut_too_big, &expect(PHASE_BEFORE_SWAP, 0)), Err(RecordError::FeesInconsistent));
        // the split explains ONE delta: before_swap's fees are measured against delta_specified,
        // not the unspecified one it may also carry, and after_swap's against delta_unspecified
        let wrong_delta = HookRecordV1 { delta_specified: 0, delta_unspecified: 300, fees: split, ..HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0) };
        assert_eq!(validate_record(&wrong_delta, &expect(PHASE_BEFORE_SWAP, 0)), Err(RecordError::FeesInconsistent));
        let wrong_after = HookRecordV1 { delta_specified: 300, delta_unspecified: 0, fees: split, ..HookRecordV1::pass_through(1, PHASE_AFTER_SWAP, 1) };
        assert_eq!(validate_record(&wrong_after, &expect(PHASE_AFTER_SWAP, 1)), Err(RecordError::FieldNotAllowed), "delta_specified from after_swap fails the delta permission first");
        // the same flag gate as the delta it explains
        let mut no_flag = expect(PHASE_BEFORE_SWAP, 0);
        no_flag.flags &= !BEFORE_SWAP_RETURNS_DELTA_FLAG;
        let fees_only = HookRecordV1 { fees: [1, 0, 0, 0, 0], ..HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0) };
        assert_eq!(validate_record(&fees_only, &no_flag), Err(RecordError::FieldNotAllowed));
        let mut no_after = expect(PHASE_AFTER_SWAP, 1);
        no_after.flags &= !AFTER_SWAP_RETURNS_DELTA_FLAG;
        let fees_after = HookRecordV1 { fees: [1, 0, 0, 0, 0], ..HookRecordV1::pass_through(1, PHASE_AFTER_SWAP, 1) };
        assert_eq!(validate_record(&fees_after, &no_after), Err(RecordError::FieldNotAllowed));
        // fees with a zero delta claim more than nothing
        assert_eq!(validate_record(&fees_only, &expect(PHASE_BEFORE_SWAP, 0)), Err(RecordError::FeesInconsistent));
        // every other phase: none at all, whatever the flags
        for phase in 0..PHASE_COUNT {
            if phase == PHASE_BEFORE_SWAP || phase == PHASE_AFTER_SWAP {
                continue;
            }
            let round = first_round_of(phase);
            let rec = HookRecordV1 { fees: [0, 0, 0, 1, 0], ..HookRecordV1::pass_through(1, phase, round) };
            let mut e = expect(phase, round);
            e.flags = ALL_HOOK_FLAGS;
            e.max_rounds = MAX_ROUNDS_CAP;
            assert_eq!(validate_record(&rec, &e), Err(RecordError::FieldNotAllowed), "phase {phase}");
            assert_eq!(rec.fee_delta(), None);
        }
        // an overflowing sum is refused as Overflow, not accepted by wrapping
        let huge = HookRecordV1 { delta_specified: i128::MAX, fees: [u64::MAX, 1, 0, 0, 0], ..HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0) };
        assert_eq!(huge.fees_charged(), None);
        assert_eq!(validate_record(&huge, &expect(PHASE_BEFORE_SWAP, 0)), Err(RecordError::Overflow));
        // a pass-through record carries no fees and is unaffected
        assert_eq!(HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0).fees(), [0; 5]);
    }

    /// The fifth slot is a CHARGE (in fees_charged), unlike FEE_PLATFORM (inside the skim).
    #[test]
    fn the_team_slot_is_the_fifth_and_it_is_charged() {
        assert_eq!(FEE_TEAM, 4);
        let buy = HookRecordV1 { delta_specified: 301, fees: [200, 100, 0, 5, 1], ..HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0) };
        assert_eq!(buy.fees_charged(), Some(301));
        assert_eq!(validate_record(&buy, &expect(PHASE_BEFORE_SWAP, 0)), Ok(()));
        let short = HookRecordV1 { delta_specified: 300, ..buy.clone() };
        assert_eq!(validate_record(&short, &expect(PHASE_BEFORE_SWAP, 0)), Err(RecordError::FeesInconsistent));
        let team_only = HookRecordV1 { delta_specified: 1, fees: [0, 0, 0, 0, 1], ..HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0) };
        assert_eq!(validate_record(&team_only, &expect(PHASE_BEFORE_SWAP, 0)), Ok(()));
    }

    #[test]
    fn fee_assembly_follows_the_delta_assembly() {
        let b = [1u64, 2, 3, 4, 5];
        let a = [6u64, 7, 8, 9, 10];
        // exact-in zero_for_one: specified is currency0, so before_swap's split is fees0
        assert_eq!(HookDeltaAssembly::assemble_fees(-10, true, b, a), (b, a));
        // exact-in one_for_zero: specified is currency1
        assert_eq!(HookDeltaAssembly::assemble_fees(-10, false, b, a), (a, b));
        // exact-out zero_for_one: specified is currency1
        assert_eq!(HookDeltaAssembly::assemble_fees(10, true, b, a), (a, b));
        assert_eq!(HookDeltaAssembly::assemble_fees(10, false, b, a), (b, a));
        // FLOOR: an exact-in buy (wSOL = currency0 in, zero_for_one) takes at before_swap only
        assert_eq!(HookDeltaAssembly::assemble_fees(-10, true, b, [0; 5]), (b, [0; 5]));
        // FLOOR: an exact-in sell (token in, one_for_zero) takes the quote leg at after_swap,
        // which is the unspecified currency = currency0
        assert_eq!(HookDeltaAssembly::assemble_fees(-10, false, [0; 5], a), (a, [0; 5]));
        // the currency each split lands on is the currency `assemble` gives its delta
        for (amt, zfo) in [(-10i128, true), (-10, false), (10, true), (10, false)] {
            let d = HookDeltaAssembly::assemble(amt, zfo, 300, 700);
            let (f0, f1) = HookDeltaAssembly::assemble_fees(amt, zfo, [300, 0, 0, 0, 0], [700, 0, 0, 0, 0]);
            assert_eq!((f0[FEE_SKIM] as i128, f1[FEE_SKIM] as i128), (d.amount0, d.amount1));
        }
    }

    #[test]
    fn o5_actions_need_requests_actions() {
        let rec = HookRecordV1 { actions: vec![swap_action()], ..HookRecordV1::pass_through(1, PHASE_AFTER_SWAP, 1) };
        let mut e = expect(PHASE_AFTER_SWAP, 1);
        assert_eq!(validate_record(&rec, &e), Ok(()));
        e.flags &= !REQUESTS_ACTIONS_FLAG;
        assert_eq!(validate_record(&rec, &e), Err(RecordError::FieldNotAllowed));
        let nine = HookRecordV1 { actions: vec![swap_action(); 9], ..HookRecordV1::pass_through(1, PHASE_AFTER_SWAP, 1) };
        assert_eq!(validate_record(&nine, &expect(PHASE_AFTER_SWAP, 1)), Err(RecordError::FieldNotAllowed));
    }

    #[test]
    fn o5_want_callback_needs_requests_actions() {
        // `after_actions` is gated by REQUESTS_ACTIONS (hook_dispatch::phase_flag), so a record
        // that books one on a pool without the flag is refused, from either phase that may ask.
        for (phase, round) in [(PHASE_AFTER_SWAP, 1), (PHASE_AFTER_ACTIONS, 2)] {
            let want = HookRecordV1 { want_callback: true, ..HookRecordV1::pass_through(1, phase, round) };
            let mut e = expect(phase, round);
            e.max_rounds = MAX_ROUNDS_CAP;
            assert_eq!(validate_record(&want, &e), Ok(()), "phase {phase} with the flag");
            e.flags &= !REQUESTS_ACTIONS_FLAG;
            assert_eq!(validate_record(&want, &e), Err(RecordError::FieldNotAllowed), "phase {phase} without the flag");
            // a record that does not ask is unaffected by the flag
            assert_eq!(validate_record(&HookRecordV1::pass_through(1, phase, round), &e), Ok(()));
        }
    }

    #[test]
    fn o8_floor_books_exactly_one_after_actions_round() {
        let want = |phase, round| HookRecordV1 { want_callback: true, ..HookRecordV1::pass_through(1, phase, round) };
        // max_rounds 3: after_swap (round 1) may ask for round 2; round 2 may not ask for 3.
        assert_eq!(validate_record(&want(PHASE_AFTER_SWAP, 1), &expect(PHASE_AFTER_SWAP, 1)), Ok(()));
        assert_eq!(validate_record(&want(PHASE_AFTER_ACTIONS, 2), &expect(PHASE_AFTER_ACTIONS, 2)), Err(RecordError::RoundsExceeded));
        // at the AMM cap of 4, round 2 may ask for round 3, round 3 may not
        let mut e = expect(PHASE_AFTER_ACTIONS, 2);
        e.max_rounds = MAX_ROUNDS_CAP;
        assert_eq!(validate_record(&want(PHASE_AFTER_ACTIONS, 2), &e), Ok(()));
        e.round = 3;
        assert_eq!(validate_record(&want(PHASE_AFTER_ACTIONS, 3), &e), Err(RecordError::RoundsExceeded));
        // a record that does not ask is always fine at the last round
        assert_eq!(validate_record(&HookRecordV1::pass_through(1, PHASE_AFTER_ACTIONS, 2), &expect(PHASE_AFTER_ACTIONS, 2)), Ok(()));
    }

    #[test]
    fn specified_delta_may_not_flip_but_may_reach_zero() {
        assert_eq!(apply_specified_delta(-100, 30), Ok(-70));
        assert_eq!(apply_specified_delta(-100, 100), Ok(0), "exactly zero skips the walk");
        assert_eq!(apply_specified_delta(-100, 101), Err(RecordError::DeltaExceedsSwapAmount));
        assert_eq!(apply_specified_delta(100, -100), Ok(0));
        assert_eq!(apply_specified_delta(100, -101), Err(RecordError::DeltaExceedsSwapAmount));
        assert_eq!(apply_specified_delta(i128::MAX, 1), Err(RecordError::Overflow));
    }

    #[test]
    fn hook_delta_assembly_matches_hooks_sol() {
        // exact-in zero_for_one: specified is currency0
        assert_eq!(HookDeltaAssembly::assemble(-10, true, 3, 4), BalanceDelta { amount0: 3, amount1: 4 });
        // exact-in one_for_zero: specified is currency1
        assert_eq!(HookDeltaAssembly::assemble(-10, false, 3, 4), BalanceDelta { amount0: 4, amount1: 3 });
        // exact-out zero_for_one: specified is currency1
        assert_eq!(HookDeltaAssembly::assemble(10, true, 3, 4), BalanceDelta { amount0: 4, amount1: 3 });
        assert_eq!(HookDeltaAssembly::assemble(10, false, 3, 4), BalanceDelta { amount0: 3, amount1: 4 });
        assert_eq!(HookDeltaAssembly::assemble(-10, true, 0, 0), BalanceDelta::default());
    }

    #[test]
    fn callback_discriminators_cover_every_phase() {
        for p in 0..PHASE_COUNT {
            assert!(callback_discriminator(p).is_some());
        }
        assert!(callback_discriminator(PHASE_COUNT).is_none());
        assert_eq!(callback_discriminator(PHASE_BEFORE_SWAP), Some(crate::disc::callback::BEFORE_SWAP));
    }

    // ---- the settle flag -------------------------------------------------------

    const SETTLING: u16 = BEFORE_SWAP_FLAG | AFTER_SWAP_FLAG | AFTER_SWAP_RETURNS_DELTA_FLAG | SETTLES_FLAG;

    fn expect_settle(phase: u8, round: u8, flags: u16) -> RecordExpect {
        RecordExpect { version: 1, phase, round, flags, max_rounds: 2 }
    }

    #[test]
    fn a_record_without_a_price_limit_encodes_exactly_as_before() {
        let rec = HookRecordV1 { delta_specified: 7, fees: [1, 0, 0, 0, 0], ..HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0) };
        assert_eq!(encode_record(&rec), borsh::to_vec(&rec).unwrap());
        assert_eq!(decode_record(&encode_record(&rec)).unwrap(), rec);
    }

    #[test]
    fn the_price_limit_rides_a_sixteen_byte_tail_and_round_trips() {
        let rec = HookRecordV1 { sqrt_price_limit_x64: 1u128 << 70, ..HookRecordV1::pass_through(1, PHASE_BEFORE_SWAP, 0) };
        let bytes = encode_record(&rec);
        assert_eq!(bytes.len(), borsh::to_vec(&rec).unwrap().len() + RECORD_PRICE_LIMIT_TAIL_LEN);
        assert_eq!(decode_record(&bytes).unwrap(), rec);
        // any other trailing length is still refused (the strict decode)
        for extra in [1usize, 8, 15, 17, 32] {
            let mut b = borsh::to_vec(&rec).unwrap();
            b.extend(std::iter::repeat(1u8).take(extra));
            assert_eq!(decode_record(&b), Err(RecordError::BadReturn), "{extra} trailing bytes");
        }
        // a written zero tail is a second encoding of "no limit" and is refused
        let mut zero = borsh::to_vec(&rec).unwrap();
        zero.extend_from_slice(&[0u8; 16]);
        assert_eq!(decode_record(&zero), Err(RecordError::BadReturn));
        // the worst case with the tail still fits return data
        assert!(RECORD_MAX_LEN + RECORD_PRICE_LIMIT_TAIL_LEN <= MAX_RETURN_DATA);
    }

    #[test]
    fn the_price_limit_is_before_swap_and_settles_only() {
        let lim = |phase, round| HookRecordV1 { sqrt_price_limit_x64: 5, ..HookRecordV1::pass_through(1, phase, round) };
        assert_eq!(validate_record(&lim(PHASE_BEFORE_SWAP, 0), &expect_settle(PHASE_BEFORE_SWAP, 0, SETTLING)), Ok(()));
        assert_eq!(
            validate_record(&lim(PHASE_BEFORE_SWAP, 0), &expect_settle(PHASE_BEFORE_SWAP, 0, SETTLING & !SETTLES_FLAG)),
            Err(RecordError::FieldNotAllowed)
        );
        assert_eq!(validate_record(&lim(PHASE_AFTER_SWAP, 1), &expect_settle(PHASE_AFTER_SWAP, 1, SETTLING)), Err(RecordError::FieldNotAllowed));
    }

    #[test]
    fn an_after_swap_may_take_the_specified_side_only_when_it_settles() {
        let rec = HookRecordV1 { delta_specified: 100, delta_unspecified: -40, ..HookRecordV1::pass_through(1, PHASE_AFTER_SWAP, 1) };
        assert_eq!(validate_record(&rec, &expect_settle(PHASE_AFTER_SWAP, 1, SETTLING)), Ok(()));
        // without SETTLES, or without AFTER_SWAP_RETURNS_DELTA, the specified side stays before_swap's
        assert_eq!(validate_record(&rec, &expect_settle(PHASE_AFTER_SWAP, 1, SETTLING & !SETTLES_FLAG)), Err(RecordError::FieldNotAllowed));
        let no_delta = SETTLING & !AFTER_SWAP_RETURNS_DELTA_FLAG;
        assert_eq!(validate_record(&rec, &expect_settle(PHASE_AFTER_SWAP, 1, no_delta)), Err(RecordError::FieldNotAllowed));
        // after_actions still carries no delta at all
        let late = HookRecordV1 { delta_specified: 1, ..HookRecordV1::pass_through(1, PHASE_AFTER_ACTIONS, 2) };
        let mut e = expect_settle(PHASE_AFTER_ACTIONS, 2, SETTLING | REQUESTS_ACTIONS_FLAG);
        e.max_rounds = 4;
        assert_eq!(validate_record(&late, &e), Err(RecordError::FieldNotAllowed));
    }
}
