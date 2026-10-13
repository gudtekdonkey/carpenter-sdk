/**
 * The Address Lookup Table program's own instructions (it has no IDL), the table account's decoder, and the deactivate /
 * close builders.
 * A table frozen with `FreezeLookupTable` can never be deactivated or closed: the builders here serve unfrozen tables
 *    only, and the chain refuses them for a frozen one.
 */

import { AccountRole, type Address } from '@solana/kit';

import { addressToBytes, bytesToAddress } from '../bytes.ts';
import { SYSTEM_PROGRAM_ID } from '../constants.ts';
import { ADDRESS_LOOKUP_TABLE_PROGRAM_ID } from '../pda.ts';
import { finalizeUnits, type BuiltTransaction } from './common.ts';
import type { ProgramInstruction } from './instructions.ts';

// ---- the ALT program's wire (solana-address-lookup-table-interface; bincode, u32 LE variant tag) ---------------------

/** `ProgramInstruction` variants: Create 0, Freeze 1, Extend 2, Deactivate 3, Close 4. */
export const LOOKUP_TABLE_IX = { create: 0, freeze: 1, extend: 2, deactivate: 3, close: 4 } as const;
/** The table account's header before the addresses (`LOOKUP_TABLE_META_SIZE`). */
export const LOOKUP_TABLE_META_SIZE = 56;
/** `LOOKUP_TABLE_MAX_ADDRESSES`: a table holds at most 256 (a v0 index is one byte). */
export const LOOKUP_TABLE_MAX_ADDRESSES = 256;
/** The SlotHashes sysvar: 512 `(slot u64, hash [32])` entries after an 8-byte length. */
export const SLOT_HASHES_SYSVAR = 'SysvarS1otHashes111111111111111111111111111' as Address;
/**
 * The `dataSlice` that reads a `recent_slot` the ALT program accepts: SlotHashes ENTRY 8 (`8 + 8 × 40`), a slot every
 * bank — the simulating one included — still holds. Never the finalized / confirmed slot.
 */
export const LOOKUP_TABLE_RECENT_SLOT_SLICE = { offset: 8 + 8 * 40, length: 8 } as const;
/**
 * A deactivated table closes once its deactivation slot has left SlotHashes (512 entries): from
 * `deactivationSlot + LOOKUP_TABLE_CLOSE_COOLDOWN_SLOTS` on. 513 is one past the window, the safe side.
 */
export const LOOKUP_TABLE_CLOSE_COOLDOWN_SLOTS = 513n;
/** `u64::MAX`: the deactivation slot of a table that is not deactivated. */
export const LOOKUP_TABLE_ACTIVE_SLOT = 0xffff_ffff_ffff_ffffn;
/**
 * CU limits [est.]: a create + extend 40,000, an extend 20,000, deactivate / close 10,000.
 */
export const LOOKUP_TABLE_CU = { createAndExtend: 40_000, extend: 20_000, freeze: 5_000, deactivate: 10_000, close: 10_000 } as const;

const ixData = (tag: number, body: number) => {
  const data = new Uint8Array(4 + body);
  new DataView(data.buffer).setUint32(0, tag, true);
  return data;
};
const alt = (name: string, accountNames: string[], argNames: string[], accounts: ProgramInstruction['accounts'], data: Uint8Array): ProgramInstruction => ({
  name,
  programAddress: ADDRESS_LOOKUP_TABLE_PROGRAM_ID,
  accounts,
  data,
  accountNames,
  argNames,
});

/** `CreateLookupTable { recent_slot, bump_seed }`: [table w, authority s, payer s w, system]. `table` MUST be `findAddressLookupTable(authority, recentSlot)`. */
export function createLookupTableIx(a: { table: Address; authority: Address; payer: Address; recentSlot: bigint; bump: number }): ProgramInstruction {
  const data = ixData(LOOKUP_TABLE_IX.create, 9);
  new DataView(data.buffer).setBigUint64(4, a.recentSlot, true);
  data[12] = a.bump;
  return alt('create_lookup_table', ['lookup_table', 'authority', 'payer', 'system_program'], ['recent_slot', 'bump_seed'], [
    { address: a.table, role: AccountRole.WRITABLE },
    { address: a.authority, role: AccountRole.READONLY_SIGNER },
    { address: a.payer, role: AccountRole.WRITABLE_SIGNER },
    { address: SYSTEM_PROGRAM_ID, role: AccountRole.READONLY },
  ], data);
}

/** `ExtendLookupTable { new_addresses }` (bincode `Vec`: u64 length, then the keys): [table w, authority s, payer s w, system]. */
export function extendLookupTableIx(a: { table: Address; authority: Address; payer: Address; addresses: readonly Address[] }): ProgramInstruction {
  if (a.addresses.length === 0) throw new Error('extendLookupTableIx: no addresses (the ALT program refuses an empty extend)');
  const data = ixData(LOOKUP_TABLE_IX.extend, 8 + 32 * a.addresses.length);
  new DataView(data.buffer).setBigUint64(4, BigInt(a.addresses.length), true);
  a.addresses.forEach((x, i) => data.set(addressToBytes(x), 12 + 32 * i));
  return alt('extend_lookup_table', ['lookup_table', 'authority', 'payer', 'system_program'], ['new_addresses'], [
    { address: a.table, role: AccountRole.WRITABLE },
    { address: a.authority, role: AccountRole.READONLY_SIGNER },
    { address: a.payer, role: AccountRole.WRITABLE_SIGNER },
    { address: SYSTEM_PROGRAM_ID, role: AccountRole.READONLY },
  ], data);
}

