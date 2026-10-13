/**
 * Every `carpenter_amm` account, decoded from its bytes, and the shared codec (`decodeAccount` / `encodeAccount`,
 * `FieldKind`, `LayoutDef`) a hook's SDK builds its own accounts on.
 *
 * The field tables are transcriptions of the program's `layout!` lists: STRUCT offsets, discriminator outside.
 * Account offset = struct offset + 8. 128-bit fields are two LE u64 limbs (low first).
 * Decoders check the discriminator AND the length; a wrong account is an error, never zeros.
 */

import type { Address } from '@solana/kit';

import { addressToBytes, bytesToAddress, equalBytes, readI128LE, readUintLE, toTwos128, writeUintLE } from './bytes.ts';

export type FieldKind =
  | { t: 'u8' }
  | { t: 'u16' }
  | { t: 'u32' }
  | { t: 'i32' }
  | { t: 'u64' }
  // A signed 64-bit field (e.g. a unix timestamp such as `GenesisSale.open_time`), decoded as a bigint so it
  // sits beside other 64-bit fields without a unit change in arithmetic.
  | { t: 'i64' }
  | { t: 'u128' }
  | { t: 'i128' }
  | { t: 'pubkey' }
  | { t: 'pubkeys'; n: number }
  | { t: 'bytes'; n: number }
  | { t: 'ticks'; n: number }
  // `StairState`'s per-step arrays, `[i32; 16]` and `[U128; 16]`.
  | { t: 'i32s'; n: number }
  | { t: 'u128s'; n: number }
  // `Launch.team_vest_days`, `[u16; 4]`.
  | { t: 'u16s'; n: number }
  // `StairOwners.tokens` / `.cut`, `[u64; 16]`.
  | { t: 'u64s'; n: number };
export interface FieldDef {
  /** The Rust field name (snake_case). The decoded key is its camelCase form. */
  readonly name: string;
  readonly offset: number;
  readonly kind: FieldKind;
}

export interface LayoutDef {
  /** The Anchor account name. */
  readonly name: string;
  readonly size: number;
  /** `sha256("account:<name>")[..8]`. */
  readonly discriminator: Uint8Array;
  readonly fields: readonly FieldDef[];
  /** True for the bitmap, whose words follow the fixed struct, and `FloorPool`, whose TWAP tail does. */
  readonly trailing?: boolean;
}

export function kindSize(k: FieldKind): number {
  switch (k.t) {
    case 'u8':
      return 1;
    case 'u16':
      return 2;
    case 'u32':
    case 'i32':
      return 4;
    case 'u64':
    case 'i64':
      return 8;
    case 'u128':
    case 'i128':
      return 16;
    case 'pubkey':
      return 32;
    case 'pubkeys':
      return 32 * k.n;
    case 'bytes':
      return k.n;
    case 'ticks':
      return 64 * k.n;
    case 'i32s':
      return 4 * k.n;
    case 'u128s':
      return 16 * k.n;
    case 'u16s':
      return 2 * k.n;
    case 'u64s':
      return 8 * k.n;
  }
}

const u8 = { t: 'u8' } as const;
const u16 = { t: 'u16' } as const;
const u32 = { t: 'u32' } as const;
const i32 = { t: 'i32' } as const;
const u64 = { t: 'u64' } as const;
const i64 = { t: 'i64' } as const;
const u128 = { t: 'u128' } as const;
const pk = { t: 'pubkey' } as const;
const bytes = (n: number): FieldKind => ({ t: 'bytes', n });
const d = (...b: number[]): Uint8Array => new Uint8Array(b);

type Row = readonly [string, number, FieldKind];
function def(name: string, size: number, discriminator: Uint8Array, rows: readonly Row[], trailing = false): LayoutDef {
  return { name, size, discriminator, trailing, fields: rows.map(([n, o, kind]) => ({ name: n, offset: o, kind })) };
}

// ---- carpenter_amm -------------------------------------------------------------------------

