// Shared test helpers.

import { readFileSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import type { AccountMeta, Address } from '@solana/kit';

import { bytesToAddress } from '../src/bytes.ts';
import {
  closePositionIx,
  closeTickArrayIx,
  initializePoolIx,
  initializeTickArrayIx,
  modifyLiquidityIx,
  swapFromBalanceIx,
  swapIx,
  transferPositionIx,
  type ProgramInstruction,
} from '../src/tx/instructions.ts';

export const SDK_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '..');

/** A deterministic, distinct address per `n` (0xa0, zeros, then `n` big-endian in the last two bytes). */
export function addr(n: number): Address {
  const b = new Uint8Array(32);
  b[0] = 0xa0;
  b[30] = (n >> 8) & 0xff;
  b[31] = n & 0xff;
  return bytesToAddress(b);
}

export const idlPath = (program: 'carpenter_amm'): string => join(SDK_ROOT, 'idl', `${program}.json`);

const A = addr;
const none: readonly AccountMeta[] = [];

/**
 * One instance of every `carpenter_amm` instruction encoder, keyed `carpenter_amm/<instruction>`, on fixed inputs.
 * `idl-parity.test.ts` checks each against the shipped IDL and `golden.test.ts` pins their bytes.
 */
export function everyInstruction(): Record<string, ProgramInstruction> {
  return {
    'carpenter_amm/initialize_tick_array': initializeTickArrayIx({ program: A(1), payer: A(2), pool: A(3), tickArray: A(4), startIndex: -3600, rentOwner: A(5) }),
    'carpenter_amm/close_tick_array': closeTickArrayIx({ program: A(1), eventAuthority: A(2), rentOwner: A(3), pool: A(4), bitmap: A(5), tickArray: A(6) }),
    'carpenter_amm/initialize_pool': initializePoolIx(
      {
        program: A(1), eventAuthority: A(2), payer: A(3), initializer: A(4), config: A(5), pool: A(6), bitmap: A(7), vault0: A(8), vault1: A(9),
        mint0: A(10), mint1: A(11), tokenProgram0: A(12), tokenProgram1: A(13), mintExclusivity0: A(14), mintExclusivity1: A(15),
        mintPools0: A(16), mintPools1: A(17), mintPools0Writable: false, mintPools1Writable: true, poolIndexEntry: A(18),
        approvedHook: A(19), hookProgram: A(20), hookProgramdata: A(21), quoteMint0: A(22), quoteMint1: A(23), hookSlice: none,
      },
      { fee: 0x800000, tickSpacing: 60, hook: A(20), sqrtPriceX64: 1n << 64n },
    ),
    'carpenter_amm/modify_liquidity': modifyLiquidityIx(
      {
        program: A(1), eventAuthority: A(2), owner: A(3), payer: A(4), pool: A(5), bitmap: A(6), position: A(7), tickArrayLower: A(8),
        tickArrayUpper: A(9), ownerToken0: A(10), ownerToken1: A(11), vault0: A(12), vault1: A(13), mint0: A(14), mint1: A(15),
        tokenProgram0: A(16), tokenProgram1: A(17), hookProgram: null, positionAuthority: null, hookSlice: none,
      },
      { positionId: new Uint8Array(32).fill(3), salt: 1n, tickLower: -60, tickUpper: 60, liquidityDelta: 1000n, amount0Max: 1n, amount1Max: 1n, rentOwner: A(18) },
    ),
    'carpenter_amm/close_position': closePositionIx({ program: A(1), eventAuthority: A(2), owner: A(3), position: A(4) }),
    'carpenter_amm/transfer_position': transferPositionIx({ program: A(1), eventAuthority: A(2), owner: A(3), position: A(4), pool: A(5) }, A(6)),
    'carpenter_amm/swap': swapIx(
      {
        program: A(1), eventAuthority: A(2), trader: A(3), pool: A(4), bitmap: A(5), traderTokenIn: A(6), traderTokenOut: A(7), vault0: A(8),
        vault1: A(9), mint0: A(10), mint1: A(11), tokenProgram0: A(12), tokenProgram1: A(13), hookProgram: A(14), hookSlice: none,
        tickArrays: [], actionPositions: [],
      },
      { zeroForOne: true, amountSpecified: -5n, sqrtPriceLimitX64: 4295048017n, amountLimit: 0n },
    ),
    'carpenter_amm/swap_from_balance': swapFromBalanceIx(
      {
        program: A(1), eventAuthority: A(2), trader: A(3), pool: A(4), bitmap: A(5), traderTokenIn: A(6), traderTokenOut: A(7), vault0: A(8),
        vault1: A(9), mint0: A(10), mint1: A(11), tokenProgram0: A(12), tokenProgram1: A(13), hookProgram: A(14), hookSlice: none,
        tickArrays: [], actionPositions: [],
      },
      { zeroForOne: true, keep: 7n, sqrtPriceLimitX64: 4295048017n, amountLimit: 3n, limitInBasis: 11n },
    ),
  };
}

