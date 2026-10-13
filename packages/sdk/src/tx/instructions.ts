/**
 * Raw `carpenter_amm` instruction encoders: `discriminator ‖ borsh(args)` and the account list in the program's
 * `#[derive(Accounts)]` order, plus the shared encoder machinery (`makeInstruction`, the slot helpers) a hook's SDK
 * reuses. Pure and synchronous; every address is already resolved (the builders resolve PDAs).
 *
 * Account order is the wire format. Each instruction also carries `accountNames`, the Rust field names in order;
 *    They follow the shipped IDL's account list.
 * `#[event_cpi]` appends `event_authority` then `program` AFTER the named accounts and BEFORE the remaining accounts.
 * Anchor `Option<Account>`: `None` is the executing PROGRAM's own id in that slot, read-only.
 */

import { AccountRole, type AccountMeta, type Address, type Instruction } from '@solana/kit';

import { BorshWriter } from '../borsh.ts';
import { SYSTEM_PROGRAM_ID } from '../constants.ts';

export interface ProgramInstruction extends Instruction {
  readonly programAddress: Address;
  readonly accounts: readonly AccountMeta[];
  readonly data: Uint8Array;
  /** The Rust account field names, in order, for the named part (remaining accounts excluded). */
  readonly accountNames: readonly string[];
  /**
   * The Rust ARGUMENT names, in order, for what follows the discriminator in `data`.
   * They follow the IDL's `args`; checking `data` against the IDL's types catches an argument added,
   * dropped or reordered on the Rust side.
   * Anchor's borsh does not require full consumption: a trailing argument the program no longer
   * takes is silently dropped, and a missing leading one fails `InstructionDidNotDeserialize`.
   */
  readonly argNames: readonly string[];
  /** The Anchor instruction name (snake_case). */
  readonly name: string;
}

export type Slot = readonly [name: string, address: Address, role: AccountRole];

export const ro = (name: string, address: Address): Slot => [name, address, AccountRole.READONLY];
export const rw = (name: string, address: Address): Slot => [name, address, AccountRole.WRITABLE];
export const rs = (name: string, address: Address): Slot => [name, address, AccountRole.READONLY_SIGNER];
export const ws = (name: string, address: Address): Slot => [name, address, AccountRole.WRITABLE_SIGNER];
/** An Anchor `Option<…>` slot: `null` → the program id, read-only. */
export const opt = (name: string, address: Address | null, role: AccountRole, program: Address): Slot =>
  address === null ? [name, program, AccountRole.READONLY] : [name, address, role];

const d = (...b: number[]): Uint8Array => new Uint8Array(b);

