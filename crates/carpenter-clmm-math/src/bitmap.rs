// Derived from raydium-io/raydium-clmm @ ed7c84a5, programs/amm/src/libraries/tick_array_bit_map.rs and the
// start-index helpers of programs/amm/src/states/tick_array.rs (`get_array_start_index`,
// `check_is_valid_start_index`, `tick_count`), Apache-2.0.
// Modified by FLOOR, 2026: re-shaped for Carpenter's `TickArrayBitmap`. Upstream searches a fixed
// `U1024` (512 arrays either side of 0, plus an extension account beyond that); here the bitmap is a
// spacing-sized `&[u64]` that covers the whole tick range, bit `k/(60·s) + arrays_per_side` for the array
// starting at `k`, so there is no extension and no extension-binding bug class. The most/least-significant
// set-bit search keeps upstream's direction semantics (zero_for_one searches toward lower start indices),
// done word by word instead of by shifting a 1024-bit integer. A miss returns `None`: no extension exists
// to ask, which is the case Raydium fcad161 fixed ("default-bitmap-boundary misclassification").
// `array_start_index` refuses a tick outside [MIN_TICK, MAX_TICK] (upstream's product overflowed i32).
// Anchor / `ErrorCode` replaced by `MathResult` / `MathError`. Upstream's bitmap tests are ported
// against the new layout (same start indices, same expected sequences) where their assertion survives it.

use crate::error::{MathError, MathResult};
use crate::tick_math::{MAX_TICK, MIN_TICK};

/// Ticks per tick array (upstream `TICK_ARRAY_SIZE`).
pub const TICK_ARRAY_SIZE: i32 = 60;

/// Which way a search moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Toward lower start indices: price down, `zero_for_one == true`.
    Down,
    /// Toward higher start indices: price up, `zero_for_one == false`.
    Up,
}

impl Direction {
    #[inline]
    pub const fn from_zero_for_one(zero_for_one: bool) -> Self {
        if zero_for_one {
            Direction::Down
        } else {
            Direction::Up
        }
    }
}

/// Ticks covered by one array: `60 · tick_spacing` (upstream `TickArrayState::tick_count`).
#[inline]
pub fn ticks_in_array(tick_spacing: u16) -> MathResult<i32> {
    if tick_spacing == 0 {
        return Err(MathError::InvalidTickSpacing);
    }
    Ok(TICK_ARRAY_SIZE * i32::from(tick_spacing))
}

/// `ceil(443,636 / (60 · s))`.
pub fn arrays_per_side(tick_spacing: u16) -> MathResult<u32> {
    let per = ticks_in_array(tick_spacing)?;
    Ok(((MAX_TICK + per - 1) / per) as u32)
}

/// Bits the bitmap must hold: `2 · arrays_per_side + 1`.
pub fn bit_count(tick_spacing: u16) -> MathResult<u32> {
    Ok(2 * arrays_per_side(tick_spacing)? + 1)
}

/// Words the bitmap account carries: `N = ceil((2 · arrays_per_side + 1) / 64)`.
pub fn word_count(tick_spacing: u16) -> MathResult<usize> {
    Ok(((bit_count(tick_spacing)? + 63) / 64) as usize)
}

/// The start index of the array holding `tick`: rounds toward negative infinity
/// (upstream `TickArrayState::get_array_start_index`).
///
/// FLOOR: refuses a tick outside `[MIN_TICK, MAX_TICK]` as `TickOutOfRange`. Upstream computed
/// `start * per` unchecked, which overflows i32 for a tick near the i32 bounds and a large spacing
/// (`array_start_index(i32::MIN, 32767)`: -1093 · 1,966,020 < i32::MIN), panicking in debug and
/// wrapping to a wrong index in release. In range the product is bounded by ±(443,636 + 60·65,535).
pub fn array_start_index(tick: i32, tick_spacing: u16) -> MathResult<i32> {
    let per = ticks_in_array(tick_spacing)?;
    if !(MIN_TICK..=MAX_TICK).contains(&tick) {
        return Err(MathError::TickOutOfRange);
    }
    let mut start = tick / per;
    if tick < 0 && tick % per != 0 {
        start -= 1;
    }
    Ok(start * per)
}

