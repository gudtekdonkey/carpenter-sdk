//! Why a quote was refused.
//!
//! Every refusal the chain can produce for the same input carries the SAME NAME here as the
//! program's error variant ([`QuoteError::name`]). A parity harness compares a failed transaction's
//! error name with `name()`, so a quote that says "this swap fails" and a chain that says "this swap
//! succeeds" (or fails for another reason) is a mismatch, not a pass. Names that have no on-chain
//! twin start with `Quote`.
//!
//! The AMM's own refusals are variants here. A hook's refusals travel as [`QuoteError::Hook`], under
//! the hook program's error name; the hook's crate keeps the typed list (FLOOR's is
//! `floor_quote::FloorError`).

use core::fmt;

use carpenter_clmm_math::MathError;
use carpenter_types::hook::RecordError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QuoteError {
    // ---- input decoding (quoter-only) ------------------------------------------------------
    /// An account's bytes are the wrong length or carry the wrong discriminator.
    QuoteBadAccount(&'static str),
    /// The bitmap's header names another pool or spacing, or its word count is wrong.
    QuoteBadBitmap,
    /// A tick array names another pool, is misaligned, or appears twice.
    QuoteBadTickArray,
    /// The pool's hook sets a swap flag and no quoter for that hook program was supplied, so its
    /// callbacks cannot be replayed off-chain. Never quoted as a plain pool instead.
    QuoteHookNotQuotable,
    /// A hooked pool was quoted without the hook's own state (FLOOR: its `FloorPool`).
    QuoteHookStateMissing,
    /// A hook action the quoter does not replay. Kept for the name table: both action kinds the hook
    /// interface defines (`ModifyPosition`, `Swap`) are replayed.
    QuoteActionNotSupported,

    // ---- swap prelude and walk (carpenter_amm names) ------------------------------------------
    /// The pool's `paused` flag is set (the config authority's pause): the program refuses every swap
    /// with `PoolPaused` before any callback or walk. A guardian pause (`guardian_paused_until`) is a
    /// time window and is not judged here — the quoter reads no clock; check it against the cluster's
    /// clock where one is at hand.
    PoolPaused,
    /// `amount_specified == 0`.
    AmountZero,
    /// Swap prelude: `|amount_specified| > u64::MAX`.
    ///
    /// **Reports as `MathOverflow`.** `carpenter_amm` has no `AmountTooLarge` error; the program
    /// enforces the bound as a checked narrowing to `u64`, which raises `MathOverflow`, the same
    /// name `AmountOverflow` below reports under. If the program ever adds a dedicated variant,
    /// change the `name()` arm and this comment together.
    AmountTooLarge,
    /// v4 `Pool.sol:322-337`: the limit is on the wrong side of the price or out of bounds.
    PriceLimitInvalid,
    /// Exact output at a 100 % fee (v4 `Pool.sol:310-316`).
    InvalidFeeForExactOut,
    /// The bitmap marks an array the caller did not supply. `start_index` names it, so a
    /// client can fetch it and quote again.
    TickArrayMissing { start_index: i32 },
    /// A hook action needs an array the caller did not supply.
    HookActionAccountMissing { start_index: i32 },
    /// The hook's specified delta flips the swap's direction (`Hooks.sol:273-278`).
    HookDeltaExceedsSwapAmount,
    /// The trader side flips once the hook delta is applied.
    HookDeltaExceedsSwapOutput,
    /// A negative hook delta on a pool whose hook does not settle.
    HookDeltaNegative,
    /// A settling hook's `after_swap` took more of the specified side than the walk left unfilled, a
    /// negative amount, or any of it on an exact-out.
    HookSettleOverfill,
    /// An action's debit exceeds the hook's claims at that moment.
    HookClaimsInsufficient,
    /// A record asks for a callback round beyond the pool's `max_rounds`.
    HookRoundsExceeded,
    /// A record field the AMM would refuse for its phase or the pool's flags.
    HookFieldNotAllowed,
    /// A fee override above 1,000,000.
    FeeInvalid,
    /// A record's `fees` claims more than its delta, or a platform cut above the skim.
    HookFeesInconsistent,
    /// The trader's settled amount does not fit a u64.
    AmountOverflow,

    // ---- hook refusals -------------------------------------------------------------------------
    /// A refusal raised by a hook quoter (or a helper of the hook's crate), carrying the on-chain error
    /// name it mirrors — e.g. FLOOR's `ExactOutNotSupported` or `WrongPool`. [`QuoteError::name`]
    /// returns the string unchanged.
    Hook(&'static str),

    // ---- math ---------------------------------------------------------------------------------
    /// A `carpenter-clmm-math` error, by its own name.
    Math(MathError),
    /// Checked integer arithmetic overflowed.
    Overflow,
}

impl QuoteError {
    /// The on-chain error name this refusal corresponds to (see the module docs).
    pub fn name(&self) -> &'static str {
        use QuoteError::*;
        match self {
            QuoteBadAccount(_) => "QuoteBadAccount",
            QuoteBadBitmap => "QuoteBadBitmap",
            QuoteBadTickArray => "QuoteBadTickArray",
            QuoteHookNotQuotable => "QuoteHookNotQuotable",
            QuoteHookStateMissing => "QuoteHookStateMissing",
            QuoteActionNotSupported => "QuoteActionNotSupported",
            PoolPaused => "PoolPaused",
            AmountZero => "AmountZero",
            // the prelude bound is a checked narrowing to u64; the chain has no other name (above).
            AmountTooLarge => "MathOverflow",
            PriceLimitInvalid => "PriceLimitInvalid",
            InvalidFeeForExactOut => "InvalidFeeForExactOut",
            TickArrayMissing { .. } => "TickArrayMissing",
            HookActionAccountMissing { .. } => "HookActionAccountMissing",
            HookDeltaExceedsSwapAmount => "HookDeltaExceedsSwapAmount",
            HookDeltaExceedsSwapOutput => "HookDeltaExceedsSwapOutput",
            HookDeltaNegative => "HookDeltaNegative",
            HookSettleOverfill => "HookSettleOverfill",
            HookClaimsInsufficient => "HookClaimsInsufficient",
            HookRoundsExceeded => "HookRoundsExceeded",
            HookFieldNotAllowed => "HookFieldNotAllowed",
            FeeInvalid => "FeeInvalid",
            HookFeesInconsistent => "HookFeesInconsistent",
            // The chain's settlement narrowing and checked arithmetic both surface as `MathOverflow`.
            AmountOverflow => "MathOverflow",
            Hook(name) => name,
            Math(e) => math_name(*e),
            Overflow => "MathOverflow",
        }
    }
}

fn math_name(e: MathError) -> &'static str {
    match e {
        MathError::TickOutOfRange => "TickOutOfRange",
        MathError::SqrtPriceOutOfRange => "SqrtPriceOutOfRange",
        MathError::CalculateOverflow => "CalculateOverflow",
        MathError::ZeroLiquidity => "ZeroLiquidity",
        MathError::ZeroSqrtPrice => "ZeroSqrtPrice",
        MathError::MaxTokenOverflow => "MaxTokenOverflow",
        MathError::LiquidityUnderflow => "LiquidityUnderflow",
        MathError::LiquidityOverflow => "LiquidityOverflow",
        MathError::PriceTargetOvershoot => "PriceTargetOvershoot",
        MathError::InvalidTickArrayBoundary => "InvalidTickArrayBoundary",
        MathError::FeeRateTooHigh => "FeeRateTooHigh",
        MathError::InvalidFeeForExactOut => "InvalidFeeForExactOut",
        MathError::InvalidTickSpacing => "InvalidTickSpacing",
        MathError::BitmapLengthMismatch => "BitmapLengthMismatch",
        MathError::DirectionMismatch => "DirectionMismatch",
    }
}

