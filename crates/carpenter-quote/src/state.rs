//! Account bytes → the account layouts, and the set of tick arrays a walk reads.
//!
//! Decoding copies (`pod_read_unaligned`): account bytes handed in from JS or read off an RPC are
//! not always at an 8-byte alignment, and a `cast_ref` on them panics on the misaligned
//! case only (a wasm page that dies on one pool in eight).

use std::collections::BTreeMap;

use carpenter_clmm_math::bitmap;
use carpenter_types::layout::{Layout, Pool, TickArray, TickArrayBitmapHeader};

use crate::error::{QuoteError, QuoteResult};

/// Decode an Anchor zero-copy account: `discriminator ‖ struct`, exact length. The bitmap, whose
/// length depends on the spacing, has its own decoder; a hook's account that may carry a tail is
/// decoded by that hook's crate.
pub fn decode_account<T: Layout>(data: &[u8]) -> QuoteResult<T> {
    if data.len() != T::ACCOUNT_LEN {
        return Err(QuoteError::QuoteBadAccount(T::NAME));
    }
    if data[..8] != T::DISCRIMINATOR {
        return Err(QuoteError::QuoteBadAccount(T::NAME));
    }
    Ok(bytemuck::pod_read_unaligned(&data[8..]))
}

/// Encode a layout back to account bytes (the inverse of [`decode_account`]).
pub fn encode_account<T: Layout>(value: &T) -> Vec<u8> {
    let mut out = Vec::with_capacity(T::ACCOUNT_LEN);
    out.extend_from_slice(&T::DISCRIMINATOR);
    out.extend_from_slice(bytemuck::bytes_of(value));
    out
}

pub fn decode_pool(data: &[u8]) -> QuoteResult<Pool> {
    decode_account::<Pool>(data)
}

pub fn decode_tick_array(data: &[u8]) -> QuoteResult<TickArray> {
    decode_account::<TickArray>(data)
}

/// The bitmap account: a 40-byte header, then `N` little-endian words. Checks the header's
/// pool and spacing against the pool being quoted and the exact `8 + 40 + 8·N` length.
pub fn decode_bitmap(data: &[u8], pool_key: &[u8; 32], tick_spacing: u16) -> QuoteResult<Vec<u64>> {
    let words = bitmap::word_count(tick_spacing)?;
    let fixed = TickArrayBitmapHeader::ACCOUNT_LEN;
    if data.len() != fixed + 8 * words || data[..8] != TickArrayBitmapHeader::DISCRIMINATOR {
        return Err(QuoteError::QuoteBadBitmap);
    }
    let header: TickArrayBitmapHeader = bytemuck::pod_read_unaligned(&data[8..fixed]);
    if &header.pool != pool_key || header.tick_spacing != tick_spacing {
        return Err(QuoteError::QuoteBadBitmap);
    }
    Ok(data[fixed..]
        .chunks_exact(8)
        .map(|c| u64::from_le_bytes(c.try_into().expect("8-byte chunk")))
        .collect())
}

/// The tick arrays a quote may read, keyed by `start_index`, plus the pool's bitmap.
///
/// The bitmap is optional. Without it, "initialized" is DERIVED from the arrays supplied (an array
/// with `initialized_tick_count > 0` is marked), which is exactly the chain's bitmap when every
/// initialized array is supplied, and silently less liquidity when one is not. With it, a marked
/// array that was not supplied is `TickArrayMissing { start_index }`, as on chain. Clients
/// that can read the bitmap account should pass it.
#[derive(Clone, Debug)]
pub struct TickArraySet {
    pub tick_spacing: u16,
    pub arrays: BTreeMap<i32, TickArray>,
    pub words: Vec<u64>,
    /// Whether `words` came from the bitmap account (true) or were derived (false).
    pub bitmap_supplied: bool,
}

impl TickArraySet {
    /// Build from decoded arrays. Each must name `pool_key`, start on a `60·s` boundary, and appear
    /// once.
    pub fn new(pool_key: &[u8; 32], tick_spacing: u16, arrays: Vec<TickArray>, words: Option<Vec<u64>>) -> QuoteResult<Self> {
        let mut map = BTreeMap::new();
        for a in arrays {
            if &a.pool != pool_key || !bitmap::is_valid_start_index(a.start_index, tick_spacing)? {
                return Err(QuoteError::QuoteBadTickArray);
            }
            if map.insert(a.start_index, a).is_some() {
                // duplicate keys in the set are refused on chain too.
                return Err(QuoteError::QuoteBadTickArray);
            }
        }
        let (words, bitmap_supplied) = match words {
            Some(w) => {
                bitmap::check_words(&w, tick_spacing)?;
                (w, true)
            }
            None => {
                let mut w = vec![0u64; bitmap::word_count(tick_spacing)?];
                for (start, a) in &map {
                    if a.initialized_tick_count > 0 {
                        bitmap::set_initialized(&mut w, bitmap::array_bit(*start, tick_spacing)?, true)?;
                    }
                }
                (w, false)
            }
        };
        Ok(Self { tick_spacing, arrays: map, words, bitmap_supplied })
    }

    /// Build from raw account bytes: each tick array is a full 3,888-byte account.
    pub fn from_accounts(pool_key: &[u8; 32], tick_spacing: u16, arrays: &[&[u8]], bitmap: Option<&[u8]>) -> QuoteResult<Self> {
        let decoded = arrays.iter().map(|d| decode_tick_array(d)).collect::<QuoteResult<Vec<_>>>()?;
        let words = match bitmap {
            Some(b) => Some(decode_bitmap(b, pool_key, tick_spacing)?),
            None => None,
        };
        Self::new(pool_key, tick_spacing, decoded, words)
    }

    /// Whether the bitmap marks the array starting at `start_index`.
    pub fn is_marked(&self, start_index: i32) -> QuoteResult<bool> {
        Ok(bitmap::is_initialized(&self.words, bitmap::array_bit(start_index, self.tick_spacing)?)?)
    }
}

/// Encode a bitmap account (the inverse of [`decode_bitmap`]).
pub fn encode_bitmap(pool_key: &[u8; 32], tick_spacing: u16, words: &[u64]) -> Vec<u8> {
    let header = TickArrayBitmapHeader {
        pool: *pool_key,
        tick_spacing,
        pad0: 0,
        arrays_per_side: carpenter_types::layout::bitmap_arrays_per_side(tick_spacing),
    };
    let mut out = encode_account(&header);
    for w in words {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}
