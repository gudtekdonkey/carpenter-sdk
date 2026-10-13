/**
 * Where account bytes come from. Every read helper takes an `AccountSource`, never an RPC, so the helpers are pure over
 * bytes: tests feed fixtures, an app feeds `fromKitRpc(rpc)`, and an indexer is one more implementation. Each read
 * returns the SLOT it was read at.
 * `getProgramAccounts` is server-side only: public RPCs refuse or rate-limit it.
 */

import {
  getBase58Decoder,
  type Address,
  type Base58EncodedBytes,
  type GetAccountInfoApi,
  type GetGenesisHashApi,
  type GetMinimumBalanceForRentExemptionApi,
  type GetMultipleAccountsApi,
  type GetProgramAccountsApi,
  type Rpc,
} from '@solana/kit';

import { base64ToBytes } from './bytes.ts';

export interface RawAccount {
  readonly owner: Address;
  readonly lamports: bigint;
  readonly executable: boolean;
  readonly data: Uint8Array;
}

export interface MemcmpFilter {
  readonly memcmp: { readonly offset: number; readonly bytes: Uint8Array };
}
export interface DataSizeFilter {
  readonly dataSize: number;
}
export type AccountFilter = MemcmpFilter | DataSizeFilter;

export interface AccountSource {
  getAccount(address: Address): Promise<{ slot: bigint; account: RawAccount | null }>;
  getMultipleAccounts(addresses: readonly Address[]): Promise<{ slot: bigint; accounts: (RawAccount | null)[] }>;
  getProgramAccounts(program: Address, filters: readonly AccountFilter[]): Promise<{ slot: bigint; accounts: { address: Address; account: RawAccount }[] }>;
}

/** Rent is read at runtime, never hardcoded. */
export type RentExemptMinimum = (dataLen: number) => Promise<bigint> | bigint;

/** The RPC methods `fromKitRpc` / `rentFromRpc` / `chainKeyFromRpc` call. */
export type SolanaRpc = Rpc<GetAccountInfoApi & GetMultipleAccountsApi & GetProgramAccountsApi & GetMinimumBalanceForRentExemptionApi & GetGenesisHashApi>;

const MULTIPLE_ACCOUNTS_CHUNK = 100;

type Base64Account = { owner: Address; lamports: bigint; executable: boolean; data: readonly [string, 'base64'] | [string, 'base64'] };
function toRaw(a: Base64Account): RawAccount {
  return { owner: a.owner, lamports: a.lamports, executable: a.executable, data: base64ToBytes(a.data[0]) };
}

/** An `AccountSource` over a `@solana/kit` RPC. Not exercised by the unit tests (no network). */
export function fromKitRpc(rpc: SolanaRpc, commitment: 'processed' | 'confirmed' | 'finalized' = 'confirmed'): AccountSource {
  const b58 = getBase58Decoder();
  return {
    async getAccount(address) {
      const r = await rpc.getAccountInfo(address, { encoding: 'base64', commitment }).send();
      return { slot: r.context.slot, account: r.value ? toRaw(r.value) : null };
    },
    async getMultipleAccounts(addresses) {
      const accounts: (RawAccount | null)[] = [];
      let slot = 0n;
      for (let i = 0; i < addresses.length; i += MULTIPLE_ACCOUNTS_CHUNK) {
        const chunk = addresses.slice(i, i + MULTIPLE_ACCOUNTS_CHUNK);
        const r = await rpc.getMultipleAccounts(chunk, { encoding: 'base64', commitment }).send();
        // Chunks may land at different slots; the OLDEST is the honest tag for the whole read.
        slot = slot === 0n || r.context.slot < slot ? r.context.slot : slot;
        for (const v of r.value) accounts.push(v ? toRaw(v) : null);
      }
      return { slot, accounts };
    },
    async getProgramAccounts(program, filters) {
      const r = await rpc
        .getProgramAccounts(program, {
          encoding: 'base64',
          commitment,
          withContext: true,
          filters: filters.map((f) =>
            'memcmp' in f
              ? { memcmp: { offset: BigInt(f.memcmp.offset), bytes: b58.decode(f.memcmp.bytes) as Base58EncodedBytes, encoding: 'base58' as const } }
              : { dataSize: BigInt(f.dataSize) },
          ),
        })
        .send();
      return { slot: r.context.slot, accounts: r.value.map((x) => ({ address: x.pubkey, account: toRaw(x.account) })) };
    },
  };
}

/** Rent from the RPC (`getMinimumBalanceForRentExemption`), memoised per length. */
export function rentFromRpc(rpc: SolanaRpc): RentExemptMinimum {
  const cache = new Map<number, Promise<bigint>>();
  return (len) => {
    let p = cache.get(len);
    if (!p) {
      p = rpc
        .getMinimumBalanceForRentExemption(BigInt(len))
        .send()
        .then((v) => BigInt(v));
      cache.set(len, p);
    }
    return p;
  };
}

/** `solana:<genesis32>` for the cluster behind an RPC (CAIP-2). */
export async function chainKeyFromRpc(rpc: SolanaRpc): Promise<string> {
  const genesis = await rpc.getGenesisHash().send();
  return `solana:${String(genesis).slice(0, 32)}`;
}

/** An in-memory source for tests, fixtures and snapshots. */
export function memorySource(accounts: ReadonlyMap<Address, RawAccount> | Record<string, RawAccount>, slot = 1n): AccountSource {
  const map: ReadonlyMap<string, RawAccount> = accounts instanceof Map ? accounts : new Map(Object.entries(accounts));
  const matches = (a: RawAccount, filters: readonly AccountFilter[]) =>
    filters.every((f) => {
      if ('dataSize' in f) return a.data.length === f.dataSize;
      const { offset, bytes } = f.memcmp;
      if (offset + bytes.length > a.data.length) return false;
      for (let i = 0; i < bytes.length; i++) if (a.data[offset + i] !== bytes[i]) return false;
      return true;
    });
  return {
    async getAccount(address) {
      return { slot, account: map.get(address) ?? null };
    },
    async getMultipleAccounts(addresses) {
      return { slot, accounts: addresses.map((a) => map.get(a) ?? null) };
    },
    async getProgramAccounts(program, filters) {
      const accounts: { address: Address; account: RawAccount }[] = [];
      for (const [address, account] of map) {
        if (account.owner === program && matches(account, filters)) accounts.push({ address: address as Address, account });
      }
      return { slot, accounts };
    },
  };
}
