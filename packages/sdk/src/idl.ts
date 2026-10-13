/**
 * The `carpenter_amm` IDL this package ships, and its sha256.
 * `IDL_SHA256` hashes the IDL FILE BYTES as shipped (`idl/carpenter_amm.json`: `anchor idl build`'s output with every
 *    `docs` field removed); an app's IDL check compares it with the installed client.
 * `./generated/` is written by `npm run generate` (`scripts/sync-idls.mjs`) from `idl/` and `deployments.json`.
 */

import { CARPENTER_AMM_IDL, IDL_SHA256 } from './generated/idls.ts';
import type { AnchorIdl, ProgramName } from './deployments.ts';

export { CARPENTER_AMM_IDL, IDL_SHA256 };
/** `IDL_SHA256` under a camelCase name: `{ carpenter_amm }` → hex. */
export const idlSha256: Readonly<Record<ProgramName, string>> = IDL_SHA256;

export const IDLS: Readonly<Record<ProgramName, AnchorIdl>> = {
  carpenter_amm: CARPENTER_AMM_IDL,
};
