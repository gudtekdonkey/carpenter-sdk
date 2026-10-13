/**
 * Carpenter's deployments: `carpenter_amm`'s program id and singleton accounts per cluster, keyed by CAIP-2 id, and the
 * types of the IDL this package ships. A row names ONLY `carpenter_amm`; a hook's programs live in its own package.
 *
 * Program ids come from `deployments.json`, never a constant in an app; `DEFAULT_PROGRAM_IDS` (devnet's) exists for
 *    tests and scripts, `programIdsFromDeployment(row)` is the app's path. A row may name program ids other than the
 *    shipped IDL's `address` (another cluster's deployment); every builder, PDA and read takes the ids it is given.
 *    The devnet and local rows are the ones generation pins to the IDL's `address`.
 * `deploymentFor` throws BY NAME on an unknown cluster: a missing row must never read as a cluster with no pools.
 * `programDataHash` is the EXECUTABLE hash (trailing zero bytes stripped, then sha256 — `solana-verify
 *    get-program-hash`), never the `.so` file hash; `null` until recorded.
 */

import { DEPLOYMENTS } from './generated/deployments.ts';

/** The one program this package ships an IDL and a client for, by its Anchor crate name (the IDL file stem). */
export type ProgramName = 'carpenter_amm';

export const PROGRAM_NAMES: readonly ProgramName[] = ['carpenter_amm'];

/** The subset of an Anchor 1.x IDL the clients read. Everything else is kept. */
export interface AnchorIdl {
  readonly address: string;
  readonly metadata: { readonly name: string; readonly version: string; readonly spec: string; readonly [k: string]: unknown };
  readonly instructions: readonly AnchorIdlInstruction[];
  readonly accounts?: readonly { readonly name: string; readonly discriminator: readonly number[] }[];
  readonly events?: readonly { readonly name: string; readonly discriminator: readonly number[] }[];
  readonly errors?: readonly { readonly code: number; readonly name: string; readonly msg?: string }[];
  readonly types?: readonly unknown[];
  readonly [k: string]: unknown;
}

export interface AnchorIdlInstruction {
  readonly name: string;
  readonly discriminator: readonly number[];
  readonly accounts: readonly AnchorIdlAccountItem[];
  readonly args: readonly { readonly name: string; readonly type: unknown }[];
  readonly [k: string]: unknown;
}

/** An account slot. Anchor 1.x flattens `#[event_cpi]` into `event_authority` + `program`. */
export interface AnchorIdlAccountItem {
  readonly name: string;
  readonly writable?: boolean;
  readonly signer?: boolean;
  readonly optional?: boolean;
  /** Present when the item is a nested `Accounts` struct. */
  readonly accounts?: readonly AnchorIdlAccountItem[];
  readonly [k: string]: unknown;
}

/** One cluster's `carpenter_amm` row, keyed by CAIP-2 id (or `local`). */
export interface ClusterDeployment {
  readonly name: 'mainnet' | 'devnet' | 'testnet' | 'local';
  /** `solana:<first 32 chars of the genesis hash>`; `null` where the genesis is per-run (Surfpool). */
  readonly chainKey: string | null;
  readonly programs: { readonly carpenterAmm: string };
  readonly accounts: {
    /** `["amm_config"]` under `carpenter_amm` — derivable from the program id. */
    readonly ammConfig: string;
    /** `["protocol_treasury"]` under `carpenter_amm` (the Carpenter Platform fee's treasury), where a row records it. */
    readonly protocolTreasury?: string;
  };
  /** The deployed ProgramData's EXECUTABLE hash; `null` until recorded. */
  readonly programDataHash: { readonly carpenterAmm: string | null };
  /**
   * The quote mints LISTED on the AMM's roster (`["quote_mint", mint]`) on this cluster, keyed by mint. A record, never
   * an authority: the roster on chain is.
   */
  readonly quotes?: Readonly<Record<string, AmmQuoteListingRecord>>;
}

/** One quote on the AMM's roster (amounts and slots as decimal strings). */
export interface AmmQuoteListingRecord {
  readonly decimals: number;
  readonly tokenProgram: string;
  /** `QuoteMint.caps` as the AMM recorded it, and the same byte as words. */
  readonly caps: number;
  readonly capNames: readonly string[];
  /** `entry` is the roster account (`["quote_mint", mint]` under `carpenterAmm`); `listedSlot` the slot it was listed at. */
  readonly amm: { readonly entry: string; readonly listedSlot: string | null };
}

export interface DeploymentsFile {
  readonly version: 1;
  readonly clusters: Readonly<Record<string, ClusterDeployment>>;
}

export { DEPLOYMENTS };

/** The devnet CAIP-2 id (`solana:` + the first 32 chars of devnet's genesis hash). */
export const DEVNET_CHAIN_KEY = 'solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1';

/**
 * The mainnet-beta CAIP-2 id (genesis `5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d`). The mainnet row in
 * `deployments.json` is keyed by it, and names the program ids the shipped IDL carries.
 */
export const MAINNET_CHAIN_KEY = 'solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp';

/** `solana:<genesis32>` from a full genesis hash (CAIP-2 uses its first 32 characters). */
export function chainKeyFromGenesisHash(genesisHash: string): string {
  if (genesisHash.length < 32) throw new Error(`genesis hash too short: ${genesisHash}`);
  return `solana:${genesisHash.slice(0, 32)}`;
}

/**
 * The deployment for a cluster key, over the shipped `DEPLOYMENTS` by default. Throws BY NAME on an unknown key: a
 * missing row must never read as a cluster with no pools.
 */
export function deploymentFor(clusterKey: string, file: DeploymentsFile = DEPLOYMENTS): ClusterDeployment {
  const row = file.clusters[clusterKey];
  if (!row) {
    throw new Error(`@floorsupply/carpenter-sdk: no deployment for cluster "${clusterKey}" (known: ${Object.keys(file.clusters).join(', ')})`);
  }
  return row;
}
