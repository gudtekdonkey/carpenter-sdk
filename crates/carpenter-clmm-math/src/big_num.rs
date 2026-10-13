// Derived from raydium-io/raydium-clmm @ ed7c84a5, programs/amm/src/libraries/big_num.rs, Apache-2.0.
// Modified by FLOOR, 2026: `uint` comes from crates.io at =0.9.5 instead of the raydium-io/parity-common
// git fork; the `construct_bignum!` macro and `U1024` are removed, because their only user was
// Raydium's fixed 1024-bit pool bitmap, which the spacing-sized `[u64]` bitmap (`bitmap.rs`) replaces.

//! 128, 256 and 512 bit numbers.
//! U128 is more efficient that u128
//! https://github.com/solana-labs/solana/issues/19549
#![allow(
    clippy::assign_op_pattern,
    clippy::ptr_offset_with_cast,
    clippy::manual_range_contains
)]

use uint::construct_uint;

construct_uint! {
    pub struct U128(2);
}

construct_uint! {
    pub struct U256(4);
}

construct_uint! {
    pub struct U512(8);
}