/** The Anchor discriminators: `sha256("global:<name>")[..8]`. */
export const DISC = {
  amm: {
    initialize_pool: d(0x5f, 0xb4, 0x0a, 0xac, 0x54, 0xae, 0xe8, 0x28),
    initialize_tick_array: d(0x0b, 0xbc, 0xc1, 0xd6, 0x8d, 0x5b, 0x95, 0xb8),
    modify_liquidity: d(0x0e, 0xd6, 0xa8, 0x29, 0x37, 0x0f, 0x35, 0x11),
    close_position: d(0x7b, 0x86, 0x51, 0x00, 0x31, 0x44, 0x62, 0x62),
    // Tick-array rent recovery (`disc.rs` `CLOSE_TICK_ARRAY`); no args — the handler reads `start_index` off
    // the account.
    close_tick_array: d(0xb9, 0x0a, 0x31, 0x50, 0x41, 0x70, 0xcd, 0xc7),
    transfer_position: d(0x8b, 0x82, 0x66, 0x93, 0x87, 0x4d, 0x71, 0xde),
    swap: d(0xf8, 0xc6, 0x9e, 0x91, 0xe1, 0x75, 0x87, 0xc8),
    // `swap`'s exact-in with the amount resolved ON CHAIN as
    // `trader_token_in.amount - keep` (the graduated SOL route's second leg). `swap`'s accounts.
    swap_from_balance: d(0xcc, 0xa0, 0xe9, 0x06, 0x87, 0xf9, 0x47, 0x8a),
    // The AMM quote ROSTER, four config-authority instructions.
    list_quote_mint: d(0x64, 0x9b, 0x7a, 0x62, 0x74, 0x58, 0x39, 0x99),
    retire_quote_mint: d(0x5d, 0x39, 0xad, 0x30, 0x76, 0x9f, 0xf1, 0x1d),
    delist_quote_mint: d(0x56, 0xe0, 0x27, 0x54, 0x97, 0xa5, 0x05, 0xd1),
    relist_quote_mint: d(0x92, 0xfe, 0xd4, 0x0b, 0x62, 0x05, 0x1b, 0x0e),
    // The governance ADMISSION MASK for listed quotes.
    set_quote_caps_admitted: d(0xa2, 0x7b, 0x4c, 0x23, 0xec, 0x26, 0xd5, 0xfa),
    // A hook's own quote listings, behind the config authority's switch.
    set_hook_quotes: d(0xb4, 0xcc, 0xac, 0x70, 0x8d, 0x9f, 0x31, 0x89),
    hook_list_quote_mint: d(0x36, 0x31, 0x70, 0x02, 0x49, 0x80, 0xc4, 0xc5),
    // The config authority names, ONCE, the ONE transfer-hook program a pool of an approved hook may carry ARMED on its base mint.
    set_base_transfer_hook: d(0x8e, 0x53, 0xf5, 0xea, 0xa8, 0xf2, 0x02, 0xcf),
    // `carpenter_amm`'s `["protocol_treasury"]` — open,
    // the PERMISSIONLESS sweep, governance's uncapped withdraw, the two-step handover and the capped fee
    // setter. From `crates/carpenter-types/src/disc.rs`'s `amm::*_PROTOCOL_*` (each sha256("global:<name>")[..8]).
    initialize_protocol_treasury: d(0x7d, 0x94, 0x35, 0xc5, 0x88, 0x51, 0x41, 0x3a),
    sweep_protocol_fees: d(0x75, 0xe1, 0xf0, 0x4b, 0x47, 0xc3, 0x2c, 0x2b),
    withdraw_protocol_fees: d(0x0b, 0x44, 0xa5, 0x62, 0x12, 0xd0, 0x86, 0x49),
    set_protocol_treasury_governance: d(0xa9, 0x17, 0x66, 0x9b, 0xa9, 0x65, 0xa7, 0xdd),
    accept_protocol_treasury_governance: d(0x41, 0x00, 0xaf, 0xe8, 0xc2, 0x34, 0x56, 0xc0),
    set_protocol_fee: d(0xad, 0xef, 0x53, 0xf2, 0x88, 0x2b, 0x90, 0xd9),
  },
} as const;

export function makeInstruction(
  name: string,
  program: Address,
  disc: Uint8Array,
  args: BorshWriter | null,
  slots: readonly Slot[],
  opts: { eventAuthority?: Address; remaining?: readonly AccountMeta[]; argNames?: readonly string[] } = {},
): ProgramInstruction {
  const all: Slot[] = [...slots];
  if (opts.eventAuthority) all.push(ro('event_authority', opts.eventAuthority), ro('program', program));
  const accounts: AccountMeta[] = all.map(([, address, role]) => ({ address, role }));
  if (opts.remaining) accounts.push(...opts.remaining);
  const data = args ? new Uint8Array([...disc, ...args.toBytes()]) : new Uint8Array(disc);
  return { name, programAddress: program, accounts, data, accountNames: all.map(([n]) => n), argNames: opts.argNames ?? [] };
}

const meta = (address: Address, role: AccountRole): AccountMeta => ({ address, role });
export const remainingRw = (a: Address): AccountMeta => meta(a, AccountRole.WRITABLE);
export const remainingRo = (a: Address): AccountMeta => meta(a, AccountRole.READONLY);

// =============================================================================================
// carpenter_amm 
// =============================================================================================

/**
 * The zero key as a `rent_owner` argument: "record the payer" (the AMM resolves zero to the payer —
 * the cranker in a ratchet). 32 zero bytes, which base58 spells as the System Program's id.
 */
export const ZERO_RENT_OWNER: Address = SYSTEM_PROGRAM_ID;

/**
 * `initialize_tick_array` —
 * 44 bytes, `disc ‖ start_index i32 ‖ rent_owner 32`, the account list unchanged. `rentOwner` is the
 * key the new array records at account offset 3,888, the ONLY signer that may later close it and take
 * its rent back. Omitted (or the zero key), the program records the PAYER. Evaluated only when the
 * array is created: an existing array is never re-owned.
 * IDEMPOTENT: an existing array is left as it is and the instruction succeeds, so it can be
 * prepended defensively (`buildSwap`'s `ensureTickArrayStarts`).
 */