/// Whether `start_index` begins an array that holds at least one tick in `[MIN_TICK, MAX_TICK]`
/// (upstream `TickArrayState::check_is_valid_start_index`).
pub fn is_valid_start_index(start_index: i32, tick_spacing: u16) -> MathResult<bool> {
    let per = ticks_in_array(tick_spacing)?;
    if start_index > MAX_TICK {
        return Ok(false);
    }
    if start_index < MIN_TICK {
        return Ok(start_index == array_start_index(MIN_TICK, tick_spacing)?);
    }
    Ok(start_index % per == 0)
}

/// The bit of the array starting at `start_index`: `start_index / (60·s) + arrays_per_side`.
pub fn array_bit(start_index: i32, tick_spacing: u16) -> MathResult<u32> {
    if !is_valid_start_index(start_index, tick_spacing)? {
        return Err(MathError::InvalidTickArrayBoundary);
    }
    let per = ticks_in_array(tick_spacing)?;
    let aps = arrays_per_side(tick_spacing)? as i32;
    let bit = start_index / per + aps;
    debug_assert!(bit >= 0 && (bit as u32) < bit_count(tick_spacing)?);
    Ok(bit as u32)
}

/// The start index of the array whose bit is `bit`; the inverse of [`array_bit`].
pub fn start_index_of_bit(bit: u32, tick_spacing: u16) -> MathResult<i32> {
    if bit >= bit_count(tick_spacing)? {
        return Err(MathError::InvalidTickArrayBoundary);
    }
    let per = ticks_in_array(tick_spacing)?;
    let aps = arrays_per_side(tick_spacing)? as i32;
    let start = (bit as i32 - aps) * per;
    if !is_valid_start_index(start, tick_spacing)? {
        // only the unused top bit (2·aps) can land here
        return Err(MathError::InvalidTickArrayBoundary);
    }
    Ok(start)
}

/// Refuses a word slice of the wrong length for `tick_spacing` (every load checks the length).
pub fn check_words(words: &[u64], tick_spacing: u16) -> MathResult<()> {
    if words.len() != word_count(tick_spacing)? {
        return Err(MathError::BitmapLengthMismatch);
    }
    Ok(())
}

/// Whether bit `bit` is set. A bit beyond the slice is an error, never "unset".
pub fn is_initialized(words: &[u64], bit: u32) -> MathResult<bool> {
    let w = words
        .get((bit / 64) as usize)
        .ok_or(MathError::InvalidTickArrayBoundary)?;
    Ok(w & (1u64 << (bit % 64)) != 0)
}

/// Sets or clears bit `bit` (the AMM flips it when an array's initialized-tick count goes 0↔1).
pub fn set_initialized(words: &mut [u64], bit: u32, initialized: bool) -> MathResult<()> {
    let w = words
        .get_mut((bit / 64) as usize)
        .ok_or(MathError::InvalidTickArrayBoundary)?;
    let mask = 1u64 << (bit % 64);
    if initialized {
        *w |= mask;
    } else {
        *w &= !mask;
    }
    Ok(())
}

/// The nearest set bit STRICTLY past `start` in `dir`, or `None` when there is none in the slice.
///
/// Exclusive, like upstream `next_initialized_tick_array_start_index` (which begins one array past
/// `last_tick_array_start_index`); pair it with [`is_initialized`] for the array the price is in.
/// A `start` at or beyond the slice's capacity searches every bit below it for `Down` and finds
/// nothing for `Up`.
pub fn next_initialized_array(words: &[u64], start: u32, dir: Direction) -> Option<u32> {
    let capacity = words.len() as u64 * 64;
    match dir {
        Direction::Down => {
            if start == 0 || words.is_empty() {
                return None;
            }
            // highest candidate bit is start - 1, clamped into the slice
            let top = core::cmp::min(start as u64 - 1, capacity - 1);
            let mut wi = (top / 64) as usize;
            let b = (top % 64) as u32;
            // keep bits 0..=b of the first word
            let mut word = if b == 63 {
                words[wi]
            } else {
                words[wi] & ((1u64 << (b + 1)) - 1)
            };
            loop {
                if word != 0 {
                    return Some(wi as u32 * 64 + (63 - word.leading_zeros()));
                }
                if wi == 0 {
                    return None;
                }
                wi -= 1;
                word = words[wi];
            }
        }
        Direction::Up => {
            let first = start as u64 + 1;
            if first >= capacity {
                return None;
            }
            let mut wi = (first / 64) as usize;
            let b = (first % 64) as u32;
            // keep bits b..=63 of the first word
            let mut word = words[wi] & (u64::MAX << b);
            loop {
                if word != 0 {
                    return Some(wi as u32 * 64 + word.trailing_zeros());
                }
                wi += 1;
                if wi == words.len() {
                    return None;
                }
                word = words[wi];
            }
        }
    }
}

