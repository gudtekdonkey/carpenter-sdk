/**
 * `predict`: the accounts a swap needs before it is sent — `swapTickArrays`, the union of the trader's walk and, on a
 * hooked pool, every predicted hook action's walk, from a quoter result.
 */

import type { Address } from '@solana/kit';

import { DEFAULT_PROGRAM_IDS, findTickArray, type ProgramIds } from './pda.ts';

/**
 * The tick arrays a swap needs: the union of the trader's walk and every hook action's walk, deduplicated, as PDAs.
 * A router that passes only the walk fails a swap whose hook acts in another tick array (`TickArrayMissing`, 6029). A
 * plain pool's quote (`createQuoter`) has no actions; a hook's quoter reports them as `actionArrays`.
 */
export async function swapTickArrays(
  pool: Address,
  quote: { arraysTouched: readonly number[]; actionArrays?: readonly number[] },
  ids: ProgramIds = DEFAULT_PROGRAM_IDS,
): Promise<{ starts: number[]; addresses: Address[] }> {
  const starts = [...new Set([...quote.arraysTouched, ...(quote.actionArrays ?? [])])];
  const addresses = await Promise.all(starts.map(async (st) => (await findTickArray(pool, st, ids)).address));
  return { starts, addresses };
}