impl From<MathError> for QuoteError {
    fn from(e: MathError) -> Self {
        match e {
            // exact output at a 100 % fee has one name whichever layer catches it.
            MathError::InvalidFeeForExactOut => QuoteError::InvalidFeeForExactOut,
            other => QuoteError::Math(other),
        }
    }
}

impl From<RecordError> for QuoteError {
    fn from(e: RecordError) -> Self {
        match e {
            RecordError::BadReturn | RecordError::FieldNotAllowed => QuoteError::HookFieldNotAllowed,
            RecordError::FeeTooLarge => QuoteError::FeeInvalid,
            RecordError::RoundsExceeded => QuoteError::HookRoundsExceeded,
            RecordError::DeltaExceedsSwapAmount => QuoteError::HookDeltaExceedsSwapAmount,
            RecordError::Overflow => QuoteError::Overflow,
            RecordError::FeesInconsistent => QuoteError::HookFeesInconsistent,
        }
    }
}

impl fmt::Display for QuoteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QuoteError::QuoteBadAccount(which) => write!(f, "QuoteBadAccount({which})"),
            QuoteError::TickArrayMissing { start_index } => write!(f, "TickArrayMissing({start_index})"),
            QuoteError::HookActionAccountMissing { start_index } => write!(f, "HookActionAccountMissing({start_index})"),
            other => f.write_str(other.name()),
        }
    }
}

impl std::error::Error for QuoteError {}

pub type QuoteResult<T> = Result<T, QuoteError>;
