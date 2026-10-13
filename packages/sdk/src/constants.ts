/**
 * Constants shared with the `carpenter_amm` program: its id, well-known foreign keys, the AMM's PDA seeds, tick / fee /
 * hook-flag constants, runtime limits and the AMM's `CU_LIMITS` rows. Every value is a copy of the program's Rust
 * constants (`crates/carpenter-types`); a hook's SDK spreads `SEEDS` / `CU_LIMITS` into its own tables.
 */

import { address, type Address } from '@solana/kit';

// ---- program ids ----------------
export const CARPENTER_AMM_PROGRAM_ID: Address = address('DdeiDx4ncHo9DXGZm1GrPe7mFKv7kNU3yx95uQEV2APA');

// ---- well-known foreign keys ----------------------------------------
export const WSOL_MINT: Address = address('So11111111111111111111111111111111111111112');
/** wSOL's `decimals`. NOT a default for "the quote's decimals": it is wSOL's own number, for the
 * paths that PIN wSOL as the quote. A path that takes a quote mint must read that mint's
 * `decimals`. */
export const WSOL_DECIMALS = 9;
export const TOKEN_2022_NATIVE_MINT: Address = address('9pan9bMn5HatX4EJdBwg9VgCa7Uz5HL8N1m5D3NdXejP');
export const SPL_TOKEN_PROGRAM_ID: Address = address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
export const TOKEN_2022_PROGRAM_ID: Address = address('TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb');
export const ASSOCIATED_TOKEN_PROGRAM_ID: Address = address('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL');
export const BPF_LOADER_UPGRADEABLE_ID: Address = address('BPFLoaderUpgradeab1e11111111111111111111111');
export const SYSTEM_PROGRAM_ID: Address = address('11111111111111111111111111111111');
export const COMPUTE_BUDGET_PROGRAM_ID: Address = address('ComputeBudget111111111111111111111111111111');

// ---- seeds ---------------------------------------------------------
const utf8 = (s: string): Uint8Array => new TextEncoder().encode(s);
export const SEEDS = {
  ammConfig: utf8('amm_config'),
  approvedHook: utf8('approved_hook'),
  mintExclusivity: utf8('mint_excl'),
  mintPools: utf8('mint_pools'),
  quoteMint: utf8('quote_mint'),
  pool: utf8('pool'),
  bitmap: utf8('bitmap'),
  tickArray: utf8('tick_array'),
  position: utf8('position'),
  poolIndex: utf8('pool_index'),
  vault: utf8('vault'),
  eventAuthority: utf8('__event_authority'),
  hookAuthority: utf8('hook_authority'),
  /** `["hook_settle", pool]` under `carpenter_amm`: the signer a settling hook's callbacks are made by (`HOOK_FLAGS.SETTLES`). */
  hookSettle: utf8('hook_settle'),
  /**  `["protocol_treasury"]` under `carpenter_amm` —
   * the AMM's OWN program-owned treasury, `crates/carpenter-types/src/seeds.rs::PROTOCOL_TREASURY`. Its
   * ADDRESS owns every token account a pool's `protocol_fees_owed0/1` is swept into (`ATA(treasury, mint,
   * token program)`, opened by the first sweep in that mint); its DATA is the governance that alone
   * withdraws and sets the protocol fee new pools copy. NOT floor_hook's: never `platformTreasury` or
   * `teamTreasury`, and derived under `ids.carpenterAmm`, never `ids.floorHook`. */
  protocolTreasury: utf8('protocol_treasury'),
} as const;

// ---- AMM --------------------
export const MIN_TICK = -443_636;
export const MAX_TICK = 443_636;
export const MIN_SQRT_PRICE_X64 = 4_295_048_016n;
export const MAX_SQRT_PRICE_X64 = 79_226_673_521_066_979_257_578_248_091n;
export const TICK_ARRAY_SIZE = 60;
export const DYNAMIC_FEE_FLAG = 0x80_0000;
export const MAX_LP_FEE = 1_000_000;
export const MAX_SWAP_FEE = 1_000_000;