export function initializeTickArrayIx(p: { program: Address; payer: Address; pool: Address; tickArray: Address; startIndex: number; rentOwner?: Address | null }): ProgramInstruction {
  return makeInstruction('initialize_tick_array', p.program, DISC.amm.initialize_tick_array, new BorshWriter().i32(p.startIndex).pubkey(p.rentOwner ?? ZERO_RENT_OWNER), [
    ws('payer', p.payer),
    ro('pool', p.pool),
    rw('tick_array', p.tickArray),
    ro('system_program', SYSTEM_PROGRAM_ID),
  ], { argNames: ['start_index', 'rent_owner'] });
}

/**
 * `close_tick_array`: closes an EMPTY tick
 * array and refunds ALL its lamports to its recorded rent owner, who must sign. Data = the 8-byte
 * discriminator alone. Accounts: `rent_owner` s,w · `pool` · `bitmap` · `tick_array` w, then
 * `#[event_cpi]`. The program refuses, in order: a bitmap or array of another pool; any signer but the recorded rent
 * owner (`Unauthorized` 6000); an initialized tick (`TickArrayNotEmpty` 6078); a non-zero tick byte
 * (`TickArrayNotZeroed` 6079); a bitmap bit still set (`TickArrayBitmapBitSet` 6080). Not gated by a
 * pool pause or any hook. Emits `TickArrayClosed`.
 * Never offer it on a BONDING FLOOR launch's prepared graduation arrays: their rent owner is the launch's
 * `["grad_rent", launch]` reserve, which cannot sign this. The FLOOR SDK closes those (the creator, after
 * graduation).
 */
export function closeTickArrayIx(p: { program: Address; eventAuthority: Address; rentOwner: Address; pool: Address; bitmap: Address; tickArray: Address }): ProgramInstruction {
  return makeInstruction('close_tick_array', p.program, DISC.amm.close_tick_array, null, [
    ws('rent_owner', p.rentOwner),
    ro('pool', p.pool),
    ro('bitmap', p.bitmap),
    rw('tick_array', p.tickArray),
  ], { eventAuthority: p.eventAuthority });
}

export interface InitializePoolAccounts {
  program: Address;
  eventAuthority: Address;
  payer: Address;
  initializer: Address;
  config: Address;
  pool: Address;
  bitmap: Address;
  vault0: Address;
  vault1: Address;
  mint0: Address;
  mint1: Address;
  tokenProgram0: Address;
  tokenProgram1: Address;
  mintExclusivity0: Address;
  mintExclusivity1: Address;
  mintPools0: Address;
  mintPools1: Address;
  /** Writable only when that mint is not a quote mint; the SDK marks it. */
  mintPools0Writable: boolean;
  mintPools1Writable: boolean;
  poolIndexEntry: Address;
  approvedHook: Address | null;
  hookProgram: Address | null;
  hookProgramdata: Address | null;
  /**
   * The quote ROSTER entries, `["quote_mint", mint_i]`. REQUIRED, never
   * optional: an omitted optional slot reads as "no roster entry", which is exactly the answer
   * that bypasses `retire_quote_mint`. Pass the derived address whether or not the entry exists;
   * omitting either is refused `AccountNotEnoughKeys`.
   */
  quoteMint0: Address;
  quoteMint1: Address;
  /** The hook slice. */
  hookSlice: readonly AccountMeta[];
}