export const AMM_CONFIG = def('AmmConfig', 208, d(0xda, 0xf4, 0x21, 0x68, 0xcb, 0xcb, 0x2b, 0x6f), [
  ['version', 0, u8], ['bump', 1, u8], ['pad0', 2, bytes(6)],
  ['authority', 8, pk], ['pending_authority', 40, pk], ['protocol_fee_controller', 72, pk],
  // The pause guardian — the key that pauses one pool / one hook's new pools for at most 72 h
  // (`guardian_pause_pool` / `_hook`); zero = none. `pad1` is unused (wSOL is the implied quote).
  ['pause_guardian', 104, pk], ['pad1', 136, bytes(40)],
  // The governance ADMISSION MASK for listed quotes (`QUOTE_CAP_*` bits, `set_quote_caps_admitted`);
  // 0 = nothing admitted.
  ['quote_caps_admitted', 176, u8],
  // `protocol_fee`: the CURRENT SETTING new pools copy, read AS WRITTEN — `initialize_config` always
  // writes it (default 500 | 500 << 12). `pad2` is padding.
  ['pad2', 177, bytes(3)], ['protocol_fee', 180, u32], ['reserved', 184, bytes(24)],
]);

export const APPROVED_HOOK = def('ApprovedHook', 192, d(0x1b, 0x9b, 0x38, 0x21, 0xb0, 0xad, 0xe1, 0x0f), [
  ['version', 0, u8], ['bump', 1, u8], ['paused', 2, u8], ['max_rounds', 3, u8], ['callback_accounts', 4, u8],
  ['allow_public_pools', 5, u8], ['flags', 6, u16], ['hook_program', 8, pk], ['hook_authority', 40, pk],
  ['hook_authority_bump', 72, u8], ['pad0', 73, bytes(7)], ['programdata_slot', 80, u64], ['approved_slot', 88, u64],
  ['name', 96, bytes(32)], ['allow_exclusivity', 128, u8],
  // The ONE Token-2022 transfer-hook PROGRAM a pool of this approved hook may carry ARMED on its BASE
  // mint — zero = none. Set ONCE by the config authority (`set_base_transfer_hook`); `reapprove_hook`
  // preserves it.
  ['base_transfer_hook', 129, pk],
  // The config authority's switch (`set_hook_quotes`) letting THIS hook list quotes
  // for its own pools (`hook_list_quote_mint`).
  ['allow_hook_quotes', 161, u8], ['reserved', 162, bytes(22)],
  // When a guardian's pause of this hook's NEW pools ends (unix s; 0 = never). `initialize_pool` (so every
  // graduation that creates a pool) is refused while `now < it`.
  ['guardian_paused_until', 184, i64],
]);

// `has_protocol_fee` / `protocol_fee`: the Carpenter Platform fee recorded at `register_mint_exclusivity` — for a FLOOR launch, at
// `create_launch` — which its pool charges at graduation. Read via `launchProtocolFee`.
export const MINT_EXCLUSIVITY = def('MintExclusivity', 72, d(0x3a, 0x90, 0x07, 0x22, 0x47, 0xe3, 0x02, 0x37), [
  ['mint', 0, pk], ['hook_program', 32, pk], ['bump', 64, u8], ['has_protocol_fee', 65, u8], ['pad0', 66, bytes(2)],
  ['protocol_fee', 68, u32],
]);

export const MINT_POOL_COUNT = def('MintPoolCount', 56, d(0xd7, 0x9e, 0xe2, 0xc8, 0x59, 0x05, 0x63, 0x6c), [
  ['mint', 0, pk], ['count', 32, u64], ['bump', 40, u8], ['reserved', 41, bytes(15)],
]);

// The quote ROSTER entry. `state` is 1 (Listed), 2 (Retired) or 3 (Hook-listed) and NEVER 0:
// a zeroed account of this length must read as ABSENT, and a delisted entry is CLOSED rather than
// zeroed.
export const QUOTE_MINT = def('QuoteMint', 120, d(0xf9, 0xdf, 0x0c, 0xd4, 0x7d, 0x1c, 0xb9, 0x67), [
  ['mint', 0, pk], ['authority_digest', 32, bytes(32)], ['listed_slot', 64, u64], ['retired_slot', 72, u64],
  ['state', 80, u8], ['bump', 81, u8], ['decimals', 82, u8], ['caps', 83, u8], ['pad0', 84, bytes(4)],
  // State 3 = HOOK-LISTED, a quote only for pools of `hook_program`'s hook (zero on
  // every governance listing).
  ['hook_program', 88, pk],
]);

