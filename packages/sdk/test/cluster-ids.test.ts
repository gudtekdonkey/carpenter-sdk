// A deployment row whose program ids are NOT the shipped IDL's (another cluster): `npm run generate`'s rules accept it
// with its derivable accounts checked against ITS ids, and every public builder, planner and PDA given
// `programIdsFromDeployment(row)` names that row's programs only: no devnet id reaches any instruction.

import assert from 'node:assert/strict';
import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';

import { blockhash, getBase64Encoder, getProgramDerivedAddress, type Address, type Instruction } from '@solana/kit';

import { validateDeployments } from '../scripts/validate-deployments.mjs';
import { programIdsFromDeployment } from '../src/adapters.ts';
import { addressToBytes, bytesToAddress } from '../src/bytes.ts';
import { SPL_TOKEN_PROGRAM_ID, TOKEN_2022_PROGRAM_ID, WSOL_MINT } from '../src/constants.ts';
import { DEPLOYMENTS, deploymentFor, DEVNET_CHAIN_KEY, MAINNET_CHAIN_KEY, type ClusterDeployment, type DeploymentsFile } from '../src/deployments.ts';
import { getInitializePoolInstructionAsync } from '../src/generated/client/carpenter_amm/index.js';
import { CARPENTER_AMM_IDL } from '../src/generated/idls.ts';
import {
  DEFAULT_PROGRAM_IDS,
  findAmmConfig,
  findApprovedHook,
  findBitmap,
  findEventAuthority,
  findMintExclusivity,
  findMintPools,
  findPool,
  findPoolIndexEntry,
  findPosition,
  findProtocolTreasury,
  findQuoteMint,
  findTickArray,
  findVault,
  NO_HOOK,
  type ProgramIds,
} from '../src/pda.ts';
import { swapTickArrays } from '../src/predict.ts';
import { getSqrtPriceAtTick } from '../src/tickmath.ts';
import {
  buildClosePosition,
  buildCloseTickArrays,
  buildInitializePool,
  buildInitializeTickArray,
  buildModifyLiquidity,
  buildSwap,
  buildSwapFromBalance,
} from '../src/tx/builders.ts';
import { planCreatePool } from '../src/tx/createPool.ts';
import { readSwapQuoteAccounts } from '../src/adapters.ts';
import type { AccountSource } from '../src/source.ts';
import { stateView } from '../src/stateView.ts';
import { addr, SDK_ROOT } from './helpers.ts';
import { test } from './node-test.ts';

const utf8 = (s: string) => new TextEncoder().encode(s);

function randomAddress(): Address {
  const b = new Uint8Array(32);
  crypto.getRandomValues(b);
  return bytesToAddress(b);
}

/** Testnet's CAIP-2 id: a cluster with no shipped row, for synthetic rows. */
const TESTNET_CHAIN_KEY = 'solana:4uhcVJyU9pJkvQyS88uRDiswHXSCkY3z';

/** A row for a cluster this package does not ship, with fresh random program ids and its derived accounts. */
async function syntheticRow(): Promise<ClusterDeployment> {
  const carpenterAmm = randomAddress();
  const [ammConfig] = await getProgramDerivedAddress({ programAddress: carpenterAmm, seeds: [utf8('amm_config')] });
  const [protocolTreasury] = await getProgramDerivedAddress({ programAddress: carpenterAmm, seeds: [utf8('protocol_treasury')] });
  return {
    name: 'testnet',
    chainKey: TESTNET_CHAIN_KEY,
    programs: { carpenterAmm },
    accounts: { ammConfig, protocolTreasury },
    programDataHash: { carpenterAmm: null },
  };
}

/** Devnet's program id and every id-only PDA a builder could fall back to. */
async function devnetKeys(): Promise<Set<string>> {
  const d = DEFAULT_PROGRAM_IDS;
  return new Set<string>([
    d.carpenterAmm,
    (await findAmmConfig(d)).address,
    (await findProtocolTreasury(d)).address,
    (await findEventAuthority(d.carpenterAmm)).address,
    ...Object.values(DEPLOYMENTS.clusters).flatMap((r) => [r.programs.carpenterAmm, ...Object.values(r.accounts)]),
  ]);
}

/** Every address an instruction list names: program ids and account metas. */
function addressesOf(ixs: readonly Instruction[]): string[] {
  return ixs.flatMap((ix) => [ix.programAddress, ...(ix.accounts ?? []).map((m) => m.address)]);
}