export function initializePoolIx(a: InitializePoolAccounts, args: { fee: number; tickSpacing: number; hook: Address; sqrtPriceX64: bigint }): ProgramInstruction {
  const w = new BorshWriter().u32(args.fee).u16(args.tickSpacing).pubkey(args.hook).u128(args.sqrtPriceX64);
  const P = a.program;
  return makeInstruction(
    'initialize_pool',
    P,
    DISC.amm.initialize_pool,
    w,
    [
      ws('payer', a.payer),
      rs('initializer', a.initializer),
      ro('config', a.config),
      rw('pool', a.pool),
      rw('bitmap', a.bitmap),
      rw('vault0', a.vault0),
      rw('vault1', a.vault1),
      ro('mint0', a.mint0),
      ro('mint1', a.mint1),
      ro('token_program0', a.tokenProgram0),
      ro('token_program1', a.tokenProgram1),
      ro('mint_exclusivity0', a.mintExclusivity0),
      ro('mint_exclusivity1', a.mintExclusivity1),
      a.mintPools0Writable ? rw('mint_pools0', a.mintPools0) : ro('mint_pools0', a.mintPools0),
      a.mintPools1Writable ? rw('mint_pools1', a.mintPools1) : ro('mint_pools1', a.mintPools1),
      rw('pool_index_entry', a.poolIndexEntry),
      opt('approved_hook', a.approvedHook, AccountRole.READONLY, P),
      opt('hook_program', a.hookProgram, AccountRole.READONLY, P),
      opt('hook_programdata', a.hookProgramdata, AccountRole.READONLY, P),
      ro('system_program', SYSTEM_PROGRAM_ID),
      ro('quote_mint0', a.quoteMint0),
      ro('quote_mint1', a.quoteMint1),
    ],
    { eventAuthority: a.eventAuthority, remaining: a.hookSlice, argNames: ['fee', 'tick_spacing', 'hook', 'sqrt_price_x64'] },
  );
}

export interface ModifyLiquidityAccounts {
  program: Address;
  eventAuthority: Address;
  owner: Address;
  payer: Address | null;
  pool: Address;
  bitmap: Address;
  position: Address;
  tickArrayLower: Address;
  tickArrayUpper: Address;
  ownerToken0: Address | null;
  ownerToken1: Address | null;
  vault0: Address;
  vault1: Address;
  mint0: Address;
  mint1: Address;
  tokenProgram0: Address;
  tokenProgram1: Address;
  hookProgram: Address | null;
  /**
   * Proves `positionId` when the position is CREATED and the id is not the owner's own key (the
   * program's `modify_liquidity`): a present authority must BE the id, and with none the owner's key must be.
   * `null` — the ordinary wallet LP — is the program's own id in that slot, read-only.
   */
  positionAuthority: Address | null;
  /** The hook slice. On a FLOOR pool the AMM never CONSUMES it (no liquidity phase flags), so the SDK keeps passing
   *  `[floor_pool w]` here — the transfer hook's `floor_pool`, found by key — and not the swap's 4-account slice. */
  hookSlice: readonly AccountMeta[];
  /** A hooked base mint's transfer-hook accounts, appended after the hook slice (found by key). */
  transferHookAccounts?: readonly AccountMeta[];
}

/**
 * `positionId` is the FIRST wire argument, and `position_authority` follows `hook_program` in the account
 * list. The position PDA is `["position", pool, position_id, salt u64 LE]`, so an ordinary wallet LP passes
 * its OWN key as `positionId`.
 */