/// The start index of the next initialized array strictly past `last_start_index` in the swap
/// direction, or `None` when the direction is exhausted. Checks the slice length for the spacing and
/// refuses an invalid `last_start_index`. A set bit that decodes to no valid array (the unused top bit)
/// is `InvalidTickArrayBoundary`, never silently skipped.
pub fn next_initialized_array_start_index(
    words: &[u64],
    tick_spacing: u16,
    last_start_index: i32,
    zero_for_one: bool,
) -> MathResult<Option<i32>> {
    check_words(words, tick_spacing)?;
    let bit = array_bit(last_start_index, tick_spacing)?;
    match next_initialized_array(words, bit, Direction::from_zero_for_one(zero_for_one)) {
        None => Ok(None),
        Some(found) => start_index_of_bit(found, tick_spacing).map(Some),
    }
}

#[cfg(test)]
mod test {
    use super::*;

    fn bitmap_with(tick_spacing: u16, starts: &[i32]) -> Vec<u64> {
        let mut words = vec![0u64; word_count(tick_spacing).unwrap()];
        for &s in starts {
            set_initialized(&mut words, array_bit(s, tick_spacing).unwrap(), true).unwrap();
        }
        words
    }

    /// The account size table: s=60 → N=4 (80-B account), s=10 → 240 B, s=1 → 1,904 B.
    #[test]
    fn sizes_match_spec_4_1() {
        assert_eq!(word_count(60).unwrap(), 4);
        assert_eq!(8 + 40 + 8 * word_count(60).unwrap(), 80);
        assert_eq!(8 + 40 + 8 * word_count(10).unwrap(), 240);
        assert_eq!(8 + 40 + 8 * word_count(1).unwrap(), 1_904);
        assert_eq!(arrays_per_side(60).unwrap(), 124);
        assert_eq!(ticks_in_array(0), Err(MathError::InvalidTickSpacing));
    }

    /// Every valid start index of every spacing round-trips through its bit, the extreme arrays
    /// land on bits 0 and 2·aps−1, and no two arrays share a bit.
    #[test]
    fn bit_round_trip_covers_the_full_tick_range() {
        for spacing in [1u16, 2, 10, 15, 60, 100, 200, 1000, 32767] {
            let per = ticks_in_array(spacing).unwrap();
            let aps = arrays_per_side(spacing).unwrap();
            let lo = array_start_index(MIN_TICK, spacing).unwrap();
            let hi = array_start_index(MAX_TICK, spacing).unwrap();
            assert_eq!(array_bit(lo, spacing).unwrap(), 0, "spacing {spacing}");
            assert_eq!(
                array_bit(hi, spacing).unwrap(),
                2 * aps - 1,
                "spacing {spacing}"
            );
            let mut expected = 0;
            let mut s = lo;
            while s <= hi {
                let bit = array_bit(s, spacing).unwrap();
                assert_eq!(bit, expected);
                assert_eq!(start_index_of_bit(bit, spacing).unwrap(), s);
                expected += 1;
                s += per;
            }
            assert!(array_bit(lo - per, spacing).is_err());
            assert!(array_bit(hi + per, spacing).is_err());
            assert!(array_bit(per / 2, spacing).is_err() || per == 1);
            // the top bit is allocated but decodes to no array
            assert!(start_index_of_bit(2 * aps, spacing).is_err());
            assert!(start_index_of_bit(bit_count(spacing).unwrap(), spacing).is_err());
        }
    }

