
import assert from 'node:assert/strict';

import { CARPENTER_AMM_PROGRAM_ID, carpenterAmm, decodeTickArray, deploymentFor, DEVNET_CHAIN_KEY, encodeTickArray, IDLS, MAINNET_CHAIN_KEY, TICK_ARRAY_LEN_V1, TICK_ARRAY_LEN_V2 } from '../src/index.ts';
import { lamports } from '@solana/kit';

import { addr } from './helpers.ts';
import { test } from './node-test.ts';

test('the generated client names the IDL\'s program, which is the mainnet row\'s and the default', () => {
  assert.equal(carpenterAmm.CARPENTER_AMM_PROGRAM_ADDRESS, IDLS.carpenter_amm.address);
  assert.equal(carpenterAmm.CARPENTER_AMM_PROGRAM_ADDRESS, deploymentFor(MAINNET_CHAIN_KEY).programs.carpenterAmm);
  assert.equal(CARPENTER_AMM_PROGRAM_ID, deploymentFor(MAINNET_CHAIN_KEY).programs.carpenterAmm);
  assert.notEqual(deploymentFor(DEVNET_CHAIN_KEY).programs.carpenterAmm, CARPENTER_AMM_PROGRAM_ID, 'devnet runs its own program');
});

test('the generated TickArray codec decodes a 3,920-byte account exactly as its 3,888-byte body', () => {
  assert.equal(carpenterAmm.getTickArraySize(), TICK_ARRAY_LEN_V1, 'the IDL struct is the 3,888-byte body');
  const fields = { pool: addr(9), startIndex: -3600, initializedTickCount: 1, ticks: [{ liquidityGross: 5n, liquidityNet: -5n, feeGrowthOutside1X64: 3n }] };
  const v1 = encodeTickArray(fields);
  const v2 = encodeTickArray(fields, addr(77));
  assert.equal(v2.length, TICK_ARRAY_LEN_V2);
  const d1 = carpenterAmm.getTickArrayDecoder().decode(v1);
  const d2 = carpenterAmm.getTickArrayDecoder().decode(v2);
  assert.deepEqual(d2, d1, 'the rent-owner tail is never read');
  assert.equal(d2.pool, addr(9));
  assert.equal(d2.startIndex, -3600);
  assert.equal(d2.initializedTickCount, 1);
  const acct = { exists: true as const, address: addr(10), data: v2, executable: false, lamports: lamports(20_563_840n), programAddress: addr(11), space: BigInt(v2.length) };
  const decoded = carpenterAmm.decodeTickArray(acct);
  assert.deepEqual(decoded.data, d1);
  // the SDK's own decoder also reads the rent owner
  const sdk = decodeTickArray(v2);
  assert.equal(sdk.rentOwner, addr(77));
  assert.equal(decoded.data.startIndex, sdk.startIndex);
  assert.equal(decoded.data.pool, sdk.pool);
});