export function modifyLiquidityIx(
  a: ModifyLiquidityAccounts,
  args: {
    positionId: Uint8Array; salt: bigint; tickLower: number; tickUpper: number; liquidityDelta: bigint; amount0Max: bigint; amount1Max: bigint; hookData?: Uint8Array;
    /**
     * The argument after `hook_data`'s Borsh
     * `u32 len ‖ bytes` — the rent owner each tick array THIS call creates records (zero / omitted =
     * the payer). Ignored when nothing is created; an existing array is never re-owned.
     */
    rentOwner?: Address | null;
    /**
     * The least the caller must RECEIVE of each side (principal plus fees), sent as the
     * trailing `min_out` AFTER `rent_owner` (`u64 ‖ u64`). Omitted = 0 / 0: no bound. A side with a minimum that
     * the caller would pay on fails `SlippageExceeded`.
     * ALWAYS written (16 bytes, 0 / 0 when omitted — the program reads 0 / 0 as no bound, exactly as
     * it reads 0 bytes), so the wire is the Rust arg list in full.
     */
    amount0Min?: bigint;
    amount1Min?: bigint;
  },
): ProgramInstruction {
  const w = new BorshWriter()
    .fixed(args.positionId, 32)
    .u64(args.salt)
    .i32(args.tickLower)
    .i32(args.tickUpper)
    .i128(args.liquidityDelta)
    .u64(args.amount0Max)
    .u64(args.amount1Max)
    .vecU8(args.hookData ?? new Uint8Array())
    .pubkey(args.rentOwner ?? ZERO_RENT_OWNER)
    .u64(args.amount0Min ?? 0n)
    .u64(args.amount1Min ?? 0n);
  const P = a.program;
  return makeInstruction(
    'modify_liquidity',
    P,
    DISC.amm.modify_liquidity,
    w,
    [
      rs('owner', a.owner),
      opt('payer', a.payer, AccountRole.WRITABLE_SIGNER, P),
      rw('pool', a.pool),
      rw('bitmap', a.bitmap),
      rw('position', a.position),
      rw('tick_array_lower', a.tickArrayLower),
      rw('tick_array_upper', a.tickArrayUpper),
      opt('owner_token0', a.ownerToken0, AccountRole.WRITABLE, P),
      opt('owner_token1', a.ownerToken1, AccountRole.WRITABLE, P),
      rw('vault0', a.vault0),
      rw('vault1', a.vault1),
      ro('mint0', a.mint0),
      ro('mint1', a.mint1),
      ro('token_program0', a.tokenProgram0),
      ro('token_program1', a.tokenProgram1),
      ro('system_program', SYSTEM_PROGRAM_ID),
      opt('hook_program', a.hookProgram, AccountRole.READONLY, P),
      opt('position_authority', a.positionAuthority, AccountRole.READONLY_SIGNER, P),
    ],
    {
      eventAuthority: a.eventAuthority,
      remaining: [...a.hookSlice, ...(a.transferHookAccounts ?? [])],
      argNames: ['position_id', 'salt', 'tick_lower', 'tick_upper', 'liquidity_delta', 'amount0_max', 'amount1_max', 'hook_data', 'rent_owner', 'min_out'],
    },
  );
}

export function closePositionIx(p: { program: Address; eventAuthority: Address; owner: Address; position: Address }): ProgramInstruction {
  return makeInstruction('close_position', p.program, DISC.amm.close_position, null, [ws('owner', p.owner), rw('position', p.position)], {
    eventAuthority: p.eventAuthority,
  });
}

/**
 * `transfer_position`. The current owner signs; the
 * handler writes `Position.owner` and nothing else, so the fee accrual since the last modify
 * travels with the position — poke it out first (a zero-delta `modify_liquidity`) to keep it.
 * On a HOOKED pool the `hook_authority` is refused as source AND destination
 * (`PositionNotTransferable`), which is why `pool` is passed: it is what decides.
 */
export function transferPositionIx(
  p: { program: Address; eventAuthority: Address; owner: Address; position: Address; pool: Address },
  newOwner: Address,
): ProgramInstruction {
  return makeInstruction(
    'transfer_position',
    p.program,
    DISC.amm.transfer_position,
    new BorshWriter().pubkey(newOwner),
    [rs('owner', p.owner), rw('position', p.position), ro('pool', p.pool)],
    { eventAuthority: p.eventAuthority, argNames: ['new_owner'] },
  );
}

export interface SwapAccounts {
  program: Address;
  eventAuthority: Address;
  trader: Address;
  pool: Address;
  bitmap: Address;
  traderTokenIn: Address | null;
  traderTokenOut: Address | null;
  vault0: Address;
  vault1: Address;
  mint0: Address;
  mint1: Address;
  tokenProgram0: Address;
  tokenProgram1: Address;
  hookProgram: Address | null;
  /** The hook slice first, then the UNORDERED tick-array / action-position set. FLOOR: `[floor_pool w, trader_bucket w, hook event_authority r, floor_hook r]` — the bucket the SIGNER's
   *  `["trader_ref", trader, mint0]`, then the callbacks' `#[event_cpi]` pair (`floorSwapHookSlice` in `@floor/sdk-solana`). */
  hookSlice: readonly AccountMeta[];
  tickArrays: readonly Address[];
  actionPositions: readonly Address[];
  /**
   * a HOOKED base mint's transfer-hook accounts
   * (from the hook's SDK, minus what this instruction already names), APPENDED LAST. The AMM's
   * hook-aware legs find them BY KEY in the remaining set, so their position carries no meaning.
   */
  transferHookAccounts?: readonly AccountMeta[];
}

