/**
 * What every transaction builder shares: the `BuiltTransaction` shape, the 64-account lock check and the
 * `SetComputeUnitLimit` prefix (`finalize` over `CU_LIMITS`). `builders.ts` re-exports every name here.
 */

import type { Address, Instruction } from '@solana/kit';

import { CU_LIMITS, HEAP_FRAMES, LEGACY_TX_BYTES, MAX_ACCOUNT_LOCKS, MAX_TX_CU, type CuLimitKey } from '../constants.ts';
import type { ProgramInstruction } from './instructions.ts';
import { requestHeapFrameIx, setComputeUnitLimitIx } from './spl.ts';

export interface BuiltTransaction {
  readonly instructions: Instruction[];
  /** Every unique account the message references (program ids included), first-seen order. */
  readonly accounts: Address[];
  readonly cuLimit: number;
  /** The `RequestHeapFrame` sent (`HEAP_FRAMES`), or `null`: the default 32 KiB heap. */
  readonly heapFrame: number | null;
  readonly addressCount: number;
  /** The program instruction the transaction exists for. */
  readonly main: ProgramInstruction;
}

export class TooManyAccountsError extends Error {
  readonly count: number;
  /**
   * Set when the limit a batch hit is the 1,232-byte PACKET rather than the 64 locks — the
   * wire size it measured. `buildCloseTickArrays` packs one unique account per close, and the packet
   * fills first. `null` for the lock limit.
   */
  readonly packetBytes: number | null;
  constructor(what: string, count: number, packetBytes: number | null = null) {
    super(
      packetBytes === null
        ? `${what}: ${count} unique accounts exceeds the ${MAX_ACCOUNT_LOCKS}-account lock limit`
        : `${what}: ${count} unique accounts make a ${packetBytes}-byte transaction, over Solana's ${LEGACY_TX_BYTES}-byte packet — send fewer per transaction`,
    );
    this.name = 'TooManyAccountsError';
    this.count = count;
    this.packetBytes = packetBytes;
  }
}

/** Unique addresses across instructions, program ids included (they are locked too). */
export function uniqueAccounts(instructions: readonly Instruction[]): Address[] {
  const seen = new Set<string>();
  const out: Address[] = [];
  const add = (a: Address) => {
    if (!seen.has(a)) {
      seen.add(a);
      out.push(a);
    }
  };
  for (const ix of instructions) {
    add(ix.programAddress);
    for (const acc of ix.accounts ?? []) add(acc.address);
  }
  return out;
}

/**
 * Every builder: `SetComputeUnitLimit` from `CU_LIMITS[cu]` and — for the keys `HEAP_FRAMES` lists — a
 * `RequestHeapFrame` right after it. This is the ONE `finalize`.
 */
export function finalize(what: string, cu: CuLimitKey, body: readonly Instruction[], main: ProgramInstruction): BuiltTransaction {
  return finalizeUnits(what, CU_LIMITS[cu], body, main, HEAP_FRAMES[cu] ?? null);
}

/**
 * `finalize` for a COMPOSED transaction whose limit is a SUM of `CU_LIMITS` rows (a swap plus the
 * `touch` that opens the buyer's record; a claim-all of N claims). Capped at the 1.4 M per-transaction
 * ceiling — the runtime refuses a larger request, it does not clamp it.
 */
export function finalizeUnits(
  what: string,
  units: number,
  body: readonly Instruction[],
  main: ProgramInstruction,
  heapFrame: number | null = null,
): BuiltTransaction {
  const cuLimit = Math.min(units, MAX_TX_CU);
  const budget: Instruction[] = heapFrame === null ? [setComputeUnitLimitIx(cuLimit)] : [setComputeUnitLimitIx(cuLimit), requestHeapFrameIx(heapFrame)];
  const instructions: Instruction[] = [...budget, ...body];
  const accounts = uniqueAccounts(instructions);
  if (accounts.length > MAX_ACCOUNT_LOCKS) throw new TooManyAccountsError(what, accounts.length);
  return { instructions, accounts, cuLimit, heapFrame, addressCount: accounts.length, main };
}

/** The locks {@link finalize} would count for `body` under `cu`'s budget instructions — program ids included. */
export function lockCount(cu: CuLimitKey, body: readonly Instruction[]): number {
  const heapFrame = HEAP_FRAMES[cu] ?? null;
  const limit = Math.min(CU_LIMITS[cu], MAX_TX_CU);
  const budget: Instruction[] = heapFrame === null ? [setComputeUnitLimitIx(limit)] : [setComputeUnitLimitIx(limit), requestHeapFrameIx(heapFrame)];
  return uniqueAccounts([...budget, ...body]).length;
}

/** The `CU_LIMITS` rows a composed transaction spends, summed (never above the per-transaction cap). */
export function sumCu(...keys: readonly CuLimitKey[]): number {
  return Math.min(keys.reduce((n, k) => n + CU_LIMITS[k], 0), MAX_TX_CU);
}