export const POOL = def('Pool', 632, d(0xf1, 0x9a, 0x6d, 0x04, 0x11, 0xb1, 0x6d, 0xbc), [
  ['version', 0, u8], ['bump', 1, u8], ['hook_flags', 2, u16], ['tick_spacing', 4, u16],
  ['hook_max_rounds', 6, u8], ['hook_callback_accounts', 7, u8], ['fee', 8, u32], ['lp_fee', 12, u32],
  ['protocol_fee', 16, u32], ['tick_current', 20, i32], ['mint0', 24, pk], ['mint1', 56, pk],
  ['hook', 88, pk], ['hook_authority', 120, pk], ['vault0', 152, pk], ['vault1', 184, pk],
  ['token_program0', 216, pk], ['token_program1', 248, pk], ['sqrt_price_x64', 280, u128],
  ['liquidity', 296, u128], ['fee_growth_global0_x64', 312, u128], ['fee_growth_global1_x64', 328, u128],
  ['protocol_fees_owed0', 344, u64], ['protocol_fees_owed1', 352, u64], ['hook_claims0', 360, u64],
  ['hook_claims1', 368, u64], ['volume0', 376, u128], ['volume1', 392, u128], ['lp_fees0', 408, u128],
  ['lp_fees1', 424, u128], ['hook_fees0', 440, u128], ['hook_fees1', 456, u128], ['swap_count', 472, u64],
  ['created_slot', 480, u64], ['decimals0', 488, u8], ['decimals1', 489, u8], ['bitmap_bump', 490, u8],
  ['vault0_bump', 491, u8], ['vault1_bump', 492, u8], ['pad0', 493, bytes(3)], ['reserved0', 496, u64],
  // `paused`: the config authority's per-pool pause (`set_pool_paused`).
  ['paused', 504, u8],
  // The admitted base transfer hook, copied ONCE from `ApprovedHook.base_transfer_hook` at
  // `initialize_pool` (zero = none), and the side it applies to. The all-zero key means none.
  ['base_transfer_hook', 505, pk], ['base_hook_side', 537, u8],
  // The currency this pool's `protocol_fee` is charged in —
  // `PROTOCOL_FEE_ON_INPUT` 0 (v4's input rule), `_ON_CURRENCY0` 1 / `_ON_CURRENCY1` 2
  // (the pool's one listed quote, on buys AND sells). Set once at `initialize_pool`; read via `protocolFeeOf`.
  ['protocol_fee_currency', 538, u8], ['reserved', 539, bytes(85)],
  // When a guardian's pause of this pool ends (unix s; 0 = never).
  // Paused = `paused != 0 || now < guardianPausedUntil` — read through `poolPausedAt`, never one alone.
  ['guardian_paused_until', 624, i64],
]);

export const TICK_ARRAY_BITMAP = def(
  'TickArrayBitmap',
  40,
  d(0x21, 0xfb, 0x69, 0xf4, 0x05, 0xfc, 0x9b, 0xce),
  [['pool', 0, pk], ['tick_spacing', 32, u16], ['pad0', 34, u16], ['arrays_per_side', 36, u32]],
  true,
);

export const TICK_ARRAY = def('TickArray', 3880, d(0x45, 0x61, 0xbd, 0xbe, 0x6e, 0x07, 0x42, 0xbb), [
  ['pool', 0, pk], ['start_index', 32, i32], ['initialized_tick_count', 36, u16], ['pad0', 38, u16],
  ['ticks', 40, { t: 'ticks', n: 60 }],
]);

// The `TickArray` STRUCT is 3,880 B (the table above); the ACCOUNT carries its RENT OWNER as 32 raw bytes AFTER the
// body — not a struct field, not in the IDL:
//   body (V1)  = 3,888 B — discriminator + struct.
//   account (V2) = 3,920 B — the body, then `rent_owner` at account offset 3,888: the only signer that may
//                  `close_tick_array` it (the payer by default).
/** The body only (`8 + TICK_ARRAY.size`) — 3,888. The programs do not accept an account of this length. */
export const TICK_ARRAY_LEN_V1: number = 8 + TICK_ARRAY.size;
/** Account offset of the 32-byte rent owner — which is also where the body ends. 3,888. */
export const TICK_ARRAY_RENT_OWNER_OFFSET: number = TICK_ARRAY_LEN_V1;
/** Every tick array created: the body, then the rent owner — 3,920. */
export const TICK_ARRAY_LEN_V2: number = TICK_ARRAY_RENT_OWNER_OFFSET + 32;
/**
 * THE one length the programs accept —
 * `carpenter_types::layout::TICK_ARRAY_LEN`, 3,920 (a 3,888-byte V1 account is refused `SeedsMismatch`). The V1 / V2 names
 * stay for the body / tail arithmetic and for readers; an off-chain QUOTER still takes the 3,888-byte BODY.
 */
