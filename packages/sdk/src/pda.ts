/**
 * Every `carpenter_amm` PDA, the hook-interface `["hook_authority"]`, and the foreign derivations (ATA, ProgramData,
 * address lookup table), derived exactly as the programs derive them.
 * Integers in seeds are LITTLE-endian. A pool PDA takes the mints already sorted; `sortMints` is the one place
 *    that sorts.
 */

import { address, getProgramDerivedAddress, type Address } from '@solana/kit';

import { addressToBytes, compareBytes, i32le, u16le, u64le, u32le } from './bytes.ts';
import {
  ASSOCIATED_TOKEN_PROGRAM_ID,
  BPF_LOADER_UPGRADEABLE_ID,
  CARPENTER_AMM_PROGRAM_ID,
  SEEDS,
  SYSTEM_PROGRAM_ID,
} from './constants.ts';

export interface Pda {
  readonly address: Address;
  readonly bump: number;
}

/**
 * Program ids, overridable per deployment (tests, a redeployed devnet). Carpenter needs only its own;
 * a hook's SDK extends this shape with its programs (`@floor/sdk-solana`'s `ProgramIds` does), and any
 * such object is accepted wherever this one is.
 */
export interface ProgramIds {
  readonly carpenterAmm: Address;
}

export const DEFAULT_PROGRAM_IDS: ProgramIds = {
  carpenterAmm: CARPENTER_AMM_PROGRAM_ID,
};

export async function findPda(programAddress: Address, seeds: readonly Uint8Array[]): Promise<Pda> {
  const [addr, bump] = await getProgramDerivedAddress({ programAddress, seeds: [...seeds] });
  return { address: addr, bump: bump as number };
}

const k = addressToBytes;

/** Sort two mints bytewise. Returns `[mint0, mint1]`. Throws on equal mints. */
export function sortMints(a: Address, b: Address): [Address, Address] {
  const c = compareBytes(k(a), k(b));
  if (c === 0) throw new Error('a pool needs two different mints');
  return c < 0 ? [a, b] : [b, a];
}

// ---- carpenter_amm ----------------------------------------------------------------------

export const findAmmConfig = (ids: ProgramIds = DEFAULT_PROGRAM_IDS) => findPda(ids.carpenterAmm, [SEEDS.ammConfig]);
export const findApprovedHook = (hookProgram: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS) =>
  findPda(ids.carpenterAmm, [SEEDS.approvedHook, k(hookProgram)]);
export const findMintExclusivity = (mint: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS) =>
  findPda(ids.carpenterAmm, [SEEDS.mintExclusivity, k(mint)]);
export const findMintPools = (mint: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS) => findPda(ids.carpenterAmm, [SEEDS.mintPools, k(mint)]);
/** The quote ROSTER entry, `["quote_mint", mint]`. */
export const findQuoteMint = (mint: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS) =>
  findPda(ids.carpenterAmm, [SEEDS.quoteMint, k(mint)]);

/**
 * `["pool", mint0, mint1, fee u32 LE, tick_spacing u16 LE, hook]`. `hook` is the
 * all-zero key for a hookless pool.
 * Refuses unsorted mints rather than sorting them: the caller's pool orientation is part of
 *    what it means (currency0 / currency1), so a silent swap would flip every amount.
 */
export async function findPool(
  args: { mint0: Address; mint1: Address; fee: number; tickSpacing: number; hook: Address },
  ids: ProgramIds = DEFAULT_PROGRAM_IDS,
): Promise<Pda> {
  if (compareBytes(k(args.mint0), k(args.mint1)) >= 0) throw new Error('findPool: mint0 must sort below mint1');
  return findPda(ids.carpenterAmm, [SEEDS.pool, k(args.mint0), k(args.mint1), u32le(args.fee), u16le(args.tickSpacing), k(args.hook)]);
}

export const findBitmap = (pool: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS) => findPda(ids.carpenterAmm, [SEEDS.bitmap, k(pool)]);
export const findTickArray = (pool: Address, startIndex: number, ids: ProgramIds = DEFAULT_PROGRAM_IDS) =>
  findPda(ids.carpenterAmm, [SEEDS.tickArray, k(pool), i32le(startIndex)]);