// ---- THE CARPENTER PLATFORM FEE — v4's protocol fee ----
// `AmmConfig`'s CURRENT SETTING (governance-set, capped) is COPIED into `Pool.protocol_fee` at
// `initialize_pool` and never written into a live pool. Every figure a client shows or quotes uses the
// POOL's own `protocolFee` / `protocolFeeCurrency`, never these defaults; `src/protocolFee.ts` holds the
// arithmetic. Each value mirrors `crates/carpenter-types/src/constants.rs`.
/** The default Carpenter Platform fee, pips PER DIRECTION (500 = 0.05 %), on top of the LP fee. */
export const PROTOCOL_FEE_DEFAULT = 500;
/** The most governance may set (`set_protocol_fee`), pips per direction (0.1 %); above it `ProtocolFeeTooLarge`. 0 is allowed. */
export const MAX_PROTOCOL_FEE = 1_000;
/** `PROTOCOL_FEE_DEFAULT` in both directions, v4-packed `zero_for_one | one_for_zero << 12`. */
export const PROTOCOL_FEE_DEFAULT_PACKED = 2_048_500;
/** v4's pips denominator. */
export const PIPS_DENOMINATOR = 1_000_000;
/** `Pool.protocol_fee_currency` 0: v4's rule, the fee is charged in the swap's INPUT (no listed quote side, or two listed quotes neither of which is wSOL). */
export const PROTOCOL_FEE_ON_INPUT = 0;
/** `Pool.protocol_fee_currency` 1: currency 0 is the pool's one listed quote — charged in it on buys AND sells. Every FLOOR pool. */
export const PROTOCOL_FEE_ON_CURRENCY0 = 1;
/** `Pool.protocol_fee_currency` 2: currency 1 is the pool's one listed quote. */
export const PROTOCOL_FEE_ON_CURRENCY1 = 2;

export const HOOK_FLAGS = {
  BEFORE_INITIALIZE: 1 << 13,
  AFTER_INITIALIZE: 1 << 12,
  BEFORE_ADD_LIQUIDITY: 1 << 11,
  AFTER_ADD_LIQUIDITY: 1 << 10,
  BEFORE_REMOVE_LIQUIDITY: 1 << 9,
  AFTER_REMOVE_LIQUIDITY: 1 << 8,
  BEFORE_SWAP: 1 << 7,
  AFTER_SWAP: 1 << 6,
  BEFORE_DONATE: 1 << 5,
  AFTER_DONATE: 1 << 4,
  BEFORE_SWAP_RETURNS_DELTA: 1 << 3,
  AFTER_SWAP_RETURNS_DELTA: 1 << 2,
  AFTER_ADD_LIQUIDITY_RETURNS_DELTA: 1 << 1,
  AFTER_REMOVE_LIQUIDITY_RETURNS_DELTA: 1 << 0,
  REQUESTS_ACTIONS: 1 << 14,
  /**
   * Solana-only, Uniswap v4's settle: the hook is called under its own signer `["hook_settle", pool]` (`findHookSettle`,
   * the FIRST account of its slice), is handed both vaults and both mints in the swap callbacks so it can mint into a
   * vault, may cap the trader's walk with a price limit from `before_swap`, may take the unfilled remainder of an exact-in
   * from `after_swap`, and may pay the trader out of its claims (negative deltas). Requires BEFORE_SWAP, AFTER_SWAP and
   * AFTER_SWAP_RETURNS_DELTA.
   */
  SETTLES: 1 << 15,
} as const;

/**
 * `QuoteMint.state`: Listed, Retired or Hook-listed, NEVER 0 — a zeroed account of the entry's
 * length reads as ABSENT, and a delisted entry is CLOSED rather than zeroed. Mirrors
 * `crates/carpenter-types/src/layout.rs`'s `QUOTE_STATE_*`.
 */
export const QUOTE_STATE_LISTED = 1;
export const QUOTE_STATE_RETIRED = 2;
/** A quote only for pools of the hook in `QuoteMint.hookProgram`, listed by that hook. */
export const QUOTE_STATE_HOOK_LISTED = 3;

/**
 * The capability bitmap the program's mint gate records on a roster entry (`QuoteMint.caps`,
 * `QuoteMintListed.caps`) — what the mint's ISSUER can do to our vault, read off the mint's TLV at
 * listing. It gates nothing by itself: the Tier-B bits (`QUOTE_CAPS_CUSTODIAL`) are compared against
 * the governance-set admission mask (`AmmConfig.quote_caps_admitted`), the Tier-A bits
 * (`QUOTE_CAPS_INERT`) are recorded and never refused. The values mirror the program's mint gate.
 * Issuer-custodied quotes are ACCEPTED: these names RECORD the issuer's power over a vault, they do not refuse it.
 */