    /// Upstream `get_array_start_index_test` (tick_array.rs:863-890 @ ed7c84a5), every assertion
    /// verbatim: rounding toward negative infinity, incl. negatives and both tick bounds.
    #[test]
    fn get_array_start_index_test() {
        assert_eq!(array_start_index(120, 3).unwrap(), 0);
        assert_eq!(array_start_index(1002, 30).unwrap(), 0);
        assert_eq!(array_start_index(-120, 3).unwrap(), -180);
        assert_eq!(array_start_index(-1002, 30).unwrap(), -1800);
        assert_eq!(array_start_index(-20, 10).unwrap(), -600);
        assert_eq!(array_start_index(20, 10).unwrap(), 0);
        assert_eq!(array_start_index(-1002, 10).unwrap(), -1200);
        assert_eq!(array_start_index(-600, 10).unwrap(), -600);
        assert_eq!(array_start_index(-30720, 1).unwrap(), -30720);
        assert_eq!(array_start_index(30720, 1).unwrap(), 30720);
        assert_eq!(array_start_index(MIN_TICK, 1).unwrap(), -443640);
        assert_eq!(array_start_index(MAX_TICK, 1).unwrap(), 443580);
        assert_eq!(array_start_index(MAX_TICK, 60).unwrap(), 442800);
        assert_eq!(array_start_index(MIN_TICK, 60).unwrap(), -446400);
    }

    /// The crate's own boundary picks on top of upstream's set (kept: they cover the 599/600/601
    /// edges upstream does not).
    #[test]
    fn array_start_index_rounds_toward_negative_infinity() {
        assert_eq!(array_start_index(0, 10).unwrap(), 0);
        assert_eq!(array_start_index(599, 10).unwrap(), 0);
        assert_eq!(array_start_index(600, 10).unwrap(), 600);
        assert_eq!(array_start_index(-1, 10).unwrap(), -600);
        assert_eq!(array_start_index(-601, 10).unwrap(), -1200);
    }

    /// `is_valid_start_index` directly, every branch (upstream `check_is_valid_start_index`): the
    /// one array that starts below MIN_TICK is valid, the array below it is not, anything above
    /// MAX_TICK is not, and a misaligned in-range index is not.
    #[test]
    fn is_valid_start_index_covers_every_branch() {
        for spacing in [1u16, 10, 60, 32_767, u16::MAX] {
            let per = ticks_in_array(spacing).unwrap();
            let lo = array_start_index(MIN_TICK, spacing).unwrap();
            let hi = array_start_index(MAX_TICK, spacing).unwrap();
            assert!(is_valid_start_index(lo, spacing).unwrap(), "lo, spacing {spacing}");
            assert!(is_valid_start_index(hi, spacing).unwrap(), "hi, spacing {spacing}");
            assert!(is_valid_start_index(0, spacing).unwrap(), "0, spacing {spacing}");
            // The below-MIN_TICK branch: only the array holding MIN_TICK is valid down there.
            assert!(!is_valid_start_index(lo - per, spacing).unwrap(), "lo - per, spacing {spacing}");
            if lo < MIN_TICK {
                assert!(!is_valid_start_index(lo + 1, spacing).unwrap(), "lo + 1, spacing {spacing}");
            }
            // Above MAX_TICK: never, aligned or not.
            assert!(!is_valid_start_index(MAX_TICK + 1, spacing).unwrap(), "MAX+1, spacing {spacing}");
            assert!(!is_valid_start_index(hi + per, spacing).unwrap(), "hi + per, spacing {spacing}");
            assert!(!is_valid_start_index(i32::MAX, spacing).unwrap(), "i32::MAX, spacing {spacing}");
            // Misaligned in range.
            assert!(!is_valid_start_index(1, spacing).unwrap(), "1, spacing {spacing}");
            assert!(!is_valid_start_index(per - 1, spacing).unwrap(), "per - 1, spacing {spacing}");
            assert!(!is_valid_start_index(-1, spacing).unwrap(), "-1, spacing {spacing}");
        }
        // Spacing 60 (the FLOOR default): the exact numbers.
        assert!(is_valid_start_index(-446_400, 60).unwrap());
        assert!(!is_valid_start_index(-450_000, 60).unwrap());
        assert!(is_valid_start_index(442_800, 60).unwrap());
        assert!(!is_valid_start_index(446_400, 60).unwrap());
        assert!(!is_valid_start_index(442_801, 60).unwrap());
    }