export const TICK_ARRAY_LEN: number = TICK_ARRAY_LEN_V2;

// `owner` is a FIELD and `position_id` the third seed
// (`["position", pool, position_id, salt]`), re-checked as a FIELD on every modify — which is what
// lets a position be transferred without its address moving.
export const POSITION = def('Position', 200, d(0xaa, 0xbc, 0x8f, 0xe4, 0x7a, 0x40, 0xf7, 0xd0), [
  ['version', 0, u8], ['bump', 1, u8], ['pad0', 2, bytes(6)], ['pool', 8, pk], ['owner', 40, pk],
  ['salt', 72, u64], ['tick_lower', 80, i32], ['tick_upper', 84, i32], ['liquidity', 88, u128],
  ['fee_growth_inside0_last_x64', 104, u128], ['fee_growth_inside1_last_x64', 120, u128],
  ['position_id', 136, pk], ['reserved', 168, bytes(32)],
]);

export const POOL_INDEX_ENTRY = def('PoolIndexEntry', 48, d(0x7a, 0xdd, 0x11, 0x66, 0xaf, 0x30, 0xb3, 0xf2), [
  ['pool', 0, pk], ['created_slot', 32, u64], ['bump', 40, u8], ['reserved', 41, bytes(7)],
]);

/**
 * `["protocol_treasury"]` under `carpenter_amm`: the AMM's own program-owned treasury: its ADDRESS owns every token account a pool's `protocol_fees_owed0/1` is
 * swept into (`sweep_protocol_fees`), its DATA the governance that alone withdraws
 * (`withdraw_protocol_fees`, uncapped), sets the protocol fee new pools copy (`set_protocol_fee`) and hands
 * itself over in two steps (`carpenter_types::layout::ProtocolTreasury`).
 */
export const PROTOCOL_TREASURY = def('ProtocolTreasury', 104, d(0xa2, 0x1a, 0x7b, 0x3d, 0x66, 0x92, 0x2f, 0x49), [
  ['version', 0, u8], ['bump', 1, u8], ['pad0', 2, bytes(6)], ['governance', 8, pk], ['pending_governance', 40, pk],
  ['reserved', 72, bytes(32)],
]);

/** Every `carpenter_amm` account layout (`@floorsupply/sdk-solana`'s `LAYOUTS` appends FLOOR's). */
export const LAYOUTS: readonly LayoutDef[] = [
  AMM_CONFIG, APPROVED_HOOK, MINT_EXCLUSIVITY, MINT_POOL_COUNT, QUOTE_MINT, POOL, TICK_ARRAY_BITMAP, TICK_ARRAY, POSITION,
  POOL_INDEX_ENTRY,

  PROTOCOL_TREASURY,
];

// ---- decoded shapes ---------------------------------------------------------------------------

export interface Tick {
  liquidityGross: bigint;
  liquidityNet: bigint;
  feeGrowthOutside0X64: bigint;
  feeGrowthOutside1X64: bigint;
}