/**
 * The `Swap` accounts struct's named slots, in `#[derive(Accounts)]` order. ONE list for `swapIx`
 * and `swapFromBalanceIx`: the program reuses the SAME struct for both
 * instructions, so the SDK builds both from the same slots and the two cannot drift.
 */
function swapSlots(a: SwapAccounts): Slot[] {
  const P = a.program;
  return [
    rs('trader', a.trader),
    rw('pool', a.pool),
    // WRITABLE: a `ModifyPosition` action inside `swap` can flip a tick array's `initialized_tick_count` across 0,
    // which writes the bitmap (the program's `swap` declares it `#[account(mut)]`).
    rw('bitmap', a.bitmap),
    opt('trader_token_in', a.traderTokenIn, AccountRole.WRITABLE, P),
    opt('trader_token_out', a.traderTokenOut, AccountRole.WRITABLE, P),
    rw('vault0', a.vault0),
    rw('vault1', a.vault1),
    ro('mint0', a.mint0),
    ro('mint1', a.mint1),
    ro('token_program0', a.tokenProgram0),
    ro('token_program1', a.tokenProgram1),
    opt('hook_program', a.hookProgram, AccountRole.READONLY, P),
  ];
}

/** The hook slice, then the unordered tick-array / action-position set. */
const swapRemaining = (a: SwapAccounts): AccountMeta[] => [
  ...a.hookSlice,
  ...a.tickArrays.map(remainingRw),
  ...a.actionPositions.map(remainingRw),
  ...(a.transferHookAccounts ?? []),
];

export function swapIx(
  a: SwapAccounts,
  args: { zeroForOne: boolean; amountSpecified: bigint; sqrtPriceLimitX64: bigint; amountLimit: bigint; hookData?: Uint8Array },
): ProgramInstruction {
  const w = new BorshWriter()
    .bool(args.zeroForOne)
    .i128(args.amountSpecified)
    .u128(args.sqrtPriceLimitX64)
    .u64(args.amountLimit)
    .vecU8(args.hookData ?? new Uint8Array());
  return makeInstruction('swap', a.program, DISC.amm.swap, w, swapSlots(a), {
    eventAuthority: a.eventAuthority,
    remaining: swapRemaining(a),
    argNames: ['zero_for_one', 'amount_specified', 'sqrt_price_limit_x64', 'amount_limit', 'hook_data'],
  });
}

/**
 * `swap_from_balance(zero_for_one, keep, sqrt_price_limit_x64, amount_limit, limit_in_basis,
 * hook_data)`: `swap`'s exact-in, the amount RESOLVED ON CHAIN as
 * `trader_token_in.amount − keep`, over `swap`'s accounts. `53 + n` bytes, `swap`'s length.
 *
 * - `keep` — the trader's RAW balance of the input token read at the press (never a UI amount);
 *   `0n` ONLY when the account does not exist yet and is created in this transaction — a `0n`
 *   from a FAILED read would sell the trader's whole pre-existing holding.
 * - `amountLimit` — exact-in's minimum out, unchanged (`SlippageExceeded`).
 * - `limitInBasis` — the rate clause: `out * limitInBasis >= amountLimit * in`; `0n` = off.
 * - The program refuses by name: `BalanceAtOrBelowKeep` (6070), `BalanceSwapNotOwner` (6071, the
 *   input account's authority must be the trader), `BalanceSwapSelfCall` (6072),
 *   `BalanceSwapPartialFill` (6073, the walk must spend ALL of it).
 */
export function swapFromBalanceIx(
  a: SwapAccounts,
  args: { zeroForOne: boolean; keep: bigint; sqrtPriceLimitX64: bigint; amountLimit: bigint; limitInBasis: bigint; hookData?: Uint8Array },
): ProgramInstruction {
  const w = new BorshWriter()
    .bool(args.zeroForOne)
    .u64(args.keep)
    .u128(args.sqrtPriceLimitX64)
    .u64(args.amountLimit)
    .u64(args.limitInBasis)
    .vecU8(args.hookData ?? new Uint8Array());
  return makeInstruction('swap_from_balance', a.program, DISC.amm.swap_from_balance, w, swapSlots(a), {
    eventAuthority: a.eventAuthority,
    remaining: swapRemaining(a),
    argNames: ['zero_for_one', 'keep', 'sqrt_price_limit_x64', 'amount_limit', 'limit_in_basis', 'hook_data'],
  });
}