    /// A tick outside `[MIN_TICK, MAX_TICK]` is an error, never an i32 overflow (debug panic /
    /// release wrap), at the largest spacing where `start * per` would leave i32.
    #[test]
    fn array_start_index_refuses_out_of_range_ticks() {
        for tick in [i32::MIN, i32::MAX, MIN_TICK - 1, MAX_TICK + 1] {
            for spacing in [1u16, 60, 32_767, u16::MAX] {
                assert_eq!(
                    array_start_index(tick, spacing),
                    Err(MathError::TickOutOfRange),
                    "tick {tick} spacing {spacing}"
                );
            }
        }
        for spacing in [32_767u16, u16::MAX] {
            assert!(array_start_index(MIN_TICK, spacing).unwrap() <= MIN_TICK);
            assert!(array_start_index(MAX_TICK, spacing).unwrap() <= MAX_TICK);
        }
    }

    /// Port of upstream `find_next_init_pos_in_bit_map_with_eigenvalues`: the same eleven set arrays
    /// at spacing 10, the same expected walk in both directions.
    #[test]
    fn find_next_init_pos_in_bit_map_with_eigenvalues() {
        let s = 10;
        let words = bitmap_with(
            s,
            &[
                -307200, -39000, -38400, -1800, -1200, -600, 0, 600, 1200, 38400, 306600,
            ],
        );
        let next = |from: i32, zfo: bool| {
            next_initialized_array_start_index(&words, s, from, zfo)
                .unwrap()
                .unwrap()
        };
        assert_eq!(next(0, true), -600);
        assert_eq!(next(-600, true), -1200);
        assert_eq!(next(-1200, true), -1800);
        assert_eq!(next(-1800, true), -38400);
        assert_eq!(next(-38400, true), -39000);
        assert_eq!(next(-39000, true), -307200);
        assert_eq!(
            next_initialized_array_start_index(&words, s, -307200, true).unwrap(),
            None
        );

        assert_eq!(next(0, false), 600);
        assert_eq!(next(600, false), 1200);
        assert_eq!(next(1200, false), 38400);
        assert_eq!(next(38400, false), 306600);
        assert_eq!(
            next_initialized_array_start_index(&words, s, 306600, false).unwrap(),
            None
        );
    }

    /// Port of upstream `next_initialized_tick_array_with_all_initialized_bit_test`, on the new layout:
    /// with every array initialized, each step moves exactly one array, over the WHOLE range (upstream
    /// stopped at its ±512-array default bitmap unless the spacing was 60).
    #[test]
    fn next_initialized_tick_array_with_all_initialized_bit_test() {
        for spacing in [1u16, 10, 60] {
            let per = ticks_in_array(spacing).unwrap();
            let mut words = vec![0u64; word_count(spacing).unwrap()];
            let lo = array_start_index(MIN_TICK, spacing).unwrap();
            let hi = array_start_index(MAX_TICK, spacing).unwrap();
            let mut s = lo;
            while s <= hi {
                set_initialized(&mut words, array_bit(s, spacing).unwrap(), true).unwrap();
                s += per;
            }
            let mut s = lo;
            while s < hi {
                let n = next_initialized_array_start_index(&words, spacing, s, false)
                    .unwrap()
                    .unwrap();
                assert_eq!(n, s + per);
                s = n;
            }
            assert_eq!(
                next_initialized_array_start_index(&words, spacing, hi, false).unwrap(),
                None
            );
            let mut s = hi;
            while s > lo {
                let n = next_initialized_array_start_index(&words, spacing, s, true)
                    .unwrap()
                    .unwrap();
                assert_eq!(n, s - per);
                s = n;
            }
            assert_eq!(
                next_initialized_array_start_index(&words, spacing, lo, true).unwrap(),
                None
            );
        }
    }

