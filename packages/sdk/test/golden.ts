// The golden vectors: every derivation and encoding a client depends on, computed from fixed inputs, so a change in a
// PDA seed, an account order, a discriminator, a layout size or the tick math shows up as a diff against
// `fixtures/golden.json`, reviewed and never silently absorbed. The stored values were recorded by the SDK this package
// was split from; `golden.test.ts` recomputes them here.

import { address, type Address } from '@solana/kit';

import { bytesToHex } from '../src/bytes.ts';
import { WSOL_MINT } from '../src/constants.ts';
import { EVENT_NAMES, eventDiscriminator } from '../src/events.ts';
import { LAYOUTS } from '../src/layouts.ts';
import {
  DEFAULT_PROGRAM_IDS,
  findAmmConfig,
  findApprovedHook,
  findBitmap,
  findEventAuthority,
  findHookAuthority,
  findMintExclusivity,
  findMintPools,
  findPoolIndexEntry,
  findPosition,
  findProgramData,
  findQuoteMint,
  findTickArray,
  findVault,
} from '../src/pda.ts';
import { getSqrtPriceAtTick, getTickAtSqrtPrice } from '../src/tickmath.ts';
import { everyInstruction } from './helpers.ts';

export const GOLDEN_VERSION = 1;

/** Fixed inputs: a pool and its base mint, an approved hook program, a quote-roster mint (8 decimals). */
export const FIXTURE = {
  pool: '9SYeJ763BMa7CqfCHxTWUu3MXNiRtF7xscLwP6quvd6t' as Address,
  mint: '3a8z9SYUzHmvXAZP9VadX5NqwKjN7VpUYYULM1vcywLt' as Address,
  hook: '6foted69N6wB1oJzsxccbXUxefBt3Hkn7tN2yHifXnTS' as Address,
  quoteMint: 'CaF1bz1E5KwZfsnm18n9WQU3vu3VrW7pudixqc8BERAV' as Address,
  /** Two position salts of the hook authority. */
  salts: { bid: 1n, seed: 3n },
} as const;

/** JSON-safe: bigint → `"<n>n"`, Uint8Array → hex, else as is. */
export function toJson(v: unknown): unknown {
  if (typeof v === 'bigint') return `${v}n`;
  if (v instanceof Uint8Array) return bytesToHex(v);
  if (Array.isArray(v)) return v.map(toJson);
  if (v && typeof v === 'object') return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, toJson(x)]));
  return v;
}

/** The vector was recorded on devnet: every address is derived under devnet's program ids, never the default. */
export const GOLDEN_PROGRAM_IDS = { ...DEFAULT_PROGRAM_IDS, carpenterAmm: address('Arv1M4LifBrWMywvxpeAGvh37aVHPjpXPCg7sacbe3Hy') };

export async function computeGolden(): Promise<Record<string, unknown>> {
  const ids = GOLDEN_PROGRAM_IDS;
  const { pool, mint, hook, quoteMint } = FIXTURE;
  const pdas: Record<string, { address: string; bump: number }> = {};
  const put = async (name: string, p: Promise<{ address: string; bump: number }>) => {
    const r = await p;
    pdas[name] = { address: r.address, bump: r.bump };
  };
  const hookAuthority = (await findHookAuthority(hook)).address;
  await put('amm_config', findAmmConfig(ids));
  await put('approved_hook[floor_hook]', findApprovedHook(hook, ids));
  await put('hook_authority', findHookAuthority(hook));
  await put('programdata[floor_hook]', findProgramData(hook));
  await put('event_authority[carpenter_amm]', findEventAuthority(ids.carpenterAmm));
  await put('mint_pools[wsol]', findMintPools(WSOL_MINT, ids));
  await put('mint_excl[wsol]', findMintExclusivity(WSOL_MINT, ids));
  await put('quote_mint[backed-xstock-8dp]', findQuoteMint(quoteMint, ids));
  await put('bitmap[fixture]', findBitmap(pool, ids));
  await put('vault0[fixture]', findVault(pool, WSOL_MINT, ids));
  await put('vault1[fixture]', findVault(pool, mint, ids));
  await put('pool_index[fixture]', findPoolIndexEntry(pool, ids));
  await put('tick_array[fixture,-3600]', findTickArray(pool, -3600, ids));
  await put('tick_array[fixture,0]', findTickArray(pool, 0, ids));
  await put('position[fixture,bid]', findPosition(pool, hookAuthority, FIXTURE.salts.bid, ids));
  await put('position[fixture,seed]', findPosition(pool, hookAuthority, FIXTURE.salts.seed, ids));

  const instructions: Record<string, unknown> = {};
  for (const [key, ix] of Object.entries(everyInstruction())) {
    instructions[key] = {
      program: ix.programAddress,
      data: bytesToHex(ix.data),
      accounts: ix.accounts.map((a) => ({ address: a.address, role: a.role })),
      names: ix.accountNames,
      args: ix.argNames,
    };
  }
  return toJson({
    version: GOLDEN_VERSION,
    programIds: { carpenterAmm: ids.carpenterAmm },
    fixture: { pool, mint, hook, quoteMint },
    pdas,
    instructions,
    events: { carpenter_amm: Object.fromEntries(EVENT_NAMES.carpenter_amm.map((n) => [n, bytesToHex(eventDiscriminator(n))])) },
    layouts: Object.fromEntries(LAYOUTS.map((l) => [l.name, { size: l.size, discriminator: bytesToHex(l.discriminator) }])),
    tickMath: {
      sqrtAt: Object.fromEntries([-443_636, -69_081, -1, 0, 1, 6932, 89_231, 443_636].map((t) => [t, getSqrtPriceAtTick(t)])),
      tickAt: Object.fromEntries([4_295_048_016n, 1n << 64n, (1n << 64n) + 1n, 79_226_673_521_066_979_257_578_248_090n].map((s) => [String(s), getTickAtSqrtPrice(s)])),
    },
  }) as Record<string, unknown>;
}
