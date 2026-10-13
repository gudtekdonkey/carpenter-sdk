/**
 * The foreign instructions every builder composes around a program instruction: compute budget, system transfer, ATA
 * `CreateIdempotent`, wSOL wrap (`SyncNative`) / unwrap (`CloseAccount`), `ApproveChecked` / `Revoke` for the
 * CPI-Guard delegate path, and `BurnChecked`.
 * Opcodes are the SPL Token instruction enum, shared by Token-2022.
 * Never a plain `Transfer`: Token-2022 mints with extensions (e.g. `Pausable`) require `TransferChecked`.
 */

import { AccountRole, createAddressWithSeed, type AccountMeta, type Address, type Instruction } from '@solana/kit';

import { addressToBytes, bytesToHex, u32le, u64le } from '../bytes.ts';
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  COMPUTE_BUDGET_PROGRAM_ID,
  SPL_TOKEN_PROGRAM_ID,
  SYSTEM_PROGRAM_ID,
  WSOL_MINT,
} from '../constants.ts';

export interface PlainInstruction extends Instruction {
  readonly programAddress: Address;
  readonly accounts: readonly AccountMeta[];
  readonly data: Uint8Array;
}

const m = (address: Address, role: AccountRole): AccountMeta => ({ address, role });

/** ComputeBudget `SetComputeUnitLimit` (tag 2, u32 LE). Every builder prepends one. */
export function setComputeUnitLimitIx(units: number): PlainInstruction {
  if (!Number.isInteger(units) || units <= 0 || units > 1_400_000) throw new RangeError(`bad CU limit ${units}`);
  return { programAddress: COMPUTE_BUDGET_PROGRAM_ID, accounts: [], data: new Uint8Array([2, ...u32le(units)]) };
}

/**
 * ComputeBudget `RequestHeapFrame` (tag 1, u32 LE bytes). The runtime takes a multiple of 1,024 from
 * 32 KiB to 256 KiB and refuses the transaction otherwise, so a bad size is refused HERE, by name.
 * `finalize` adds one for the builders in `HEAP_FRAMES`.
 */
export function requestHeapFrameIx(bytes: number): PlainInstruction {
  if (!Number.isInteger(bytes) || bytes % 1024 !== 0 || bytes < 32 * 1024 || bytes > 256 * 1024) {
    throw new RangeError(`bad heap frame ${bytes}: a multiple of 1024 from 32768 to 262144`);
  }
  return { programAddress: COMPUTE_BUDGET_PROGRAM_ID, accounts: [], data: new Uint8Array([1, ...u32le(bytes)]) };
}

/** ComputeBudget `SetComputeUnitPrice` (tag 3, u64 LE micro-lamports). */
export function setComputeUnitPriceIx(microLamports: bigint): PlainInstruction {
  return { programAddress: COMPUTE_BUDGET_PROGRAM_ID, accounts: [], data: new Uint8Array([3, ...u64le(microLamports)]) };
}

/** System `Transfer` (tag 2 u32, lamports u64). */
export function systemTransferIx(from: Address, to: Address, lamports: bigint): PlainInstruction {
  return {
    programAddress: SYSTEM_PROGRAM_ID,
    accounts: [m(from, AccountRole.WRITABLE_SIGNER), m(to, AccountRole.WRITABLE)],
    data: new Uint8Array([...u32le(2), ...u64le(lamports)]),
  };
}

/** ATA `CreateIdempotent` (tag 1): payer, ata, owner, mint, system, token program. */
export function createAtaIdempotentIx(payer: Address, ata: Address, owner: Address, mint: Address, tokenProgram: Address): PlainInstruction {
  return {
    programAddress: ASSOCIATED_TOKEN_PROGRAM_ID,
    accounts: [
      m(payer, AccountRole.WRITABLE_SIGNER),
      m(ata, AccountRole.WRITABLE),
      m(owner, AccountRole.READONLY),
      m(mint, AccountRole.READONLY),
      m(SYSTEM_PROGRAM_ID, AccountRole.READONLY),
      m(tokenProgram, AccountRole.READONLY),
    ],
    data: new Uint8Array([1]),
  };
}

