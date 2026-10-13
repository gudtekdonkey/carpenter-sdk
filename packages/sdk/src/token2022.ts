/**
 * SPL Token / Token-2022 mint and token-account bytes, and the Token-2022 TLV extensions the builders care about.
 *
 * Layout (spl-token-2022): the base `Mint` is 82 B and the base `Account` 165 B; an extended account pads a Mint to
 * 165 B, then `account_type` (1 = Mint, 2 = Account) at byte 165, then TLV entries `type u16 LE | length u16 LE | value`
 * from byte 166.
 * Extension type numbers are the `ExtensionType` enum order in spl-token-2022. Only the EIGHT the builders depend on
 *    are pinned against the program's tests, which hand each number to the real Token-2022 program; the others are a
 *    transcription of the enum, present for `extensionTypes` reporting only, and nothing branches on them.
 *    Unknown types are listed, never an error.
 */

import type { Address } from '@solana/kit';

import { bytesToAddress, readUintLE } from './bytes.ts';

export const EXTENSION_TYPES = {
  Uninitialized: 0,
  TransferFeeConfig: 1,
  TransferFeeAmount: 2,
  MintCloseAuthority: 3,
  ConfidentialTransferMint: 4,
  ConfidentialTransferAccount: 5,
  DefaultAccountState: 6,
  ImmutableOwner: 7,
  MemoTransfer: 8,
  NonTransferable: 9,
  InterestBearingConfig: 10,
  CpiGuard: 11,
  PermanentDelegate: 12,
  NonTransferableAccount: 13,
  TransferHook: 14,
  TransferHookAccount: 15,
  ConfidentialTransferFeeConfig: 16,
  ConfidentialTransferFeeAmount: 17,
  MetadataPointer: 18,
  TokenMetadata: 19,
  GroupPointer: 20,
  TokenGroup: 21,
  GroupMemberPointer: 22,
  TokenGroupMember: 23,
  ConfidentialMintBurn: 24,
  ScaledUiAmountConfig: 25,
  Pausable: 26,
  PausableAccount: 27,
} as const;

const MINT_BASE_LEN = 82;
const ACCOUNT_BASE_LEN = 165;
const ACCOUNT_TYPE_MINT = 1;
const ACCOUNT_TYPE_ACCOUNT = 2;

export interface TlvEntry {
  readonly type: number;
  readonly value: Uint8Array;
}

/** `COption<Pubkey>`: a 4-byte tag then 32 bytes. */
function readCOptionPubkey(data: Uint8Array, at: number): Address | null {
  return readUintLE(data, at, 4) === 1n ? bytesToAddress(data.slice(at + 4, at + 36)) : null;
}

/** `OptionalNonZeroPubkey`: all-zero means none. */
function readOptionalNonZero(data: Uint8Array, at: number): Address | null {
  const b = data.slice(at, at + 32);
  return b.every((x) => x === 0) ? null : bytesToAddress(b);
}

function parseTlv(data: Uint8Array, expectType: number, owner: string): TlvEntry[] {
  if (data.length <= ACCOUNT_BASE_LEN) return [];
  const accountType = data[ACCOUNT_BASE_LEN];
  if (accountType !== expectType) throw new Error(`${owner}: account_type ${accountType}, expected ${expectType}`);
  const out: TlvEntry[] = [];
  let at = ACCOUNT_BASE_LEN + 1;
  while (at + 4 <= data.length) {
    const type = Number(readUintLE(data, at, 2));
    const len = Number(readUintLE(data, at + 2, 2));
    if (type === EXTENSION_TYPES.Uninitialized && len === 0) break;
    if (at + 4 + len > data.length) throw new Error(`${owner}: TLV entry overruns the account`);
    out.push({ type, value: data.slice(at + 4, at + 4 + len) });
    at += 4 + len;
  }
  return out;
}

export interface MintInfo {
  mintAuthority: Address | null;
  supply: bigint;
  decimals: number;
  isInitialized: boolean;
  freezeAuthority: Address | null;
  /** Every TLV type present (Token-2022 only). */
  extensionTypes: number[];
  metadataPointer: { authority: Address | null; metadataAddress: Address | null } | null;
  pausable: { authority: Address | null; paused: boolean } | null;
  /** Token-2022 `TokenMetadata` (name / symbol / uri); `null` when absent. */
  tokenMetadata: { updateAuthority: Address | null; mint: Address; name: string; symbol: string; uri: string } | null;
  transferHook: boolean;
  /**
   * The `TransferHook` extension's two keys: `authority` may
   * re-point the hook, `programId` is the ARMED program (`null` = an EMPTY slot, which Token-2022 never
   * calls). A FLOOR launch mint with a holder rate carries the slot EMPTY with `authority =
   * floor_launch`'s `lock_authority` until graduation arms it with `floor_holders` and renounces
   * the authority. `null` when the mint has no `TransferHook` extension at all.
   */
  transferHookConfig: { authority: Address | null; programId: Address | null } | null;
  transferFee: boolean;
}

