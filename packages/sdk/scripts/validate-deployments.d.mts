/** Rows whose program ids must equal the IDLs' `address`. */
export declare const IDL_PINNED_ROWS: readonly string[];
/** `[account key, program key, seeds]`: the accounts a row may carry that are PDAs of its own programs. */
export declare const DERIVED_ACCOUNTS: readonly (readonly [string, string, readonly Uint8Array[]])[];
/** Accounts every row must carry. */
export declare const REQUIRED_ACCOUNTS: readonly string[];
/** The problems with a parsed deployments.json (empty when valid). `idlAddress` maps a `programs` key to its IDL's `address`. */
export declare function validateDeployments(deployments: unknown, idlAddress: Readonly<Record<string, string>>): Promise<string[]>;