/** Token `SyncNative` (17). wSOL is SPL Token. */
export function syncNativeIx(account: Address): PlainInstruction {
  return { programAddress: SPL_TOKEN_PROGRAM_ID, accounts: [m(account, AccountRole.WRITABLE)], data: new Uint8Array([17]) };
}

/** Token `CloseAccount` (9): account, destination, owner. */
export function closeAccountIx(account: Address, destination: Address, owner: Address, tokenProgram: Address = SPL_TOKEN_PROGRAM_ID): PlainInstruction {
  return {
    programAddress: tokenProgram,
    accounts: [m(account, AccountRole.WRITABLE), m(destination, AccountRole.WRITABLE), m(owner, AccountRole.READONLY_SIGNER)],
    data: new Uint8Array([9]),
  };
}

/** Token `ApproveChecked` (13, amount u64, decimals u8): source, mint, delegate, owner. */
export function approveCheckedIx(
  source: Address,
  mint: Address,
  delegate: Address,
  owner: Address,
  amount: bigint,
  decimals: number,
  tokenProgram: Address,
): PlainInstruction {
  return {
    programAddress: tokenProgram,
    accounts: [m(source, AccountRole.WRITABLE), m(mint, AccountRole.READONLY), m(delegate, AccountRole.READONLY), m(owner, AccountRole.READONLY_SIGNER)],
    data: new Uint8Array([13, ...u64le(amount), decimals]),
  };
}

/**
 * Token `TransferChecked` (12, amount u64, decimals u8): source, mint, destination, owner — then, for a
 * mint whose `TransferHook` is ARMED, the hook's extra accounts (from the hook's SDK), which
 * Token-2022 finds BY KEY and forwards to the hook's `execute`.
 */
export function transferCheckedIx(
  source: Address,
  mint: Address,
  destination: Address,
  owner: Address,
  amount: bigint,
  decimals: number,
  tokenProgram: Address,
  extra: readonly AccountMeta[] = [],
): PlainInstruction {
  return {
    programAddress: tokenProgram,
    accounts: [m(source, AccountRole.WRITABLE), m(mint, AccountRole.READONLY), m(destination, AccountRole.WRITABLE), m(owner, AccountRole.READONLY_SIGNER), ...extra],
    data: new Uint8Array([12, ...u64le(amount), decimals]),
  };
}

/**
 * Token `BurnChecked` (15, amount u64, decimals u8): account, mint, owner. A burn calls NO transfer
 * hook: a mint whose transfer hook keeps per-holder state needs the hook's own instructions around it.
 */
export function burnCheckedIx(account: Address, mint: Address, owner: Address, amount: bigint, decimals: number, tokenProgram: Address): PlainInstruction {
  return {
    programAddress: tokenProgram,
    accounts: [m(account, AccountRole.WRITABLE), m(mint, AccountRole.WRITABLE), m(owner, AccountRole.READONLY_SIGNER)],
    data: new Uint8Array([15, ...u64le(amount), decimals]),
  };
}

/** Token `Revoke` (5): source, owner. */
export function revokeIx(source: Address, owner: Address, tokenProgram: Address): PlainInstruction {
  return {
    programAddress: tokenProgram,
    accounts: [m(source, AccountRole.WRITABLE), m(owner, AccountRole.READONLY_SIGNER)],
    data: new Uint8Array([5]),
  };
}

/** Wrap `lamports` into `owner`'s wSOL ATA: create-idempotent, transfer, `SyncNative`. The builders here use `tempWsol` instead. */
export function wrapSolIxs(owner: Address, wsolAta: Address, wsolMint: Address, lamports: bigint): PlainInstruction[] {
  return [createAtaIdempotentIx(owner, wsolAta, owner, wsolMint, SPL_TOKEN_PROGRAM_ID), systemTransferIx(owner, wsolAta, lamports), syncNativeIx(wsolAta)];
}

/**
 * Unwrap: close the wSOL ATA back to its owner (every wSOL lamport + the rent returns) — including wSOL the owner held
 * before. The builders here close only the temporary account they created (`tempWsol`).
 */