export interface AmmConfig {
  version: number; bump: number; authority: Address; pendingAuthority: Address; protocolFeeController: Address;
  /** The pause guardian; the all-zero key = none. */
  pauseGuardian: Address;
  /** The `QUOTE_CAP_*` bits governance admits for a LISTED quote; 0 = none. */
  quoteCapsAdmitted: number;
  /** The v4-packed setting NEW pools copy, read as written. */
  protocolFee: number;
  /**
   * DERIVED by `decodeAmmConfig` (not an account field): the v4-packed Carpenter Platform fee a pool created
   * NOW copies — `protocolFee` itself, exposed under this name for readers.
   * What NEW pools pay; a live pool's rate is its own `Pool.protocolFee`.
   */
  currentProtocolFee: number;
}
export interface ApprovedHook {
  version: number; bump: number; paused: number; maxRounds: number; callbackAccounts: number; allowPublicPools: number;
  flags: number; hookProgram: Address; hookAuthority: Address; hookAuthorityBump: number; programdataSlot: bigint;
  approvedSlot: bigint; name: Uint8Array; allowExclusivity: number;
  /** RAW — the all-zero key means NONE. */
  baseTransferHook: Address;
  /** 1 while this hook may list quotes for its own pools (`set_hook_quotes`). */
  allowHookQuotes: number;
  /** When the guardian's pause of NEW pools ends, unix seconds; 0 = never paused by it. */
  guardianPausedUntil: bigint;
}
export interface MintExclusivity {
  mint: Address; hookProgram: Address; bump: number;
  /** 1 once `register_mint_exclusivity` recorded `protocolFee`. */
  hasProtocolFee: number;
  /** The Carpenter Platform fee (v4-packed) recorded at registration — a FLOOR launch's pool charges THIS. */
  protocolFee: number;
}
export interface MintPoolCount { mint: Address; count: bigint; bump: number }
/** `state`: 1 Listed, 2 Retired, 3 Hook-listed. `caps` is the `QUOTE_CAP_*` bitmap. */
export interface QuoteMint {
  mint: Address; authorityDigest: Uint8Array; listedSlot: bigint; retiredSlot: bigint;
  state: number; bump: number; decimals: number; caps: number;
  /** State 3 (hook-listed) — the hook whose pools alone may use this quote; zero on a governance listing. */
  hookProgram: Address;
}
export interface Pool {
  version: number; bump: number; hookFlags: number; tickSpacing: number; hookMaxRounds: number; hookCallbackAccounts: number;
  fee: number; lpFee: number; protocolFee: number; tickCurrent: number; mint0: Address; mint1: Address; hook: Address;
  hookAuthority: Address; vault0: Address; vault1: Address; tokenProgram0: Address; tokenProgram1: Address;
  sqrtPriceX64: bigint; liquidity: bigint; feeGrowthGlobal0X64: bigint; feeGrowthGlobal1X64: bigint;
  protocolFeesOwed0: bigint; protocolFeesOwed1: bigint; hookClaims0: bigint; hookClaims1: bigint; volume0: bigint;
  volume1: bigint; lpFees0: bigint; lpFees1: bigint; hookFees0: bigint; hookFees1: bigint; swapCount: bigint;
  createdSlot: bigint; decimals0: number; decimals1: number; bitmapBump: number; vault0Bump: number; vault1Bump: number;
  paused: number;
  /** RAW — the all-zero key means NONE (no base transfer hook admitted). */
  baseTransferHook: Address;
  /** The side (0 / 1) `baseTransferHook` applies to. Meaningless while it is zero. */
  baseHookSide: number;
  /**
   * The currency code the pool's `protocolFee` is charged in
   * (`PROTOCOL_FEE_ON_*`). `protocolFee` (v4-packed pips, COPIED at creation) and `protocolFeesOwed0/1` (owed, swept by `sweep_protocol_fees`) are above.
   * Read a direction's rate through `protocolFeeOf(pool, zeroForOne)`.
   */
  protocolFeeCurrency: number;
  /** When the guardian's pause of this pool ends, unix seconds; 0 = never. */
  guardianPausedUntil: bigint;
}
export interface TickArrayBitmap { pool: Address; tickSpacing: number; arraysPerSide: number; words: bigint[] }
export interface TickArray {
  pool: Address; startIndex: number; initializedTickCount: number; ticks: Tick[];
  /**
   * The signer the array's rent (`close_tick_array`) is refunded to on close — `null` on an
   * array whose tail is all-zero. Read by
   * {@link decodeTickArray} / {@link tickArrayRentOwner}; never a struct field.
   */
  rentOwner: Address | null;
}
export interface Position {
  version: number; bump: number; pool: Address; owner: Address; salt: bigint; tickLower: number; tickUpper: number;
  liquidity: bigint; feeGrowthInside0LastX64: bigint; feeGrowthInside1LastX64: bigint; positionId: Address;
}
export interface PoolIndexEntry { pool: Address; createdSlot: bigint; bump: number }
/**
 * `carpenter_amm`'s `["protocol_treasury"]`. `governance`
 * alone signs `withdraw_protocol_fees`, `set_protocol_fee` and the handover's first step; `pendingGovernance`
 * is the proposed key, or the zero key — `11111111111111111111111111111111` — when nothing is pending.
 */
export interface ProtocolTreasury { version: number; bump: number; governance: Address; pendingGovernance: Address }

// ---- the decoder ------------------------------------------------------------------------------

export function snakeToCamel(s: string): string {
  return s.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());
}

/** Padding and reserved bytes are not surfaced. */
const HIDDEN = /^(pad\d*|reserved\d*)$/;