export const QUOTE_CAP_FREEZE = 1 << 0;
export const QUOTE_CAP_PERMANENT_DELEGATE = 1 << 1;
export const QUOTE_CAP_PAUSABLE = 1 << 2;
export const QUOTE_CAP_DEFAULT_STATE = 1 << 3;
export const QUOTE_CAP_HOOK_SLOT = 1 << 4;
export const QUOTE_CAP_SCALED_UI = 1 << 5;
export const QUOTE_CAP_INTEREST = 1 << 6;
export const QUOTE_CAP_CONFIDENTIAL = 1 << 7;
/** Every Tier-B bit: admitted ONLY in the listed-quote role, and only under the governance mask. */
export const QUOTE_CAPS_CUSTODIAL = QUOTE_CAP_FREEZE | QUOTE_CAP_PERMANENT_DELEGATE | QUOTE_CAP_PAUSABLE | QUOTE_CAP_DEFAULT_STATE | QUOTE_CAP_HOOK_SLOT;
/** Every Tier-A bit: recorded, never refused, never subject to the mask. */
export const QUOTE_CAPS_INERT = QUOTE_CAP_SCALED_UI | QUOTE_CAP_INTEREST | QUOTE_CAP_CONFIDENTIAL;
/** The bit names, for a readout (a mint's `caps` as words). */
export const QUOTE_CAP_NAMES: ReadonlyArray<readonly [bit: number, name: string]> = [
  [QUOTE_CAP_FREEZE, 'FREEZE'], [QUOTE_CAP_PERMANENT_DELEGATE, 'PERMANENT_DELEGATE'], [QUOTE_CAP_PAUSABLE, 'PAUSABLE'],
  [QUOTE_CAP_DEFAULT_STATE, 'DEFAULT_STATE'], [QUOTE_CAP_HOOK_SLOT, 'HOOK_SLOT'], [QUOTE_CAP_SCALED_UI, 'SCALED_UI'],
  [QUOTE_CAP_INTEREST, 'INTEREST'], [QUOTE_CAP_CONFIDENTIAL, 'CONFIDENTIAL'],
];

/** Solana runtime limits. */
export const MAX_ACCOUNT_LOCKS = 64;
export const MAX_TX_CU = 1_400_000;
export const LEGACY_TX_BYTES = 1_232;

/**
 * `SetComputeUnitLimit` values: each is an estimate (`[est.]`) or a measured figure × 1.5, capped at the 1.4 M
 * per-transaction limit.
 * Every builder prepends one: an unset limit is 0 under v1 transactions.
 */
export const CU_LIMITS = {
  swapPlain: 90_000, // 60k [est.] × 1.5
  modifyLiquidity: 90_000, // 60k [est.] × 1.5 — the instruction alone: buildModifyLiquidity adds one `ataCreate` per side
  initializePool: 120_000, // 80k [est.] × 1.5
  initializeTickArray: 30_000, // a single Anchor init [est.]
  closePosition: 20_000, // [est.]
  // ONE `close_tick_array` — a PDA derivation, the bitmap check, a 3,840-byte zero scan, an Anchor close and emit_cpi.
  // `buildCloseTickArrays` sends N × this.
  closeTickArray: 30_000, // [est.]
  /** A wallet-to-wallet `transfer_checked` of a hooked mint: Token-2022 + the `execute` CPI. Measured 39,115 CU for
   * the bare transfer, both sides recorded (79,953 with the receiver's ATA create and `touch`); × 1.5 = 58,673. The
   * 80,000 [est.] is kept as the ceiling. */
  hookedTransfer: 80_000,
  /** One ATA `CreateIdempotent` a composed transaction carries beside its main instruction [est.]. */
  ataCreate: 30_000,
  /** A `BurnChecked` alone [est.]. */
  burnChecked: 20_000,
} as const;
export type CuLimitKey = keyof typeof CU_LIMITS;

/**
 * `RequestHeapFrame` sizes, in bytes, per `CU_LIMITS` key (a multiple of 1,024, from 32 KiB to 256 KiB). No
 * `carpenter_amm` builder requests one: every AMM instruction runs on the default 32 KiB heap. A hook's SDK that
 * needs a frame for its own builders keeps its own table (`@floorsupply/sdk-solana` does, for the ratchet).
 */
export const HEAP_FRAMES: Readonly<Partial<Record<CuLimitKey, number>>> = {};
