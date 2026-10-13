//! Well-known foreign keys, as raw bytes (no Solana crate). Each is checked against the Solana
//! crates' own constant in the program repository's tests.

/// The wSOL mint, `So11111111111111111111111111111111111111112`. FLOOR pools are
/// `(wSOL = currency0, FLOOR mint = currency1)`, so every FLOOR mint must sort above it.
pub const WSOL_MINT: [u8; 32] = [
    6, 155, 136, 87, 254, 171, 129, 132, 251, 104, 127, 99, 70, 24, 192, 53, 218, 196, 57, 220, 26,
    235, 59, 85, 152, 160, 240, 0, 0, 0, 0, 1,
];

/// Token-2022's native mint, `9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP`. Refused by the
/// AMM's mint allowlist.
pub const TOKEN_2022_NATIVE_MINT: [u8; 32] = [
    131, 13, 252, 159, 222, 95, 230, 184, 170, 124, 4, 164, 118, 233, 30, 138, 198, 187, 38, 74,
    173, 144, 250, 25, 201, 223, 73, 216, 92, 62, 91, 94,
];

/// SPL Token, `TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA`.
pub const SPL_TOKEN_PROGRAM_ID: [u8; 32] = [
    6, 221, 246, 225, 215, 101, 161, 147, 217, 203, 225, 70, 206, 235, 121, 172, 28, 180, 133, 237,
    95, 91, 55, 145, 58, 140, 245, 133, 126, 255, 0, 169,
];

/// Token-2022, `TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb`.
pub const TOKEN_2022_PROGRAM_ID: [u8; 32] = [
    6, 221, 246, 225, 238, 117, 143, 222, 24, 66, 93, 188, 228, 108, 205, 218, 182, 26, 252, 77,
    131, 185, 13, 39, 254, 189, 249, 40, 216, 161, 139, 252,
];

/// The Associated Token Account program, `ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL`.
pub const ASSOCIATED_TOKEN_PROGRAM_ID: [u8; 32] = [
    140, 151, 37, 143, 78, 36, 137, 241, 187, 61, 16, 41, 20, 142, 13, 131, 11, 90, 19, 153, 218,
    255, 16, 132, 4, 142, 123, 216, 219, 233, 248, 89,
];

/// `BPFLoaderUpgradeab1e11111111111111111111111`: the only loader an approved hook may use
/// (loader-v4 is withdrawn).
pub const BPF_LOADER_UPGRADEABLE_ID: [u8; 32] = [
    2, 168, 246, 145, 78, 136, 161, 176, 226, 16, 21, 62, 247, 99, 174, 43, 0, 194, 185, 61, 22,
    193, 36, 210, 192, 83, 122, 16, 4, 128, 0, 0,
];

/// The System program, `11111111111111111111111111111111`.
pub const SYSTEM_PROGRAM_ID: [u8; 32] = [0; 32];