/** One field of `kind` at `at` in `data` — the codec's reader, exported for a hook SDK's tail decoders. */
export function readField(data: Uint8Array, at: number, kind: FieldKind): unknown {
  switch (kind.t) {
    case 'u8':
      return data[at] as number;
    case 'u16':
      return Number(readUintLE(data, at, 2));
    case 'u32':
      return Number(readUintLE(data, at, 4));
    case 'i32': {
      const u = Number(readUintLE(data, at, 4));
      return u >= 2 ** 31 ? u - 2 ** 32 : u;
    }
    case 'u64':
      return readUintLE(data, at, 8);
    case 'i64':
      return BigInt.asIntN(64, readUintLE(data, at, 8));
    case 'u128':
      return readUintLE(data, at, 16);
    case 'i128':
      return readI128LE(data, at);
    case 'pubkey':
      return bytesToAddress(data.slice(at, at + 32));
    case 'pubkeys':
      return Array.from({ length: kind.n }, (_, i) => bytesToAddress(data.slice(at + 32 * i, at + 32 * i + 32)));
    case 'bytes':
      return data.slice(at, at + kind.n);
    case 'i32s':
      return Array.from({ length: kind.n }, (_, i) => readField(data, at + 4 * i, { t: 'i32' }) as number);
    case 'u128s':
      return Array.from({ length: kind.n }, (_, i) => readUintLE(data, at + 16 * i, 16));
    case 'u16s':
      return Array.from({ length: kind.n }, (_, i) => Number(readUintLE(data, at + 2 * i, 2)));
    case 'u64s':
      return Array.from({ length: kind.n }, (_, i) => readUintLE(data, at + 8 * i, 8));
    case 'ticks':
      return Array.from({ length: kind.n }, (_, i): Tick => {
        const o = at + 64 * i;
        return {
          liquidityGross: readUintLE(data, o, 16),
          liquidityNet: readI128LE(data, o + 16),
          feeGrowthOutside0X64: readUintLE(data, o + 32, 16),
          feeGrowthOutside1X64: readUintLE(data, o + 48, 16),
        };
      });
  }
}

export class AccountDecodeError extends Error {}

/** Decode a whole account (discriminator included) against a layout. */
export function decodeAccount<T>(layout: LayoutDef, data: Uint8Array): T {
  const min = 8 + layout.size;
  if (layout.trailing ? data.length < min : data.length !== min) {
    throw new AccountDecodeError(`${layout.name}: account length ${data.length}, expected ${layout.trailing ? '≥ ' : ''}${min}`);
  }
  if (!equalBytes(data.subarray(0, 8), layout.discriminator)) {
    throw new AccountDecodeError(`${layout.name}: discriminator mismatch`);
  }
  const out: Record<string, unknown> = {};
  for (const f of layout.fields) {
    if (HIDDEN.test(f.name)) continue;
    out[snakeToCamel(f.name)] = readField(data, 8 + f.offset, f.kind);
  }
  return out as T;
}

/**
 * Encode a whole account (discriminator included) from camelCase fields; unset fields, padding
 * and reserved bytes are zero. The inverse of `decodeAccount` — for fixtures, mocks and
 * tests. `words` (the bitmap's trailing u64s) is written after the struct when given.
 */
export function encodeAccount(layout: LayoutDef, fields: Record<string, unknown>, words?: readonly bigint[]): Uint8Array {
  const trailing = layout.trailing ? 8 * (words?.length ?? 0) : 0;
  const out = new Uint8Array(8 + layout.size + trailing);
  out.set(layout.discriminator, 0);
  const given = new Set(Object.keys(fields));
  for (const f of layout.fields) {
    const key = snakeToCamel(f.name);
    const v = fields[key];
    given.delete(key);
    if (v === undefined) continue;
    if (HIDDEN.test(f.name)) throw new AccountDecodeError(`${layout.name}.${key} is padding`);
    writeField(out, 8 + f.offset, f.kind, v, `${layout.name}.${key}`);
  }
  if (given.size) throw new AccountDecodeError(`${layout.name}: unknown field(s) ${[...given].join(', ')}`);
  if (layout.trailing && words) words.forEach((w, i) => writeUintLE(out, 8 + layout.size + 8 * i, 8, w));
  return out;
}

