/**
 * `stateView`: the v4 `StateView` analogue, plus the lists — pools, ticks, positions, the hook allowlist, the quote
 * roster, the protocol treasury, and the token-extension reads. Every helper takes an `AccountSource` and returns the
 * decoded value WITH the slot it was read at.
 *
 * Inside a hooked flow, pool state comes from instruction data or post-state, never from a stale account: these
 *    helpers are for UIs and for building transactions.
 * Every account is checked for its OWNER program as well as its discriminator; a lookalike account owned by another
 *    program is `null`, never decoded.
 */

import { addressToBytes, compareBytes } from './bytes.ts';
import { SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID } from './constants.ts';
import {
  APPROVED_HOOK,
  MINT_EXCLUSIVITY,
  MINT_POOL_COUNT,
  POOL_INDEX_ENTRY,
  QUOTE_MINT,
  decodeProtocolTreasury,
  type ProtocolTreasury,
  type AmmConfig,
  type ApprovedHook,
  type LayoutDef,
  type MintExclusivity,
  type MintPoolCount,
  type Pool,
  type PoolIndexEntry,
  type Position,
  type QuoteMint,
  type Tick,
  type TickArray,
  type TickArrayBitmap,
  POSITION,
  decodeAmmConfig,
  decodeApprovedHook,
  decodeMintExclusivity,
  decodeMintPoolCount,
  decodePool,
  decodePoolIndexEntry,
  decodePosition,
  decodeQuoteMint,
  decodeTickArray,
  decodeTickArrayBitmap,
  hasDiscriminator,
} from './layouts.ts';
import {
  DEFAULT_PROGRAM_IDS,
  findProtocolTreasury,
  type ProgramIds,
  findAmmConfig,
  findApprovedHook,
  findBitmap,
  findMintExclusivity,
  findMintPools,
  findPosition,
  findQuoteMint,
  findTickArray,
} from './pda.ts';
import type { AccountSource, RawAccount } from './source.ts';
import { initializedArrayStarts, tickArrayStartIndex } from './tickmath.ts';
import { decodeMint, decodeTokenAccount, type MintInfo, type TokenAccountInfo } from './token2022.ts';
import type { Address } from '@solana/kit';

export interface AtSlot<T> {
  readonly slot: bigint;
  readonly value: T;
}

function ownedBy(a: RawAccount | null, program: Address): RawAccount | null {
  return a && a.owner === program ? a : null;
}

async function readOwned<T>(src: AccountSource, address: Address, program: Address, decode: (d: Uint8Array) => T): Promise<AtSlot<T | null>> {
  const { slot, account } = await src.getAccount(address);
  const a = ownedBy(account, program);
  return { slot, value: a ? decode(a.data) : null };
}

// ---- pools (StateView) ------------------------------------------------------------------------

