/**
 * The transaction format: v0, compressed through any number of address lookup tables, and the 1,232-byte packet gate
 * (`messageSize`, `PacketTooLargeError`, `compileSendable`).
 *
 * Signers are never moved into a lookup table (kit's compressor refuses them), so the fee payer and the trader stay
 *    static.
 * `messageSize` compiles an UNSIGNED transaction; the 64-byte signatures are counted in it (kit's
 *   `getTransactionSize`), so it is the wire size once signed.
 */

import {
  appendTransactionMessageInstructions,
  compileTransaction,
  compressTransactionMessageUsingAddressLookupTables,
  createTransactionMessage,
  getTransactionSize,
  setTransactionMessageFeePayer,
  setTransactionMessageLifetimeUsingBlockhash,
  type Address,
  type BlockhashLifetimeConstraint,
  type Instruction,
} from '@solana/kit';

import { LEGACY_TX_BYTES } from '../constants.ts';

/** One address lookup table and the addresses it holds, in its STORED order. */
export interface LookupTableContents {
  address: Address;
  addresses: readonly Address[];
}

export interface V0MessageArgs {
  feePayer: Address;
  instructions: readonly Instruction[];
  lifetime: BlockhashLifetimeConstraint;
  /** The deployment ALT and the addresses it holds (read from chain; FLOOR's `deploymentAltAddresses` lists FLOOR's). */
  lookupTable?: LookupTableContents | null;
  /**
   * MORE tables, compressed through together with `lookupTable`. A v0 message may name
   * several; each costs its 32-byte key + two length bytes once, whatever it holds.
   * Each table's `addresses` must be its ON-CHAIN contents in STORED order — kit writes an index into
   *    the list it is handed and the validator reads it in stored order.
   */
  lookupTables?: readonly LookupTableContents[];
}

/** Every table a message is compressed through: `lookupTable` first, then `lookupTables`, deduped by address. */
function tablesOf(a: Pick<V0MessageArgs, 'lookupTable' | 'lookupTables'>): LookupTableContents[] {
  const out: LookupTableContents[] = [];
  for (const t of [...(a.lookupTable ? [a.lookupTable] : []), ...(a.lookupTables ?? [])]) {
    if (!out.some((x) => x.address === t.address)) out.push(t);
  }
  return out;
}

/** Build the v0 message, compressed through the ALT(s) when any is given. */
export function buildV0Message(a: V0MessageArgs) {
  const base = appendTransactionMessageInstructions(
    [...a.instructions],
    setTransactionMessageLifetimeUsingBlockhash(a.lifetime, setTransactionMessageFeePayer(a.feePayer, createTransactionMessage({ version: 0 }))),
  );
  const tables = tablesOf(a);
  if (tables.length === 0) return base;
  return compressTransactionMessageUsingAddressLookupTables(base, Object.fromEntries(tables.map((t) => [t.address, [...t.addresses]])));
}

/** Serialized size of the message once signed, and whether it fits the 1,232-byte packet. */
export function messageSize(message: ReturnType<typeof buildV0Message>): { bytes: number; fitsLegacyPacket: boolean } {
  const bytes = getTransactionSize(compileTransaction(message));
  return { bytes, fitsLegacyPacket: bytes <= LEGACY_TX_BYTES };
}

/** How many bytes over the packet a size is; 0 when it fits. The number a refusal quotes. */
export function packetOverage(bytes: number): number {
  return Math.max(0, bytes - LEGACY_TX_BYTES);
}

/**
 * The packet gate as a REFUSAL, not a boolean somebody has to remember to read.
 *
 * **WHY THIS EXISTS.** `messageSize().fitsLegacyPacket` is advisory: a caller that never looks
 *    at it builds an oversized transaction, and the only thing that says so is the validator's
 *    ingest, which rejects it by size with no program error and no logs — the least informative
 *    failure on the platform. Some instructions (FLOOR's `create_launch`) are over the packet at
 *    EVERY legal input, so for those this is not a guard against a rare case, it is the normal path.
 * The message NAMES which of the two failures it is — no table supplied, or over even with the
 *    table given — because the fixes are different: create the cluster's table, or shorten the
 *    instruction.
 */
export class PacketTooLargeError extends Error {
  readonly kind: string;
  readonly bytes: number;
  readonly limit: number = LEGACY_TX_BYTES;
  readonly overBy: number;
  readonly hadLookupTable: boolean;
  constructor(kind: string, bytes: number, hadLookupTable: boolean) {
    const over = bytes - LEGACY_TX_BYTES;
    super(
      `${kind}: ${bytes} B is ${over} B over Solana's ${LEGACY_TX_BYTES}-byte packet — ` +
        (hadLookupTable
          ? 'even through the address lookup table given; the instruction itself has to shrink'
          : "no address lookup table was supplied, and this transaction does not fit without one (create the cluster's table)"),
    );
    this.name = 'PacketTooLargeError';
    this.kind = kind;
    this.bytes = bytes;
    this.overBy = over;
    this.hadLookupTable = hadLookupTable;
  }
}

/**
 * Build, compile and REFUSE by name if the wire form does not fit a packet.
 *
 * It returns the compiled transaction, so a caller that uses it cannot end up measuring one
 *   message and sending another.
 */
export function compileSendable(kind: string, a: V0MessageArgs): { transaction: ReturnType<typeof compileTransaction>; bytes: number } {
  const message = buildV0Message(a);
  const transaction = compileTransaction(message);
  const bytes = getTransactionSize(transaction);
  if (bytes > LEGACY_TX_BYTES) throw new PacketTooLargeError(kind, bytes, tablesOf(a).length > 0);
  return { transaction, bytes };
}