// ---- the shipped IDL ------------------------------------------------------------------------

export type IdlType = string | { array: [IdlType, number] } | { vec: IdlType } | { option: IdlType } | { defined: { name: string } };
export interface IdlAccount { name: string; writable?: boolean; signer?: boolean; optional?: boolean; accounts?: IdlAccount[] }
export interface IdlInstruction { name: string; discriminator: number[]; accounts: IdlAccount[]; args: { name: string; type: IdlType }[] }
export interface Idl {
  instructions: IdlInstruction[];
  types: { name: string; type: { kind: string; fields?: { name: string; type: IdlType }[] } }[];
}

export const readIdl = (): Idl => JSON.parse(readFileSync(idlPath('carpenter_amm'), 'utf8')) as Idl;

/** The IDL instruction's accounts, nested groups flattened, in order. */
export const flatAccounts = (accs: readonly IdlAccount[]): IdlAccount[] => accs.flatMap((a) => (a.accounts ? flatAccounts(a.accounts) : [a]));

/**
 * Decode `data` (after the discriminator) by the IDL's arg types, Borsh. Returns the values keyed by arg name and
 * throws when the bytes run short or are not consumed exactly.
 */
export function decodeIdlArgs(idl: Idl, ix: IdlInstruction, data: Uint8Array): Record<string, unknown> {
  let at = 0;
  const need = (n: number) => {
    if (at + n > data.length) throw new Error(`${ix.name}: args run past the data (${at} + ${n} > ${data.length})`);
  };
  const uint = (n: number, signed: boolean) => {
    need(n);
    let v = 0n;
    for (let i = n - 1; i >= 0; i--) v = (v << 8n) | BigInt(data[at + i]!);
    at += n;
    if (signed && v >= 1n << BigInt(8 * n - 1)) v -= 1n << BigInt(8 * n);
    return v;
  };
  const read = (t: IdlType): unknown => {
    if (typeof t === 'string') {
      switch (t) {
        case 'bool': need(1); return data[at++] === 1;
        case 'u8': return Number(uint(1, false));
        case 'u16': return Number(uint(2, false));
        case 'u32': return Number(uint(4, false));
        case 'i32': return Number(uint(4, true));
        case 'u64': return uint(8, false);
        case 'i64': return uint(8, true);
        case 'u128': return uint(16, false);
        case 'i128': return uint(16, true);
        case 'pubkey': need(32); return bytesToAddress(data.slice(at, (at += 32)));
        case 'bytes': {
          const n = Number(uint(4, false));
          need(n);
          return data.slice(at, (at += n));
        }
        default: throw new Error(`${ix.name}: IDL type ${t} not handled`);
      }
    }
    if ('array' in t) return Array.from({ length: t.array[1] }, () => read(t.array[0]));
    if ('vec' in t) return Array.from({ length: Number(uint(4, false)) }, () => read(t.vec));
    if ('option' in t) {
      need(1);
      return data[at++] === 1 ? read(t.option) : null;
    }
    const def = idl.types.find((x) => x.name === t.defined.name);
    if (!def || def.type.kind !== 'struct' || !def.type.fields) throw new Error(`${ix.name}: defined type ${t.defined.name} not a struct`);
    return Object.fromEntries(def.type.fields.map((f) => [f.name, read(f.type)]));
  };
  const out = Object.fromEntries(ix.args.map((a) => [a.name, read(a.type)]));
  if (at !== data.length) throw new Error(`${ix.name}: ${data.length - at} bytes left after the IDL's args`);
  return out;
}