/** `FreezeLookupTable`: [table w, authority s]. The authority becomes `None` for good; refused for an empty or deactivated table. */
export function freezeLookupTableIx(a: { table: Address; authority: Address }): ProgramInstruction {
  return alt('freeze_lookup_table', ['lookup_table', 'authority'], [], [
    { address: a.table, role: AccountRole.WRITABLE },
    { address: a.authority, role: AccountRole.READONLY_SIGNER },
  ], ixData(LOOKUP_TABLE_IX.freeze, 0));
}

/** `DeactivateLookupTable`: [table w, authority s]. Starts the cooldown; the table stays usable until it ends. */
export function deactivateLookupTableIx(a: { table: Address; authority: Address }): ProgramInstruction {
  return alt('deactivate_lookup_table', ['lookup_table', 'authority'], [], [
    { address: a.table, role: AccountRole.WRITABLE },
    { address: a.authority, role: AccountRole.READONLY_SIGNER },
  ], ixData(LOOKUP_TABLE_IX.deactivate, 0));
}

/** `CloseLookupTable`: [table w, authority s, recipient w]. Refused on chain until the cooldown ends. */
export function closeLookupTableIx(a: { table: Address; authority: Address; recipient: Address }): ProgramInstruction {
  return alt('close_lookup_table', ['lookup_table', 'authority', 'recipient'], [], [
    { address: a.table, role: AccountRole.WRITABLE },
    { address: a.authority, role: AccountRole.READONLY_SIGNER },
    { address: a.recipient, role: AccountRole.WRITABLE },
  ], ixData(LOOKUP_TABLE_IX.close, 0));
}

// ---- the table account -----------------------------------------------------------------------------------------------

export interface DecodedLookupTable {
  /** `LOOKUP_TABLE_ACTIVE_SLOT` (u64::MAX) while not deactivated. */
  deactivationSlot: bigint;
  lastExtendedSlot: bigint;
  lastExtendedSlotStartIndex: number;
  /** `null` once frozen. */
  authority: Address | null;
  addresses: Address[];
}

/** The ALT program's `LookupTableMeta` (u32 type 1, deactivation u64, last extended u64 + start u8, `Option<Pubkey>`, u16 pad) then the keys. */
export function decodeLookupTable(data: Uint8Array): DecodedLookupTable {
  if (data.length < LOOKUP_TABLE_META_SIZE || (data.length - LOOKUP_TABLE_META_SIZE) % 32 !== 0) {
    throw new Error(`decodeLookupTable: ${data.length} bytes is not a lookup table (56 + 32·n)`);
  }
  const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
  if (dv.getUint32(0, true) !== 1) throw new Error(`decodeLookupTable: account type ${dv.getUint32(0, true)}, expected 1 (LookupTable)`);
  const addresses: Address[] = [];
  for (let o = LOOKUP_TABLE_META_SIZE; o < data.length; o += 32) addresses.push(bytesToAddress(data.subarray(o, o + 32)));
  return {
    deactivationSlot: dv.getBigUint64(4, true),
    lastExtendedSlot: dv.getBigUint64(12, true),
    lastExtendedSlotStartIndex: data[20]!,
    authority: data[21] === 1 ? bytesToAddress(data.subarray(22, 54)) : null,
    addresses,
  };
}

/**
 * Where a table stands at `currentSlot`: `warming` (its newest addresses not usable yet — extended IN this slot),
 * `active`, `deactivating` (still usable, not closeable), `closeable`.
 */
export function lookupTableStatus(t: Pick<DecodedLookupTable, 'deactivationSlot' | 'lastExtendedSlot'>, currentSlot: bigint): 'warming' | 'active' | 'deactivating' | 'closeable' {
  if (t.deactivationSlot !== LOOKUP_TABLE_ACTIVE_SLOT) {
    return currentSlot >= t.deactivationSlot + LOOKUP_TABLE_CLOSE_COOLDOWN_SLOTS ? 'closeable' : 'deactivating';
  }
  return currentSlot > t.lastExtendedSlot ? 'active' : 'warming';
}

/** The `recent_slot` out of the {@link LOOKUP_TABLE_RECENT_SLOT_SLICE} read (8 bytes) or the whole SlotHashes account. */
export function recentSlotFromSlotHashes(data: Uint8Array): bigint {
  const dv = new DataView(data.buffer, data.byteOffset, data.byteLength);
  if (data.length === 8) return dv.getBigUint64(0, true);
  const off = LOOKUP_TABLE_RECENT_SLOT_SLICE.offset;
  if (data.length < off + 8) throw new Error(`recentSlotFromSlotHashes: ${data.length} bytes holds no entry 8`);
  return dv.getBigUint64(off, true);
}

/** `DeactivateLookupTable` (the authority signs). Refused on chain for a FROZEN table: it serves unfrozen tables only. */
export function buildDeactivateLookupTable(a: { authority: Address; table: Address }): BuiltTransaction {
  const main = deactivateLookupTableIx(a);
  return finalizeUnits('buildDeactivateLookupTable', LOOKUP_TABLE_CU.deactivate, [main], main);
}

/** `CloseLookupTable` once {@link lookupTableStatus} says `closeable`: the rent to `recipient` (default the authority). A FROZEN table is never closeable. */
export function buildCloseLookupTable(a: { authority: Address; table: Address; recipient?: Address }): BuiltTransaction {
  const main = closeLookupTableIx({ table: a.table, authority: a.authority, recipient: a.recipient ?? a.authority });
  return finalizeUnits('buildCloseLookupTable', LOOKUP_TABLE_CU.close, [main], main);
}

