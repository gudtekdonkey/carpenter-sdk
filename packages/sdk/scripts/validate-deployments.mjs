// The deployments.json rules `npm run generate` enforces, as a function the tests can call on synthetic rows.
//
// - The `mainnet` and `local` rows name the program ids this package's IDLs were built for: each must equal the IDL's
//   `address`. A client generated against the wrong program is the one thing generation must stop.
// - Every other row (devnet, ...) may name its own program ids: each is checked to be a base58 32-byte public key.
// - Every derivable account of a row is checked against its derivation from THAT row's program ids.
// - `chainKey` is `solana:<genesis32>` (or `null` on `local`).

import { getAddressEncoder, getProgramDerivedAddress, isAddress } from '@solana/kit';

/** Rows whose program ids must equal the IDLs' `address`. */
export const IDL_PINNED_ROWS = ['mainnet', 'local'];

const utf8 = (s) => new TextEncoder().encode(s);

/** `[account key, program key, seeds]`: the accounts a row may carry that are PDAs of its own programs. */
export const DERIVED_ACCOUNTS = [
  ['ammConfig', 'carpenterAmm', [utf8('amm_config')]],
  ['protocolTreasury', 'carpenterAmm', [utf8('protocol_treasury')]],
];
/** Accounts every row must carry. */
export const REQUIRED_ACCOUNTS = ['ammConfig'];

function isPubkey(s) {
  if (typeof s !== 'string' || !isAddress(s)) return false;
  return getAddressEncoder().encode(s).length === 32;
}

/**
 * Validate a parsed deployments.json. `idlAddress` maps a `programs` key to its IDL's `address`. Returns the list of
 * problems (empty when valid); never throws on bad data.
 */
export async function validateDeployments(deployments, idlAddress) {
  const problems = [];
  if (deployments?.version !== 1) problems.push('version must be 1');
  for (const [cluster, row] of Object.entries(deployments?.clusters ?? {})) {
    const where = `deployments.json ${cluster}`;
    const pinned = IDL_PINNED_ROWS.includes(row.name);
    for (const [key, address] of Object.entries(idlAddress)) {
      const id = row.programs?.[key];
      if (!isPubkey(id)) {
        problems.push(`${where}.programs.${key} is not a base58 32-byte public key: ${id}`);
      } else if (pinned && id !== address) {
        problems.push(`${where}.programs.${key} = ${id} but the IDL address is ${address} (a ${row.name} row must match the shipped IDL)`);
      }
    }
    for (const k of REQUIRED_ACCOUNTS) if (typeof row.accounts?.[k] !== 'string') problems.push(`${where}.accounts.${k} missing`);
    for (const [k, program, seeds] of DERIVED_ACCOUNTS) {
      const have = row.accounts?.[k];
      if (have === undefined) continue;
      const programId = row.programs?.[program];
      if (!isPubkey(have)) {
        problems.push(`${where}.accounts.${k} is not a base58 32-byte public key: ${have}`);
        continue;
      }
      if (!isPubkey(programId)) continue;
      const [want] = await getProgramDerivedAddress({ programAddress: programId, seeds });
      if (want !== have) problems.push(`${where}.accounts.${k} = ${have} but its derivation under ${program} ${programId} is ${want}`);
    }
    if (cluster === 'local' ? row.chainKey !== null : row.chainKey !== cluster) {
      problems.push(`${where}.chainKey must be ${cluster === 'local' ? 'null' : 'the row key'}: ${row.chainKey}`);
    }
    if (row.chainKey !== null && !/^solana:[1-9A-HJ-NP-Za-km-z]{32}$/.test(row.chainKey)) {
      problems.push(`${where}.chainKey is not solana:<genesis32>: ${row.chainKey}`);
    }
    for (const [mint, q] of Object.entries(row.quotes ?? {})) {
      if (!isPubkey(mint)) problems.push(`${where}.quotes key ${mint} is not a public key`);
      if (!isPubkey(q?.amm?.entry)) problems.push(`${where}.quotes.${mint}.amm.entry is not a public key`);
      else if (isPubkey(mint) && isPubkey(row.programs?.carpenterAmm)) {
        const [want] = await getProgramDerivedAddress({ programAddress: row.programs.carpenterAmm, seeds: [utf8('quote_mint'), getAddressEncoder().encode(mint)] });
        if (want !== q.amm.entry) problems.push(`${where}.quotes.${mint}.amm.entry = ${q.amm.entry} but ["quote_mint", mint] under carpenterAmm is ${want}`);
      }
    }
  }
  return problems;
}