export function decodeMint(data: Uint8Array): MintInfo {
  if (data.length < MINT_BASE_LEN) throw new Error(`mint: ${data.length} bytes`);
  const tlv = data.length > MINT_BASE_LEN ? parseTlv(data, ACCOUNT_TYPE_MINT, 'mint') : [];
  const find = (t: number) => tlv.find((e) => e.type === t);
  const mp = find(EXTENSION_TYPES.MetadataPointer);
  const pa = find(EXTENSION_TYPES.Pausable);
  const tm = find(EXTENSION_TYPES.TokenMetadata);
  const th = find(EXTENSION_TYPES.TransferHook);
  return {
    mintAuthority: readCOptionPubkey(data, 0),
    supply: readUintLE(data, 36, 8),
    decimals: data[44] as number,
    isInitialized: data[45] === 1,
    freezeAuthority: readCOptionPubkey(data, 46),
    extensionTypes: tlv.map((e) => e.type),
    metadataPointer: mp ? { authority: readOptionalNonZero(mp.value, 0), metadataAddress: readOptionalNonZero(mp.value, 32) } : null,
    pausable: pa ? { authority: readOptionalNonZero(pa.value, 0), paused: pa.value[32] === 1 } : null,
    tokenMetadata: tm ? decodeTokenMetadata(tm.value) : null,
    transferHook: !!th,
    transferHookConfig: th ? { authority: readOptionalNonZero(th.value, 0), programId: readOptionalNonZero(th.value, 32) } : null,
    transferFee: !!find(EXTENSION_TYPES.TransferFeeConfig),
  };
}

/** Spl-token-metadata-interface `TokenMetadata` (Borsh): update_authority, mint, name, symbol, uri, additional. */
function decodeTokenMetadata(v: Uint8Array): MintInfo['tokenMetadata'] {
  let at = 64;
  const str = (): string => {
    const n = Number(readUintLE(v, at, 4));
    const s = new TextDecoder().decode(v.slice(at + 4, at + 4 + n));
    at += 4 + n;
    return s;
  };
  const name = str();
  const symbol = str();
  const uri = str();
  return { updateAuthority: readOptionalNonZero(v, 0), mint: bytesToAddress(v.slice(32, 64)), name, symbol, uri };
}

export interface TokenAccountInfo {
  mint: Address;
  owner: Address;
  amount: bigint;
  delegate: Address | null;
  state: number;
  isNative: bigint | null;
  delegatedAmount: bigint;
  closeAuthority: Address | null;
  extensionTypes: number[];
  /** `CpiGuard.lock_cpi`: the AMM and the sale must move this account's tokens as delegate. */
  cpiGuardLocked: boolean;
  /** `MemoTransfer.require_incoming_transfer_memos`. */
  memoRequired: boolean;
  immutableOwner: boolean;
}

export function decodeTokenAccount(data: Uint8Array): TokenAccountInfo {
  if (data.length < ACCOUNT_BASE_LEN) throw new Error(`token account: ${data.length} bytes`);
  const tlv = parseTlv(data, ACCOUNT_TYPE_ACCOUNT, 'token account');
  const find = (t: number) => tlv.find((e) => e.type === t);
  const cpi = find(EXTENSION_TYPES.CpiGuard);
  const memo = find(EXTENSION_TYPES.MemoTransfer);
  return {
    mint: bytesToAddress(data.slice(0, 32)),
    owner: bytesToAddress(data.slice(32, 64)),
    amount: readUintLE(data, 64, 8),
    delegate: readCOptionPubkey(data, 72),
    state: data[108] as number,
    isNative: readUintLE(data, 109, 4) === 1n ? readUintLE(data, 113, 8) : null,
    delegatedAmount: readUintLE(data, 121, 8),
    closeAuthority: readCOptionPubkey(data, 129),
    extensionTypes: tlv.map((e) => e.type),
    cpiGuardLocked: cpi ? cpi.value[0] === 1 : false,
    memoRequired: memo ? memo.value[0] === 1 : false,
    immutableOwner: !!find(EXTENSION_TYPES.ImmutableOwner),
  };
}
