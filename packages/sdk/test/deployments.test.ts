
import assert from 'node:assert/strict';

import { chainKeyFromGenesisHash, DEPLOYMENTS, deploymentFor, DEVNET_CHAIN_KEY, MAINNET_CHAIN_KEY, PROGRAM_NAMES } from '../src/deployments.ts';
import { findAmmConfig, type ProgramIds } from '../src/pda.ts';
import { test } from './node-test.ts';

const B58 = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;

test('every cluster row is complete and keyed by a CAIP-2 id or `local`', () => {
  assert.equal(DEPLOYMENTS.version, 1);
  const keys = Object.keys(DEPLOYMENTS.clusters);
  assert.ok(keys.includes(DEVNET_CHAIN_KEY));
  assert.ok(keys.includes('local'));
  for (const [key, row] of Object.entries(DEPLOYMENTS.clusters)) {
    assert.ok(['mainnet', 'devnet', 'local'].includes(row.name), key);
    if (key === 'local') assert.equal(row.chainKey, null);
    else {
      assert.equal(row.chainKey, key);
      assert.match(key, /^solana:[1-9A-HJ-NP-Za-km-z]{32}$/);
    }
    assert.match(row.programs.carpenterAmm, B58, `${key}.programs.carpenterAmm`);
    assert.match(row.accounts.ammConfig, B58, `${key}.accounts.ammConfig`);
    // Only Carpenter's programs: the hook programs' ids live in their own packages.
    assert.deepEqual(Object.keys(row.programs), ['carpenterAmm'], key);
  }
  assert.deepEqual([...PROGRAM_NAMES], ['carpenter_amm']);
});

test('deploymentFor throws by name; chainKeyFromGenesisHash takes the first 32 characters', () => {
  assert.throws(() => deploymentFor('solana:nope'), /solana:nope/);
  assert.equal(chainKeyFromGenesisHash('EtWTRABZaYq6iMfeYKouRu166VU2xqa1Cr7xE9bbG1vv'), DEVNET_CHAIN_KEY);
  assert.equal(chainKeyFromGenesisHash('5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d'), MAINNET_CHAIN_KEY);
});

test('each row\'s ammConfig is ["amm_config"] under its own program id', async () => {
  for (const [key, row] of Object.entries(DEPLOYMENTS.clusters)) {
    const ids = { carpenterAmm: row.programs.carpenterAmm } as ProgramIds;
    assert.equal((await findAmmConfig(ids)).address, row.accounts.ammConfig, key);
  }
});