function assertNoDevnet(what: string, addrs: readonly string[], devnet: ReadonlySet<string>): void {
  for (const a of addrs) assert.ok(!devnet.has(a), `${what}: devnet key ${a} leaked`);
}

/** No devnet program id inside a wire transaction's bytes. */
function assertWireClean(what: string, base64: string, devnet: ReadonlySet<string>): void {
  const bytes = getBase64Encoder().encode(base64);
  for (const k of devnet) {
    const needle = addressToBytes(k as Address);
    outer: for (let i = 0; i + 32 <= bytes.length; i++) {
      for (let j = 0; j < 32; j++) if (bytes[i + j] !== needle[j]) continue outer;
      assert.fail(`${what}: devnet key ${k} in the wire bytes`);
    }
  }
}

test('generate accepts a row with its own program ids and checks its derived accounts against THEM', async () => {
  const idl = { carpenterAmm: CARPENTER_AMM_IDL.address };
  const row = await syntheticRow();
  const file = { version: 1, clusters: { ...DEPLOYMENTS.clusters, [TESTNET_CHAIN_KEY]: row } };
  assert.deepEqual(await validateDeployments(file, idl), []);
  // The shipped file passes as it is.
  assert.deepEqual(await validateDeployments(DEPLOYMENTS, idl), []);
  // A derived account from another program id is refused by name.
  const wrong = { ...row, accounts: { ...row.accounts, ammConfig: DEPLOYMENTS.clusters['local']!.accounts.ammConfig } };
  const p1 = await validateDeployments({ version: 1, clusters: { [TESTNET_CHAIN_KEY]: wrong } }, idl);
  assert.equal(p1.length, 1);
  assert.match(p1[0]!, /ammConfig .* derivation/);
  // A program id that is not a public key is refused.
  const bad = { ...row, programs: { carpenterAmm: 'not-a-key' } };
  assert.ok((await validateDeployments({ version: 1, clusters: { [TESTNET_CHAIN_KEY]: bad } }, idl)).some((m) => /not a base58 32-byte/.test(m)));
  // The mainnet and local rows stay pinned to the IDL's address; devnet names its own programs.
  const drifted = { ...row, name: 'mainnet' as const, chainKey: MAINNET_CHAIN_KEY };
  assert.ok((await validateDeployments({ version: 1, clusters: { [MAINNET_CHAIN_KEY]: drifted } }, idl)).some((m) => /must match the shipped IDL/.test(m)));
  const devnet = deploymentFor(DEVNET_CHAIN_KEY);
  assert.notEqual(devnet.programs.carpenterAmm, idl.carpenterAmm);
  assert.deepEqual(await validateDeployments({ version: 1, clusters: { [DEVNET_CHAIN_KEY]: devnet } }, idl), []);
});

test('deploymentFor: the mainnet row is shipped; another key throws by name until a row exists, then returns it', async () => {
  assert.equal(deploymentFor(MAINNET_CHAIN_KEY).programs.carpenterAmm, CARPENTER_AMM_IDL.address);
  assert.throws(() => deploymentFor(TESTNET_CHAIN_KEY), new RegExp(TESTNET_CHAIN_KEY));
  const row = await syntheticRow();
  const file: DeploymentsFile = { version: 1, clusters: { ...DEPLOYMENTS.clusters, [TESTNET_CHAIN_KEY]: row } };
  assert.equal(deploymentFor(TESTNET_CHAIN_KEY, file), row);
  assert.equal(programIdsFromDeployment(row).carpenterAmm, row.programs.carpenterAmm);
});