export function unwrapSolIx(owner: Address, wsolAta: Address): PlainInstruction {
  return closeAccountIx(wsolAta, owner, owner, SPL_TOKEN_PROGRAM_ID);
}

/** An SPL token account's size in bytes. */
export const TOKEN_ACCOUNT_LEN = 165;

/**
 * The rent-exempt minimum of a 165-byte token account at today's rent, `(128 + 165) × 6,960` lamports. The builders fund
 * a temporary wSOL account with it; the account is closed in the same transaction, so every lamport comes back, and an
 * amount above the cluster's current minimum is harmless. Pass the cluster's figure (`rentFromRpc`) to fund exactly.
 */
export const TOKEN_ACCOUNT_RENT_LAMPORTS = 2_039_280n;

/** System `CreateAccountWithSeed` (tag 3): `base`, `seed` (u64-length string), lamports, space, owner. `base` = `from`. */
export function createAccountWithSeedIx(from: Address, created: Address, seed: string, lamports: bigint, space: bigint, owner: Address): PlainInstruction {
  const seedBytes = new TextEncoder().encode(seed);
  if (seedBytes.length > 32) throw new RangeError(`seed "${seed}" is over 32 bytes`);
  return {
    programAddress: SYSTEM_PROGRAM_ID,
    accounts: [m(from, AccountRole.WRITABLE_SIGNER), m(created, AccountRole.WRITABLE)],
    data: new Uint8Array([...u32le(3), ...addressToBytes(from), ...u64le(BigInt(seedBytes.length)), ...seedBytes, ...u64le(lamports), ...u64le(space), ...addressToBytes(owner)]),
  };
}

/** Token `InitializeAccount3` (18, owner pubkey): account, mint. */
export function initializeAccount3Ix(account: Address, mint: Address, owner: Address, tokenProgram: Address = SPL_TOKEN_PROGRAM_ID): PlainInstruction {
  return {
    programAddress: tokenProgram,
    accounts: [m(account, AccountRole.WRITABLE), m(mint, AccountRole.READONLY)],
    data: new Uint8Array([18, ...addressToBytes(owner)]),
  };
}

/** A fresh seed for a temporary wSOL account: unpredictable, so no one can pre-fund its address and block the create. */
export function tempWsolSeed(): string {
  const b = new Uint8Array(12);
  globalThis.crypto.getRandomValues(b);
  return `wsol${bytesToHex(b)}`;
}

export interface TempWsol {
  /** The temporary token account (owner's base address + seed, under the SPL Token program). */
  readonly account: Address;
  readonly seed: string;
  /** Create it with `lamports` + rent and initialize it as `owner`'s wSOL account (its token amount is `lamports`). */
  readonly open: PlainInstruction[];
  /** Close it back to `owner`: every lamport, wrapped or not, returns as SOL. */
  readonly close: PlainInstruction;
}

/**
 * A wSOL account that exists only inside one transaction: created and funded (no `SyncNative` needed — initializing a
 * native-mint account sets its amount from its lamports), used, then closed. The builders use it instead of the
 * owner's wSOL ATA, so they never close an account they did not create and never unwrap wSOL the owner already held.
 */
export async function tempWsol(owner: Address, lamports: bigint, opts: { seed?: string; rentLamports?: bigint } = {}): Promise<TempWsol> {
  const seed = opts.seed ?? tempWsolSeed();
  const account = await createAddressWithSeed({ baseAddress: owner, programAddress: SPL_TOKEN_PROGRAM_ID, seed });
  const rent = opts.rentLamports ?? TOKEN_ACCOUNT_RENT_LAMPORTS;
  return {
    account,
    seed,
    open: [
      createAccountWithSeedIx(owner, account, seed, rent + lamports, BigInt(TOKEN_ACCOUNT_LEN), SPL_TOKEN_PROGRAM_ID),
      initializeAccount3Ix(account, WSOL_MINT, owner, SPL_TOKEN_PROGRAM_ID),
    ],
    close: closeAccountIx(account, owner, owner, SPL_TOKEN_PROGRAM_ID),
  };
}
