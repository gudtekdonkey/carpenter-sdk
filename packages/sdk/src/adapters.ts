/**
 * Glue for the deployment rows and the quoter: a deployment row → `ProgramIds`, and an `AccountSource` → the quoter's
 * inputs for a PLAIN pool.
 *
 * The quoter takes RAW ACCOUNT BYTES (pool, bitmap, tick arrays), never decoded structs: it decodes them itself with
 *    the same layouts the program uses, so a field cannot be mis-transcribed between a read and a quote. These helpers
 *    gather the bytes in as few RPC calls as possible and tag them with the OLDEST slot among the reads.
 * Program ids come from `deployments.json`, never a constant; `DEFAULT_PROGRAM_IDS` exists for tests and scripts.
 * A hooked pool needs its hook's accounts as well: read it with that hook's SDK (`@floor/sdk-solana` for FLOOR pools).
 */

import { address, type Address } from '@solana/kit';

import { addressToBytes } from './bytes.ts';
import type { ClusterDeployment } from './deployments.ts';
import { decodePool, decodeTickArrayBitmap, TICK_ARRAY_LEN, TICK_ARRAY_RENT_OWNER_OFFSET } from './layouts.ts';
import { DEFAULT_PROGRAM_IDS, findBitmap, findTickArray, NO_HOOK, type ProgramIds } from './pda.ts';
import type { SwapQuoteInput } from './quote/quoter.ts';
import type { AccountSource, RawAccount } from './source.ts';
import type { AtSlot } from './stateView.ts';
import { initializedArrayStarts, tickArrayStartIndex } from './tickmath.ts';

/** A Carpenter deployment row's program ids (`deployments.json`, `deploymentFor`). */
export function programIdsFromDeployment(row: ClusterDeployment): ProgramIds {
  return { carpenterAmm: address(row.programs.carpenterAmm) };
}

/** Everything `quoteSwap` needs for one pool, as bytes, plus what was found. */
export interface SwapQuoteAccounts extends Omit<SwapQuoteInput, 'zeroForOne' | 'amountSpecified' | 'sqrtPriceLimitX64' | 'floorPool' | 'supply1'> {
  poolKey: Uint8Array;
  pool: Uint8Array;
  bitmap: Uint8Array;
  /** Each the 3,888-byte BODY of a 3,920-byte array account — its rent owner is not a quote input. */
  tickArrays: Uint8Array[];
  /** The start index of each entry of `tickArrays`, in the same order. */
  tickArrayStarts: number[];
}

/**
 * Read a PLAIN pool's quoting accounts: the pool, its bitmap, the initialized tick arrays within
 * `radius` arrays of the current tick on both sides (or exactly `starts`). Two round trips (pool + bitmap, then the
 * arrays). a HOOKED pool is refused by name — its quote also needs the hook's own accounts, which
 * the hook's SDK reads (`@floor/sdk-solana`'s `readSwapQuoteAccounts` reads a FLOOR pool's `FloorPool` and base mint).
 * A walk that leaves the window fails `TickArrayMissing{start_index}`; widen `radius` or pass
 *    the named start in `starts` and read again. The quoter says which array it needs.
 */
export async function readSwapQuoteAccounts(
  src: AccountSource,
  pool: Address,
  opts: { radius?: number; starts?: readonly number[]; ids?: ProgramIds } = {},
): Promise<AtSlot<SwapQuoteAccounts>> {
  const ids = opts.ids ?? DEFAULT_PROGRAM_IDS;
  const bitmapAddr = (await findBitmap(pool, ids)).address;
  const first = await src.getMultipleAccounts([pool, bitmapAddr]);
  const poolRaw = owned(first.accounts[0], ids.carpenterAmm);
  const bitmapRaw = owned(first.accounts[1], ids.carpenterAmm);
  if (!poolRaw) throw new Error(`readSwapQuoteAccounts: no pool at ${pool}`);
  if (!bitmapRaw) throw new Error(`readSwapQuoteAccounts: no bitmap for ${pool}`);
  const p = decodePool(poolRaw.data);
  const bm = decodeTickArrayBitmap(bitmapRaw.data);
  const s = p.tickSpacing;
  if (p.hook !== NO_HOOK) throw new Error(`readSwapQuoteAccounts: ${pool} is hooked by ${p.hook} — read it with that hook's SDK, which adds the hook's accounts`);

  let starts: number[];
  if (opts.starts) {
    starts = [...new Set(opts.starts)].sort((a, b) => a - b);
  } else {
    const radius = opts.radius ?? 3;
    const here = tickArrayStartIndex(p.tickCurrent, s);
    const span = 60 * s;
    starts = initializedArrayStarts(bm.words, s).filter((st) => Math.abs(st - here) <= radius * span);
  }
  const arrayAddrs = await Promise.all(starts.map(async (st) => (await findTickArray(pool, st, ids)).address));
  const second = await src.getMultipleAccounts(arrayAddrs);

  const tickArrays: Uint8Array[] = [];
  const tickArrayStarts: number[] = [];
  starts.forEach((st, i) => {
    const a = owned(second.accounts[i], ids.carpenterAmm);
    // An array is the 3,920-byte account — the program refuses a 3,888-byte V1 account, so it is skipped here
    // like any other length, never truncated. The quoter is handed the 3,888-byte BODY (`TICK_ARRAY_BODY_LEN`).
    if (a && a.data.length === TICK_ARRAY_LEN) {
      tickArrays.push(a.data.subarray(0, TICK_ARRAY_RENT_OWNER_OFFSET));
      tickArrayStarts.push(st);
    }
  });
  const slot = first.slot < second.slot ? first.slot : second.slot;
  return {
    slot,
    value: { poolKey: addressToBytes(pool), pool: poolRaw.data, bitmap: bitmapRaw.data, tickArrays, tickArrayStarts },
  };
}

function owned(a: RawAccount | null | undefined, program: Address): RawAccount | null {
  return a && a.owner === program ? a : null;
}