export const getAmmConfig = async (src: AccountSource, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<AmmConfig | null>> =>
  readOwned(src, (await findAmmConfig(ids)).address, ids.carpenterAmm, decodeAmmConfig);

export const getPool = (src: AccountSource, pool: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<Pool | null>> =>
  readOwned(src, pool, ids.carpenterAmm, decodePool);

export interface Slot0 {
  sqrtPriceX64: bigint;
  tick: number;
  protocolFee: number;
  lpFee: number;
}

/** v4 `getSlot0`. */
export async function getSlot0(src: AccountSource, pool: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<Slot0 | null>> {
  const r = await getPool(src, pool, ids);
  const p = r.value;
  return { slot: r.slot, value: p ? { sqrtPriceX64: p.sqrtPriceX64, tick: p.tickCurrent, protocolFee: p.protocolFee, lpFee: p.lpFee } : null };
}

/** v4 `getLiquidity`. */
export async function getLiquidity(src: AccountSource, pool: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<bigint | null>> {
  const r = await getPool(src, pool, ids);
  return { slot: r.slot, value: r.value ? r.value.liquidity : null };
}

export async function getBitmap(src: AccountSource, pool: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<TickArrayBitmap | null>> {
  return readOwned(src, (await findBitmap(pool, ids)).address, ids.carpenterAmm, decodeTickArrayBitmap);
}

/**
 * One tick array (the 3,920-byte account) with its `rentOwner`: the
 * signer its rent goes back to on close, or `null` on a zero tail. A 3,888-byte V1 account throws.
 * A CLOSED array reads `null` like one never created — the account is gone (system-owned, no data).
 */
export async function getTickArray(
  src: AccountSource,
  pool: Address,
  startIndex: number,
  ids: ProgramIds = DEFAULT_PROGRAM_IDS,
): Promise<AtSlot<TickArray | null>> {
  const r = await readOwned(src, (await findTickArray(pool, startIndex, ids)).address, ids.carpenterAmm, decodeTickArray);
  // The account's `pool` field must equal the pool.
  return { slot: r.slot, value: r.value && r.value.pool === pool && r.value.startIndex === startIndex ? r.value : null };
}

/** v4 `getTickInfo`. An uninitialized tick (or a missing array) reads as all zeros, as in v4. */
export async function getTickInfo(
  src: AccountSource,
  pool: Address,
  tick: number,
  tickSpacing: number,
  ids: ProgramIds = DEFAULT_PROGRAM_IDS,
): Promise<AtSlot<Tick>> {
  if (tick % tickSpacing !== 0) throw new Error(`tick ${tick} is not a multiple of spacing ${tickSpacing}`);
  const start = tickArrayStartIndex(tick, tickSpacing);
  const r = await getTickArray(src, pool, start, ids);
  const zero: Tick = { liquidityGross: 0n, liquidityNet: 0n, feeGrowthOutside0X64: 0n, feeGrowthOutside1X64: 0n };
  return { slot: r.slot, value: r.value ? (r.value.ticks[(tick - start) / tickSpacing] ?? zero) : zero };
}

/**
 * The initialized tick arrays within `radius` arrays of the current tick, both sides, read in one
 * `getMultipleAccounts`. For the depth chart; a swap's arrays come from `predict`.
 */
export async function getTickArraysAround(
  src: AccountSource,
  pool: Address,
  radius = 3,
  ids: ProgramIds = DEFAULT_PROGRAM_IDS,
): Promise<AtSlot<TickArray[]>> {
  const p = (await getPool(src, pool, ids)).value;
  if (!p) throw new Error(`getTickArraysAround: no pool at ${pool}`);
  const bm = (await getBitmap(src, pool, ids)).value;
  if (!bm) throw new Error(`getTickArraysAround: no bitmap for ${pool}`);
  const span = 60 * p.tickSpacing;
  const here = tickArrayStartIndex(p.tickCurrent, p.tickSpacing);
  const starts = initializedArrayStarts(bm.words, p.tickSpacing).filter((s) => Math.abs(s - here) <= radius * span);
  const addrs = await Promise.all(starts.map(async (s) => (await findTickArray(pool, s, ids)).address));
  const r = await src.getMultipleAccounts(addrs);
  const out: TickArray[] = [];
  r.accounts.forEach((a, i) => {
    const owned = ownedBy(a, ids.carpenterAmm);
    if (!owned) return;
    const ta = decodeTickArray(owned.data);
    if (ta.pool === pool && ta.startIndex === starts[i]) out.push(ta);
  });
  return { slot: r.slot, value: out };
}

/**
 * V4 `getFeeGrowthInside`. Reads both bound ticks and the pool
 * in ONE `getMultipleAccounts`, so the three values share a slot.
 */
export async function getFeeGrowthInside(
  src: AccountSource,
  pool: Address,
  tickLower: number,
  tickUpper: number,
  ids: ProgramIds = DEFAULT_PROGRAM_IDS,
): Promise<AtSlot<{ feeGrowthInside0X64: bigint; feeGrowthInside1X64: bigint }>> {
  const pr = await getPool(src, pool, ids);
  const p = pr.value;
  if (!p) throw new Error(`getFeeGrowthInside: no pool at ${pool}`);
  const s = p.tickSpacing;
  const lo = tickArrayStartIndex(tickLower, s);
  const hi = tickArrayStartIndex(tickUpper, s);
  const [loAddr, hiAddr] = await Promise.all([findTickArray(pool, lo, ids), findTickArray(pool, hi, ids)]);
  const r = await src.getMultipleAccounts([pool, loAddr.address, hiAddr.address]);
  const poolNow = ownedBy(r.accounts[0] ?? null, ids.carpenterAmm);
  const P = poolNow ? decodePool(poolNow.data) : p;
  const tickOf = (acct: RawAccount | null | undefined, start: number, tick: number): Tick => {
    const owned = ownedBy(acct ?? null, ids.carpenterAmm);
    const zero: Tick = { liquidityGross: 0n, liquidityNet: 0n, feeGrowthOutside0X64: 0n, feeGrowthOutside1X64: 0n };
    if (!owned) return zero;
    const ta = decodeTickArray(owned.data);
    return ta.pool === pool ? (ta.ticks[(tick - start) / s] ?? zero) : zero;
  };
  const L = tickOf(r.accounts[1], lo, tickLower);
  const U = tickOf(r.accounts[2], hi, tickUpper);
  const M = 1n << 128n;
  const wrap = (x: bigint) => ((x % M) + M) % M;
  const inside = (global: bigint, lowerOut: bigint, upperOut: bigint): bigint => {
    const below = P.tickCurrent >= tickLower ? lowerOut : wrap(global - lowerOut);
    const above = P.tickCurrent < tickUpper ? upperOut : wrap(global - upperOut);
    return wrap(global - below - above);
  };
  return {
    slot: r.slot,
    value: {
      feeGrowthInside0X64: inside(P.feeGrowthGlobal0X64, L.feeGrowthOutside0X64, U.feeGrowthOutside0X64),
      feeGrowthInside1X64: inside(P.feeGrowthGlobal1X64, L.feeGrowthOutside1X64, U.feeGrowthOutside1X64),
    },
  };
}

/** v4 `getPositionInfo`, keyed `(pool, owner, salt)`: the position PDA `["position", pool, owner, salt u64 LE]`. */
export async function getPosition(
  src: AccountSource,
  pool: Address,
  owner: Address,
  salt: bigint,
  ids: ProgramIds = DEFAULT_PROGRAM_IDS,
): Promise<AtSlot<Position | null>> {
  return readOwned(src, (await findPosition(pool, owner, salt, ids)).address, ids.carpenterAmm, decodePosition);
}

/** Server-side only. memcmp on `owner` at ACCOUNT offset 48. */
export async function positionsForOwner(src: AccountSource, owner: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<{ address: Address; position: Position }[]>> {
  const r = await src.getProgramAccounts(ids.carpenterAmm, [
    { memcmp: { offset: 0, bytes: POSITION.discriminator } },
    { memcmp: { offset: 48, bytes: addressToBytes(owner) } },
  ]);
  return { slot: r.slot, value: decodeAll(r.accounts, POSITION, decodePosition).map(([address, position]) => ({ address, position })) };
}

function decodeAll<T>(rows: { address: Address; account: RawAccount }[], layout: LayoutDef, decode: (d: Uint8Array) => T): [Address, T][] {
  const out: [Address, T][] = [];
  for (const { address, account } of rows) {
    if (!hasDiscriminator(layout, account.data)) continue;
    out.push([address, decode(account.data)]);
  }
  return out;
}

export interface Page<T> {
  readonly items: T[];
  /** Pass back to get the next page; `null` on the last page. */
  readonly cursor: string | null;
}

/**
 * Server-side only. `PoolIndexEntry` by discriminator, paged CLIENT-side by
 * `(created_slot, pool)` ascending: stable under concurrent `initialize_pool`s, because nothing
 * about an existing entry changes when another is created.
 */
export async function listPools(
  src: AccountSource,
  opts: { cursor?: string | null; limit?: number } = {},
  ids: ProgramIds = DEFAULT_PROGRAM_IDS,
): Promise<AtSlot<Page<PoolIndexEntry>>> {
  const r = await src.getProgramAccounts(ids.carpenterAmm, [
    { memcmp: { offset: 0, bytes: POOL_INDEX_ENTRY.discriminator } },
    { dataSize: 8 + POOL_INDEX_ENTRY.size },
  ]);
  const rows = decodeAll(r.accounts, POOL_INDEX_ENTRY, decodePoolIndexEntry).map(([, e]) => e);
  rows.sort((a, b) => (a.createdSlot === b.createdSlot ? compareBytes(addressToBytes(a.pool), addressToBytes(b.pool)) : a.createdSlot < b.createdSlot ? -1 : 1));
  return { slot: r.slot, value: pageAfter(rows, (e) => `${e.createdSlot}:${e.pool}`, opts) };
}

function pageAfter<T>(rows: T[], key: (t: T) => string, opts: { cursor?: string | null; limit?: number }): Page<T> {
  const limit = opts.limit ?? 50;
  let start = 0;
  if (opts.cursor) {
    const i = rows.findIndex((r) => key(r) === opts.cursor);
    start = i < 0 ? rows.length : i + 1;
  }
  const items = rows.slice(start, start + limit);
  const last = items[items.length - 1];
  return { items, cursor: start + limit < rows.length && last !== undefined ? key(last) : null };
}

/** The `MintPoolCount` account, judged present by owner, length and discriminator, never by lamports. */
export async function getMintPoolCount(src: AccountSource, mint: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<MintPoolCount | null>> {
  const { slot, account } = await src.getAccount((await findMintPools(mint, ids)).address);
  const a = ownedBy(account, ids.carpenterAmm);
  const present = a && a.data.length === 8 + MINT_POOL_COUNT.size && hasDiscriminator(MINT_POOL_COUNT, a.data);
  return { slot, value: present ? decodeMintPoolCount(a.data) : null };
}

/**
 * The quote ROSTER entry, judged present by owner, length and discriminator. `null` means "not on the roster", which
 * is NOT the same as "not a quote": wSOL is the IMPLIED quote and carries no entry at all. A present entry with `state === 2` is RETIRED: live pools keep trading,
 * no new pool may open with it in the quote slot.
 */
export async function getQuoteMint(src: AccountSource, mint: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<QuoteMint | null>> {
  const { slot, account } = await src.getAccount((await findQuoteMint(mint, ids)).address);
  const a = ownedBy(account, ids.carpenterAmm);
  const present = a && a.data.length === 8 + QUOTE_MINT.size && hasDiscriminator(QUOTE_MINT, a.data);
  return { slot, value: present ? decodeQuoteMint(a.data) : null };
}

/** The `MintExclusivity` account, judged present by owner, length and discriminator. */
export async function getMintExclusivity(src: AccountSource, mint: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<MintExclusivity | null>> {
  const { slot, account } = await src.getAccount((await findMintExclusivity(mint, ids)).address);
  const a = ownedBy(account, ids.carpenterAmm);
  const present = a && a.data.length === 8 + MINT_EXCLUSIVITY.size && hasDiscriminator(MINT_EXCLUSIVITY, a.data);
  return { slot, value: present ? decodeMintExclusivity(a.data) : null };
}

export async function getApprovedHook(src: AccountSource, hookProgram: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<ApprovedHook | null>> {
  return readOwned(src, (await findApprovedHook(hookProgram, ids)).address, ids.carpenterAmm, decodeApprovedHook);
}

/** Server-side only. Every `ApprovedHook`. */
export async function listApprovedHooks(src: AccountSource, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<{ address: Address; hook: ApprovedHook }[]>> {
  const r = await src.getProgramAccounts(ids.carpenterAmm, [{ memcmp: { offset: 0, bytes: APPROVED_HOOK.discriminator } }]);
  return { slot: r.slot, value: decodeAll(r.accounts, APPROVED_HOOK, decodeApprovedHook).map(([address, hook]) => ({ address, hook })) };
}

/**
 * carpenter_amm's `["protocol_treasury"]`: its governance (the key that withdraws the fee AND sets the rate new pools copy) and any pending handover.
 * `null` until `initialize_protocol_treasury` has run; until then `sweep_protocol_fees` is refused and every
 * pool keeps its `protocol_fees_owed`. Owned by `ids.carpenterAmm`, never floor_hook.
 */
export async function getProtocolTreasury(src: AccountSource, ids: ProgramIds = DEFAULT_PROGRAM_IDS): Promise<AtSlot<ProtocolTreasury | null>> {
  return readOwned(src, (await findProtocolTreasury(ids)).address, ids.carpenterAmm, decodeProtocolTreasury);
}
// ---- token extensions -------------------------------------------------------------------------

function tokenProgramOwned(a: RawAccount | null): RawAccount | null {
  return a && (a.owner === TOKEN_2022_PROGRAM_ID || a.owner === SPL_TOKEN_PROGRAM_ID) ? a : null;
}

/** Mint base fields and Token-2022 extensions (Pausable, MetadataPointer, TokenMetadata …). */
export async function getMintExtensions(src: AccountSource, mint: Address): Promise<AtSlot<(MintInfo & { tokenProgram: Address }) | null>> {
  const { slot, account } = await src.getAccount(mint);
  const a = tokenProgramOwned(account);
  return { slot, value: a ? { ...decodeMint(a.data), tokenProgram: a.owner } : null };
}

/** Token account base fields and extensions. */
export async function getAtaExtensions(src: AccountSource, tokenAccount: Address): Promise<AtSlot<(TokenAccountInfo & { tokenProgram: Address }) | null>> {
  const { slot, account } = await src.getAccount(tokenAccount);
  const a = tokenProgramOwned(account);
  return { slot, value: a ? { ...decodeTokenAccount(a.data), tokenProgram: a.owner } : null };
}

/** The named `stateView` object; the functions are also exported individually. */
export const stateView = {
  getAmmConfig,
  getPool,
  getSlot0,
  getLiquidity,
  getBitmap,
  getTickArray,
  getTickInfo,
  getTickArraysAround,
  getFeeGrowthInside,
  getPosition,
  positionsForOwner,
  listPools,
  getMintPoolCount,
  getQuoteMint,
  getMintExclusivity,
  getApprovedHook,
  listApprovedHooks,
  getProtocolTreasury,
  getMintExtensions,
  getAtaExtensions,
} as const;