    /// Port of upstream `next_initialized_tick_array_start_index_boundary_test` and pool.rs
    /// `full_range_default_bitmap_returns_none_not_missing_extension` (fcad161): searching past the last
    /// array, or an empty bitmap, is `None`, never an error that asks for an extension.
    #[test]
    fn exhausted_direction_is_none_not_an_error() {
        for spacing in [1u16, 10, 15, 60] {
            let words = vec![0u64; word_count(spacing).unwrap()];
            for zfo in [true, false] {
                assert_eq!(
                    next_initialized_array_start_index(&words, spacing, 0, zfo).unwrap(),
                    None
                );
            }
            let full = vec![u64::MAX; word_count(spacing).unwrap()];
            let lo = array_start_index(MIN_TICK, spacing).unwrap();
            let hi = array_start_index(MAX_TICK, spacing).unwrap();
            assert_eq!(
                next_initialized_array_start_index(&full, spacing, lo, true).unwrap(),
                None
            );
            // with every bit set, stepping up from the last array hits the unused top bit: a named
            // error, because a set bit that names no array is corruption, not liquidity
            assert_eq!(
                next_initialized_array_start_index(&full, spacing, hi, false),
                Err(MathError::InvalidTickArrayBoundary)
            );
        }
    }

    #[test]
    fn wrong_length_and_invalid_start_are_refused() {
        let words = vec![0u64; 3];
        assert_eq!(
            next_initialized_array_start_index(&words, 60, 0, true),
            Err(MathError::BitmapLengthMismatch)
        );
        let words = vec![0u64; 4];
        assert_eq!(
            next_initialized_array_start_index(&words, 60, 1, true),
            Err(MathError::InvalidTickArrayBoundary)
        );
        assert!(is_initialized(&words, 4 * 64).is_err());
    }

    /// The raw word search at word edges: bits 0, 63, 64, 127 and the last bit, both directions.
    #[test]
    fn raw_search_crosses_word_edges() {
        let mut w = vec![0u64; 3];
        for b in [0u32, 63, 64, 127, 191] {
            set_initialized(&mut w, b, true).unwrap();
        }
        use Direction::*;
        assert_eq!(next_initialized_array(&w, 191, Down), Some(127));
        assert_eq!(next_initialized_array(&w, 127, Down), Some(64));
        assert_eq!(next_initialized_array(&w, 64, Down), Some(63));
        assert_eq!(next_initialized_array(&w, 63, Down), Some(0));
        assert_eq!(next_initialized_array(&w, 0, Down), None);
        assert_eq!(next_initialized_array(&w, 0, Up), Some(63));
        assert_eq!(next_initialized_array(&w, 63, Up), Some(64));
        assert_eq!(next_initialized_array(&w, 64, Up), Some(127));
        assert_eq!(next_initialized_array(&w, 127, Up), Some(191));
        assert_eq!(next_initialized_array(&w, 191, Up), None);
        assert_eq!(next_initialized_array(&w, 10_000, Down), Some(191));
        assert_eq!(next_initialized_array(&w, 10_000, Up), None);
        assert_eq!(next_initialized_array(&[], 5, Down), None);
        assert!(is_initialized(&w, 127).unwrap());
        set_initialized(&mut w, 127, false).unwrap();
        assert!(!is_initialized(&w, 127).unwrap());
        assert_eq!(next_initialized_array(&w, 191, Down), Some(64));
    }

    /// Raw search against a linear scan, over pseudo-random bitmaps.
    #[test]
    fn raw_search_matches_a_linear_scan() {
        let mut x: u64 = 0x9E37_79B9_7F4A_7C15;
        for _ in 0..200 {
            let mut w = vec![0u64; 5];
            for word in w.iter_mut() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // sparse
                *word = x & (x >> 3) & (x >> 5) & (x >> 11);
            }
            let set = |b: u32| is_initialized(&w, b).unwrap();
            for start in 0..(5 * 64) {
                let down = (0..start).rev().find(|&b| set(b));
                let up = (start + 1..5 * 64).find(|&b| set(b));
                assert_eq!(next_initialized_array(&w, start, Direction::Down), down);
                assert_eq!(next_initialized_array(&w, start, Direction::Up), up);
            }
        }
    }
}