test('every builder, planner and PDA given another row\'s ids names no devnet key', async () => {
  const row = await syntheticRow();
  const ids: ProgramIds = programIdsFromDeployment(row);
  assert.notEqual(ids.carpenterAmm, DEFAULT_PROGRAM_IDS.carpenterAmm);
  const devnet = await devnetKeys();
  const trader = addr(1);
  const pool = addr(2);
  const hook = addr(3);
  const [mint0, mint1] = [WSOL_MINT, addr(250)];
  const poolState = {
    mint0, mint1, vault0: addr(22), vault1: addr(23), tokenProgram0: SPL_TOKEN_PROGRAM_ID, tokenProgram1: TOKEN_2022_PROGRAM_ID,
    hook: NO_HOOK, decimals0: 9, decimals1: 6, tickSpacing: 60,
  };
  const hookSlice = [{ address: addr(40), role: 1 as const }];
  const built = {
    swap: await buildSwap({ trader, pool, poolState, zeroForOne: true, amountSpecified: -1_000n, amountLimit: 1n, tickArrayStarts: [0, -3600], ensureTickArrayStarts: [-3600], ids }),
    swapHooked: await buildSwap({
      trader, pool, poolState: { ...poolState, hook }, zeroForOne: false, amountSpecified: -1_000n, amountLimit: 1n, tickArrayStarts: [0], hookSlice, cuUnits: 400_000, ids,
    }),
    swapFromBalance: await buildSwapFromBalance({ trader, pool, poolState, zeroForOne: false, keep: 0n, amountLimit: 1n, limitInBasis: 1_000n, tickArrayStarts: [0], ids }),
    modify: await buildModifyLiquidity({ owner: trader, pool, poolState, salt: 0n, tickLower: -600, tickUpper: 600, liquidityDelta: 1_000n, amount0Max: 10n, amount1Max: 10n, ids }),
    remove: await buildModifyLiquidity({ owner: trader, pool, poolState, salt: 0n, tickLower: -600, tickUpper: 600, liquidityDelta: -1_000n, amount0Max: 0n, amount1Max: 0n, amount0Min: 1n, ids }),
    closePosition: await buildClosePosition({ owner: trader, pool, salt: 0n, ids }),
    initTickArray: await buildInitializeTickArray({ payer: trader, pool, startIndex: -3600, ids }),
    closeTickArrays: await buildCloseTickArrays({ rentOwner: trader, arrays: [{ pool, startIndex: 0 }, { pool, startIndex: 3600 }], ids }),
    initPool: await buildInitializePool({
      payer: trader, mintA: mint1, mintB: mint0, tokenProgramA: TOKEN_2022_PROGRAM_ID, tokenProgramB: SPL_TOKEN_PROGRAM_ID, fee: 3_000, tickSpacing: 60, hook: NO_HOOK,
      sqrtPriceX64: getSqrtPriceAtTick(0), ids,
    }),
    initPoolHooked: await buildInitializePool({
      payer: trader, mintA: mint1, mintB: mint0, tokenProgramA: TOKEN_2022_PROGRAM_ID, tokenProgramB: SPL_TOKEN_PROGRAM_ID, fee: 0x800000, tickSpacing: 60, hook,
      sqrtPriceX64: getSqrtPriceAtTick(0), hookSlice, ids,
    }),
  };
  for (const [name, b] of Object.entries(built)) {
    assert.equal(b.main.programAddress, ids.carpenterAmm, `${name}: the row's program`);
    assertNoDevnet(name, [...addressesOf(b.instructions), ...b.accounts], devnet);
  }
  // The event authority and the PDAs are the row's.
  assert.ok(built.swap.accounts.includes((await findEventAuthority(ids.carpenterAmm)).address));
  assert.ok(built.swap.accounts.includes((await findBitmap(pool, ids)).address));
  assert.ok(built.initPool.accounts.includes((await findAmmConfig(ids)).address));
  assert.ok(built.initPoolHooked.accounts.includes((await findApprovedHook(hook, ids)).address));

  const plan = await planCreatePool({
    payer: trader, mintA: mint1, mintB: mint0, tokenProgramA: TOKEN_2022_PROGRAM_ID, tokenProgramB: SPL_TOKEN_PROGRAM_ID, decimalsA: 6, decimalsB: 9,
    fee: 3_000, tickSpacing: 60, hook: NO_HOOK, sqrtPriceX64: getSqrtPriceAtTick(0),
    position: { tickLower: -600, tickUpper: 600, liquidity: 1_000_000n, amountAMax: 5_000n, amountBMax: 7_000n },
    lifetime: { blockhash: blockhash('EtWTRABZaYq6iMfeYKouRu166VU2xqa1Cr7xE9bbG1vv'), lastValidBlockHeight: 1_000n }, ids,
  });
  assert.equal(plan.pool, (await findPool({ mint0, mint1, fee: 3_000, tickSpacing: 60, hook: NO_HOOK }, ids)).address);
  for (const s of plan.steps) assertNoDevnet(`planCreatePool/${s.name}`, [...addressesOf(s.built.instructions), ...s.built.accounts], devnet);
  plan.transactions.forEach((t, i) => assertWireClean(`planCreatePool tx ${i}`, t, devnet));

  // The PDA helpers and the predictor derive under the ids they are given.
  const pdas = [
    await findAmmConfig(ids), await findProtocolTreasury(ids), await findBitmap(pool, ids), await findTickArray(pool, 0, ids),
    await findPosition(pool, trader, 0n, ids), await findPoolIndexEntry(pool, ids), await findVault(pool, mint0, ids),
    await findMintExclusivity(mint0, ids), await findMintPools(mint0, ids), await findQuoteMint(mint0, ids), await findApprovedHook(hook, ids),
  ].map((p) => p.address);
  assertNoDevnet('PDAs', [...pdas, ...(await swapTickArrays(pool, { arraysTouched: [0, 3600], actionArrays: [-3600] }, ids)).addresses], devnet);

  // The generated client derives its own PDAs under the `programAddress` it is given.
  const codama = await getInitializePoolInstructionAsync(
    {
      payer: { address: trader, signTransaction: async () => [] } as never,
      initializer: { address: trader, signTransaction: async () => [] } as never,
      mint0, mint1, tokenProgram0: SPL_TOKEN_PROGRAM_ID, tokenProgram1: TOKEN_2022_PROGRAM_ID,
      approvedHook: undefined, hookProgram: undefined, hookProgramdata: undefined,
      eventAuthority: (await findEventAuthority(ids.carpenterAmm)).address, program: ids.carpenterAmm,
      fee: 3_000, tickSpacing: 60, hook: NO_HOOK, sqrtPriceX64: getSqrtPriceAtTick(0),
    } as never,
    { programAddress: ids.carpenterAmm },
  );
  assert.equal(codama.programAddress, ids.carpenterAmm);
  assertNoDevnet('codama initializePool', addressesOf([codama as Instruction]), devnet);
});