function writeField(out: Uint8Array, at: number, kind: FieldKind, v: unknown, what: string): void {
  const big = (x: unknown): bigint => {
    if (typeof x === 'bigint') return x;
    if (typeof x === 'number' && Number.isInteger(x)) return BigInt(x);
    throw new AccountDecodeError(`${what}: expected an integer, got ${typeof x}`);
  };
  const key = (x: unknown): Uint8Array => {
    if (typeof x !== 'string') throw new AccountDecodeError(`${what}: expected an address`);
    return addressToBytes(x as Address);
  };
  switch (kind.t) {
    case 'u8':
      return writeUintLE(out, at, 1, big(v));
    case 'u16':
      return writeUintLE(out, at, 2, big(v));
    case 'u32':
      return writeUintLE(out, at, 4, big(v));
    case 'i32': {
      const x = big(v);
      if (x < -(1n << 31n) || x >= 1n << 31n) throw new AccountDecodeError(`${what}: i32 out of range`);
      return writeUintLE(out, at, 4, x < 0n ? x + (1n << 32n) : x);
    }
    case 'u64':
      return writeUintLE(out, at, 8, big(v));
    case 'i64': {
      const x = big(v);
      if (x < -(1n << 63n) || x >= 1n << 63n) throw new AccountDecodeError(`${what}: i64 out of range`);
      return writeUintLE(out, at, 8, BigInt.asUintN(64, x));
    }
    case 'u128':
      return writeUintLE(out, at, 16, big(v));
    case 'i128':
      return writeUintLE(out, at, 16, toTwos128(big(v)));
    case 'pubkey':
      return out.set(key(v), at);
    case 'pubkeys': {
      if (!Array.isArray(v) || v.length > kind.n) throw new AccountDecodeError(`${what}: expected ≤ ${kind.n} addresses`);
      v.forEach((a, i) => out.set(key(a), at + 32 * i));
      return;
    }
    case 'bytes': {
      if (!(v instanceof Uint8Array) || v.length !== kind.n) throw new AccountDecodeError(`${what}: expected ${kind.n} bytes`);
      return out.set(v, at);
    }
    case 'i32s':
    case 'u16s':
    case 'u64s':
    case 'u128s': {
      if (!Array.isArray(v) || v.length > kind.n) throw new AccountDecodeError(`${what}: expected ≤ ${kind.n} values`);
      const one: FieldKind = kind.t === 'i32s' ? { t: 'i32' } : kind.t === 'u16s' ? { t: 'u16' } : kind.t === 'u64s' ? { t: 'u64' } : { t: 'u128' };
      v.forEach((x, i) => writeField(out, at + kindSize(one) * i, one, x, `${what}[${i}]`));
      return;
    }
    case 'ticks': {
      if (!Array.isArray(v) || v.length > kind.n) throw new AccountDecodeError(`${what}: expected ≤ ${kind.n} ticks`);
      (v as Partial<Tick>[]).forEach((t, i) => {
        const o = at + 64 * i;
        writeUintLE(out, o, 16, t.liquidityGross ?? 0n);
        writeUintLE(out, o + 16, 16, toTwos128(t.liquidityNet ?? 0n));
        writeUintLE(out, o + 32, 16, t.feeGrowthOutside0X64 ?? 0n);
        writeUintLE(out, o + 48, 16, t.feeGrowthOutside1X64 ?? 0n);
      });
      return;
    }
  }
}

/** Does `data` carry this layout's discriminator? */
export function hasDiscriminator(layout: LayoutDef, data: Uint8Array): boolean {
  return data.length >= 8 && equalBytes(data.subarray(0, 8), layout.discriminator);
}

export const decodeAmmConfig = (data: Uint8Array): AmmConfig => {
  const c = decodeAccount<AmmConfig>(AMM_CONFIG, data);
  // the setting NEW pools copy — read as written.
  return { ...c, currentProtocolFee: c.protocolFee };
};
export const decodeApprovedHook = (data: Uint8Array) => decodeAccount<ApprovedHook>(APPROVED_HOOK, data);
export const decodeMintExclusivity = (data: Uint8Array) => decodeAccount<MintExclusivity>(MINT_EXCLUSIVITY, data);
export const decodeMintPoolCount = (data: Uint8Array) => decodeAccount<MintPoolCount>(MINT_POOL_COUNT, data);
export const decodeQuoteMint = (data: Uint8Array) => decodeAccount<QuoteMint>(QUOTE_MINT, data);
export const decodePool = (data: Uint8Array) => decodeAccount<Pool>(POOL, data);
/**
 * THE ONE SPELLING of "this pool is paused at `nowUnix`": the config authority's unbounded flag, or a guardian pause not yet
 * lapsed. While it is true every swap (both ways), add, donation and hook deposit is refused `PoolPaused`; every
 * removal, fee collection and payout still lands.
 */