/**
 * `["position", pool, position_id, salt u64 LE]`.
 *
 * The second seed is the POSITION ID, not the owner. They coincide for
 * an ordinary LP, who opens with their own key as the id — but `Position.owner` is a mutable FIELD
 * (positions transfer), while the id is fixed at open, and a hook opens with its `hook_authority`
 * as the id. Deriving from the current owner finds nothing once a position has changed hands.
 */
export const findPosition = (pool: Address, positionId: Address, salt: bigint, ids: ProgramIds = DEFAULT_PROGRAM_IDS) =>
  findPda(ids.carpenterAmm, [SEEDS.position, k(pool), k(positionId), u64le(salt)]);
export const findPoolIndexEntry = (pool: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS) => findPda(ids.carpenterAmm, [SEEDS.poolIndex, k(pool)]);
export const findVault = (pool: Address, mint: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS) =>
  findPda(ids.carpenterAmm, [SEEDS.vault, k(pool), k(mint)]);
/**
 * carpenter_amm's program-owned `["protocol_treasury"]`: the owner of every token account a pool's `protocol_fees_owed` is swept into
 * (`findAta(treasury, mint, tokenProgram)`, off-curve), and the account holding its governance.
 * Under `ids.carpenterAmm`: never floor_hook's platform or team treasury.
 */
export const findProtocolTreasury = (ids: ProgramIds = DEFAULT_PROGRAM_IDS) => findPda(ids.carpenterAmm, [SEEDS.protocolTreasury]);

/**
 * `["hook_settle", pool]` under `carpenter_amm`: the signer the AMM calls a SETTLING hook with (`HOOK_FLAGS.SETTLES`), in
 * place of the pool. It must be the FIRST account of such a pool's hook slice (`HookSettleSignerMissing` otherwise).
 */
export const findHookSettle = (pool: Address, ids: ProgramIds = DEFAULT_PROGRAM_IDS) => findPda(ids.carpenterAmm, [SEEDS.hookSettle, k(pool)]);

/** Anchor's `emit_cpi!` signer, `["__event_authority"]`, under any program. */
export const findEventAuthority = (program: Address) => findPda(program, [SEEDS.eventAuthority]);

/** One per hook program — the signer an approved hook's requests are made by. */
export const findHookAuthority = (hookProgram: Address) => findPda(hookProgram, [SEEDS.hookAuthority]);

// ---- foreign ----------------------------------------------------------------------------------

/** Loader-v3 ProgramData: `[program]` under BPFLoaderUpgradeable. */
export const findProgramData = (program: Address) => findPda(BPF_LOADER_UPGRADEABLE_ID, [k(program)]);

/** The associated token account: `[owner, token program, mint]` under the ATA program. */
export const findAta = (owner: Address, mint: Address, tokenProgram: Address) =>
  findPda(ASSOCIATED_TOKEN_PROGRAM_ID, [k(owner), k(tokenProgram), k(mint)]);

/** The Address Lookup Table program, `AddressLookupTab1e1111111111111111111111111`. */
export const ADDRESS_LOOKUP_TABLE_PROGRAM_ID: Address = address('AddressLookupTab1e1111111111111111111111111');

/**
 * An address lookup table: `[authority, recent_slot u64 LE]` under the ALT program.
 *
 * **THE SLOT IS AN INPUT, SO THIS IS NOT A DEPLOYMENT-DETERMINISTIC ADDRESS.** The ALT program
 *    re-derives the table's address from the `recent_slot` in the `CreateLookupTable` instruction
 *    data and rejects anything else, so the slot must be a real, recent one and two runs by the
 *    same authority produce two different tables. This derivation exists so a creator can PREDICT
 *    the address it is about to create (and so a guard can check one), never so a client can find
 *    an existing table — that address has to be RECORDED, which is what a cluster row's `alt` in
 *    `deployments.json` is for.
 * A slot far behind the chain's head is refused ON CHAIN (`InvalidInstruction`), not here: this
 *   is arithmetic over the seeds and knows nothing about any cluster.
 */
export const findAddressLookupTable = (authority: Address, recentSlot: bigint) =>
  findPda(ADDRESS_LOOKUP_TABLE_PROGRAM_ID, [k(authority), u64le(recentSlot)]);

/** `Pubkey::default`: the `hook` seed of a hookless pool. */
export const NO_HOOK: Address = SYSTEM_PROGRAM_ID;