test('the generated client names a shipped program id only as its own default program address', () => {
  const ids = new Set(Object.values(DEPLOYMENTS.clusters).flatMap((r) => Object.values(r.programs)));
  const root = join(SDK_ROOT, 'src/generated/client');
  const walk = (dir: string): string[] =>
    readdirSync(dir, { withFileTypes: true }).flatMap((e) => (e.isDirectory() ? walk(join(dir, e.name)) : [join(dir, e.name)]));
  for (const file of walk(root)) {
    const lines = readFileSync(file, 'utf8').split('\n');
    lines.forEach((line, i) => {
      for (const id of ids) {
        if (!line.includes(id)) continue;
        const allowed = /PROGRAM_ADDRESS = '/.test(line) || /programAddress = '/.test(line) || /TProgram extends string = '/.test(line);
        assert.ok(allowed, `${file}:${i + 1} names ${id} outside a default program address`);
      }
    });
  }
});

/** An empty source that records every address and program it is asked for. */
function recordingSource(): AccountSource & { asked: string[] } {
  const asked: string[] = [];
  return {
    asked,
    async getAccount(a) {
      asked.push(a);
      return { slot: 1n, account: null };
    },
    async getMultipleAccounts(as) {
      asked.push(...as);
      return { slot: 1n, accounts: as.map(() => null) };
    },
    async getProgramAccounts(program) {
      asked.push(program);
      return { slot: 1n, accounts: [] };
    },
  };
}

test('every read given another row\'s ids asks for that row\'s accounts only', async () => {
  const ids = programIdsFromDeployment(await syntheticRow());
  const devnet = await devnetKeys();
  const src = recordingSource();
  const pool = addr(2);
  const v = stateView;
  await v.getAmmConfig(src, ids);
  await v.getPool(src, pool, ids);
  await v.getSlot0(src, pool, ids);
  await v.getBitmap(src, pool, ids);
  await v.getTickArray(src, pool, 0, ids);
  await v.getPosition(src, pool, addr(1), 0n, ids);
  await v.positionsForOwner(src, addr(1), ids);
  await v.listPools(src, {}, ids);
  await v.getMintPoolCount(src, WSOL_MINT, ids);
  await v.getQuoteMint(src, WSOL_MINT, ids);
  await v.getMintExclusivity(src, WSOL_MINT, ids);
  await v.getApprovedHook(src, addr(3), ids);
  await v.listApprovedHooks(src, ids);
  await v.getProtocolTreasury(src, ids);
  await assert.rejects(readSwapQuoteAccounts(src, pool, { ids }), /no pool/);
  assert.ok(src.asked.includes(ids.carpenterAmm), 'the row\'s program is scanned');
  assertNoDevnet('reads', src.asked, devnet);
});
