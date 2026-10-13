//! 128-bit integers as two little-endian `u64` limbs.
//!
//! A bare `u128` in a `#[repr(C)]` account struct aligns to 16 on some targets and 8 on
//! others (SBF vs x86_64 changed with Rust 1.77), so the same struct would have two layouts.
//! `[u64; 2]` aligns to 8 everywhere. Low limb first, so the bytes equal `u128::to_le_bytes`.

use bytemuck::{Pod, Zeroable};

/// Unsigned 128-bit, stored as `[low, high]`.
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct U128(pub [u64; 2]);

impl U128 {
    pub const ZERO: Self = Self([0, 0]);

    pub const fn new(v: u128) -> Self {
        Self([v as u64, (v >> 64) as u64])
    }

    pub const fn get(self) -> u128 {
        (self.0[0] as u128) | ((self.0[1] as u128) << 64)
    }

    pub fn set(&mut self, v: u128) {
        *self = Self::new(v);
    }
}

impl From<u128> for U128 {
    fn from(v: u128) -> Self {
        Self::new(v)
    }
}

impl From<U128> for u128 {
    fn from(v: U128) -> Self {
        v.get()
    }
}

impl core::fmt::Debug for U128 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "U128({})", self.get())
    }
}

/// Signed 128-bit, two's complement, stored as `[low, high]`.
#[repr(transparent)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash, Pod, Zeroable)]
pub struct I128(pub [u64; 2]);

impl I128 {
    pub const ZERO: Self = Self([0, 0]);

    pub const fn new(v: i128) -> Self {
        let u = v as u128;
        Self([u as u64, (u >> 64) as u64])
    }

    pub const fn get(self) -> i128 {
        ((self.0[0] as u128) | ((self.0[1] as u128) << 64)) as i128
    }

    pub fn set(&mut self, v: i128) {
        *self = Self::new(v);
    }
}

impl From<i128> for I128 {
    fn from(v: i128) -> Self {
        Self::new(v)
    }
}

impl From<I128> for i128 {
    fn from(v: I128) -> Self {
        v.get()
    }
}

impl core::fmt::Debug for I128 {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "I128({})", self.get())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u128_round_trips_and_bytes_are_le() {
        for v in [0u128, 1, u64::MAX as u128, (u64::MAX as u128) + 1, u128::MAX, 0x0123_4567_89ab_cdef_fedc_ba98_7654_3210] {
            let p = U128::new(v);
            assert_eq!(p.get(), v);
            assert_eq!(bytemuck::bytes_of(&p), &v.to_le_bytes());
        }
    }

    #[test]
    fn i128_round_trips_and_bytes_are_le() {
        for v in [0i128, 1, -1, i128::MIN, i128::MAX, -(1i128 << 64), (1i128 << 64) - 7] {
            let p = I128::new(v);
            assert_eq!(p.get(), v);
            assert_eq!(bytemuck::bytes_of(&p), &v.to_le_bytes());
        }
    }

    #[test]
    fn size_and_align_are_16_and_8() {
        assert_eq!(core::mem::size_of::<U128>(), 16);
        assert_eq!(core::mem::align_of::<U128>(), 8);
        assert_eq!(core::mem::size_of::<I128>(), 16);
        assert_eq!(core::mem::align_of::<I128>(), 8);
    }
}