export const poolPausedAt = (pool: Pick<Pool, 'paused' | 'guardianPausedUntil'>, nowUnix: bigint): boolean =>
  pool.paused !== 0 || nowUnix < pool.guardianPausedUntil;
/**
 * A tick array's ACCOUNT bytes → its body's fields plus {@link TickArray.rentOwner}.
 * ONE length: {@link TICK_ARRAY_LEN}, 3,920 — the programs refuse a 3,888-byte V1 account `SeedsMismatch`, so a
 *    reader refuses it too. Any other length throws by name — never a truncated "body", never zeros.
 *    The body `[0..3888]` goes through `decodeAccount` (discriminator checked); the tail through
 *    {@link tickArrayRentOwner}.
 */
export function decodeTickArray(data: Uint8Array): TickArray {
  if (data.length !== TICK_ARRAY_LEN) {
    throw new AccountDecodeError(`TickArray: account length ${data.length}, expected ${TICK_ARRAY_LEN}`);
  }
  const body = decodeAccount<Omit<TickArray, 'rentOwner'>>(TICK_ARRAY, data.subarray(0, TICK_ARRAY_RENT_OWNER_OFFSET));
  return { ...body, rentOwner: tickArrayRentOwner(data) };
}

/**
 * The rent owner a tick array's account bytes RECORD: the tail (account bytes 3,888..3,920), or
 * `null` — on a tail that is all-zero, and on any other length. The TS twin of
 * `carpenter_types::layout::tick_array_rent_owner`, which returns the raw tail and leaves "zero is
 * none" to its callers; this applies it. It reads bytes only: the discriminator and owner program
 * are the caller's to check (`decodeTickArray` checks the discriminator).
 */
export function tickArrayRentOwner(data: Uint8Array): Address | null {
  if (data.length !== TICK_ARRAY_LEN) return null;
  const tail = data.subarray(TICK_ARRAY_RENT_OWNER_OFFSET, TICK_ARRAY_LEN);
  return tail.every((b) => b === 0) ? null : bytesToAddress(tail.slice());
}

/**
 * The inverse of {@link decodeTickArray}, for fixtures and mocks: the 3,920-byte account — the
 * `encodeAccount(TICK_ARRAY, fields)` body, then the 32-byte rent owner. It never writes
 * the 3,888-byte body-only account the programs refuse.
 */
export function encodeTickArray(fields: Record<string, unknown>, rentOwner?: Address | null): Uint8Array {
  const body = encodeAccount(TICK_ARRAY, fields);
  const out = new Uint8Array(TICK_ARRAY_LEN);
  out.set(body, 0);
  if (rentOwner !== undefined && rentOwner !== null) out.set(addressToBytes(rentOwner), TICK_ARRAY_RENT_OWNER_OFFSET);
  return out;
}
export const decodePosition = (data: Uint8Array) => decodeAccount<Position>(POSITION, data);
export const decodePoolIndexEntry = (data: Uint8Array) => decodeAccount<PoolIndexEntry>(POOL_INDEX_ENTRY, data);
/** `carpenter_amm`'s `["protocol_treasury"]` (the Carpenter Platform fee's treasury). */
export const decodeProtocolTreasury = (data: Uint8Array) => decodeAccount<ProtocolTreasury>(PROTOCOL_TREASURY, data);

/**
 * The bitmap: the 40-byte header, then exactly `N` LE u64 words where
 * `8 + 40 + 8·N == data.length`. The length is checked against the header's spacing.
 */
export function decodeTickArrayBitmap(data: Uint8Array): TickArrayBitmap {
  const h = decodeAccount<{ pool: Address; tickSpacing: number; arraysPerSide: number }>(TICK_ARRAY_BITMAP, data);
  const span = 60 * h.tickSpacing;
  const perSide = Math.ceil(443_636 / span);
  const n = Math.ceil((2 * perSide + 1) / 64);
  if (perSide !== h.arraysPerSide || data.length !== 8 + 40 + 8 * n) {
    throw new AccountDecodeError(`TickArrayBitmap: length ${data.length} / arrays_per_side ${h.arraysPerSide} disagree with spacing ${h.tickSpacing}`);
  }
  const words: bigint[] = [];
  for (let i = 0; i < n; i++) words.push(readUintLE(data, 48 + 8 * i, 8));
  return { pool: h.pool, tickSpacing: h.tickSpacing, arraysPerSide: h.arraysPerSide, words };
}
