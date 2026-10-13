// The derivations and codecs outside the program's own IDL: the position PDA, the tick arrays a swap needs, SPL Token /
// Token-2022 mint and account bytes, and the Address Lookup Table program's instructions and table account.

import assert from 'node:assert/strict';

import { getProgramDerivedAddress, type Address } from '@solana/kit';

import { addressToBytes, u64le } from '../src/bytes.ts';
import { SEEDS } from '../src/constants.ts';
import { ADDRESS_LOOKUP_TABLE_PROGRAM_ID, DEFAULT_PROGRAM_IDS, findAddressLookupTable, findPosition, findTickArray } from '../src/pda.ts';
import { swapTickArrays } from '../src/predict.ts';
import { decodeMint, decodeTokenAccount, EXTENSION_TYPES } from '../src/token2022.ts';
import {
  buildCloseLookupTable,
  buildDeactivateLookupTable,
  closeLookupTableIx,
  createLookupTableIx,
  decodeLookupTable,
  deactivateLookupTableIx,
  extendLookupTableIx,
  freezeLookupTableIx,
  LOOKUP_TABLE_ACTIVE_SLOT,
  LOOKUP_TABLE_CLOSE_COOLDOWN_SLOTS,
  LOOKUP_TABLE_CU,
  LOOKUP_TABLE_META_SIZE,
  LOOKUP_TABLE_RECENT_SLOT_SLICE,
  lookupTableStatus,
  recentSlotFromSlotHashes,
} from '../src/tx/lookupTable.ts';
import { addr } from './helpers.ts';
import { test } from './node-test.ts';

test('findPosition is ["position", pool, position_id, salt u64 LE] under carpenter_amm', async () => {
  for (const salt of [0n, 1n, 258n, (1n << 64n) - 1n]) {
    const [want, bump] = await getProgramDerivedAddress({
      programAddress: DEFAULT_PROGRAM_IDS.carpenterAmm,
      seeds: [SEEDS.position, addressToBytes(addr(1)), addressToBytes(addr(2)), u64le(salt)],
    });
    const got = await findPosition(addr(1), addr(2), salt);
    assert.deepEqual([got.address, got.bump], [want, bump], `salt ${salt}`);
  }
  assert.notEqual((await findPosition(addr(1), addr(2), 0n)).address, (await findPosition(addr(1), addr(3), 0n)).address, 'the id is a seed');
});

test('swapTickArrays: the walk ∪ the actions, deduplicated in first-seen order, as tick-array PDAs', async () => {
  const pool = addr(7);
  const r = await swapTickArrays(pool, { arraysTouched: [0, -3600, 0], actionArrays: [3600, -3600] });
  assert.deepEqual(r.starts, [0, -3600, 3600]);
  assert.deepEqual(r.addresses, await Promise.all(r.starts.map(async (s) => (await findTickArray(pool, s)).address)));
  assert.deepEqual(await swapTickArrays(pool, { arraysTouched: [], actionArrays: [] }), { starts: [], addresses: [] });
});

/** An SPL `COption<Pubkey>`: tag u32 then the key. */
const coption = (a: Address | null) => (a ? [1, 0, 0, 0, ...addressToBytes(a)] : new Array(36).fill(0));
const tlv = (type: number, value: number[]) => [type & 0xff, type >> 8, value.length & 0xff, value.length >> 8, ...value];

test('decodeMint: the 82-byte base, then a Token-2022 mint`s TLV extensions after account_type 1 at byte 165', () => {
  const base = [...coption(addr(1)), ...u64le(1_000_000n), 6, 1, ...coption(null)];
  assert.equal(base.length, 82);
  const legacy = decodeMint(new Uint8Array(base));
  assert.deepEqual([legacy.mintAuthority, legacy.supply, legacy.decimals, legacy.isInitialized, legacy.freezeAuthority, legacy.extensionTypes], [addr(1), 1_000_000n, 6, true, null, []]);
  assert.equal(legacy.transferHookConfig, null);
  const ext = [
    ...base,
    ...new Array(165 - 82).fill(0),
    1,
    ...tlv(EXTENSION_TYPES.TransferHook, [...addressToBytes(addr(5)), ...new Array(32).fill(0)]),
    ...tlv(EXTENSION_TYPES.Pausable, [...addressToBytes(addr(6)), 1]),
    ...tlv(EXTENSION_TYPES.TransferFeeConfig, new Array(108).fill(0)),
  ];
  const m = decodeMint(new Uint8Array(ext));
  assert.deepEqual(m.extensionTypes, [EXTENSION_TYPES.TransferHook, EXTENSION_TYPES.Pausable, EXTENSION_TYPES.TransferFeeConfig]);
  assert.equal(m.transferHook, true);
  assert.deepEqual(m.transferHookConfig, { authority: addr(5), programId: null }, 'an empty hook slot reads null');
  assert.deepEqual(m.pausable, { authority: addr(6), paused: true });
  assert.equal(m.transferFee, true);
  assert.throws(() => decodeMint(new Uint8Array(81)), /81 bytes/);
  const wrongType = [...ext];
  wrongType[165] = 2;
  assert.throws(() => decodeMint(new Uint8Array(wrongType)), /account_type 2/);
});

test('decodeTokenAccount: the 165-byte base, then CpiGuard / MemoTransfer / ImmutableOwner after account_type 2', () => {
  const base = [
    ...addressToBytes(addr(10)),
    ...addressToBytes(addr(11)),
    ...u64le(42n),
    ...coption(addr(12)),
    1,
    1, 0, 0, 0, ...u64le(2_039_280n),
    ...u64le(7n),
    ...coption(null),
  ];
  assert.equal(base.length, 165);
  const t = decodeTokenAccount(new Uint8Array(base));
  assert.deepEqual(
    [t.mint, t.owner, t.amount, t.delegate, t.state, t.isNative, t.delegatedAmount, t.closeAuthority, t.cpiGuardLocked, t.memoRequired, t.immutableOwner],
    [addr(10), addr(11), 42n, addr(12), 1, 2_039_280n, 7n, null, false, false, false],
  );
  const ext = new Uint8Array([...base, 2, ...tlv(EXTENSION_TYPES.ImmutableOwner, []), ...tlv(EXTENSION_TYPES.CpiGuard, [1]), ...tlv(EXTENSION_TYPES.MemoTransfer, [1])]);
  const e = decodeTokenAccount(ext);
  assert.deepEqual([e.immutableOwner, e.cpiGuardLocked, e.memoRequired], [true, true, true]);
  assert.deepEqual(e.extensionTypes, [EXTENSION_TYPES.ImmutableOwner, EXTENSION_TYPES.CpiGuard, EXTENSION_TYPES.MemoTransfer]);
  assert.throws(() => decodeTokenAccount(new Uint8Array(164)), /164 bytes/);
});

test('the lookup-table codec: instruction data and accounts, the table account round trip, its status and the recent slot', async () => {
  const [table, authority, payer] = [addr(1), addr(2), addr(3)];
  const create = createLookupTableIx({ table, authority, payer, recentSlot: 0x0102n, bump: 254 });
  assert.equal(create.programAddress, ADDRESS_LOOKUP_TABLE_PROGRAM_ID);
  assert.deepEqual([...create.data], [0, 0, 0, 0, 2, 1, 0, 0, 0, 0, 0, 0, 254], 'tag 0, recent_slot u64, bump u8');
  assert.deepEqual(create.accounts.map((a) => a.role), [1, 2, 3, 0], 'table w, authority s, payer s w, system');
  const ext = extendLookupTableIx({ table, authority, payer, addresses: [addr(9), addr(10)] });
  assert.deepEqual([...ext.data.subarray(0, 12)], [2, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0], 'tag 2, a u64-length vec');
  assert.deepEqual([...ext.data.subarray(12, 44)], [...addressToBytes(addr(9))]);
  assert.equal(ext.data.length, 12 + 64);
  assert.throws(() => extendLookupTableIx({ table, authority, payer, addresses: [] }), /no addresses/);
  assert.deepEqual([...freezeLookupTableIx({ table, authority }).data], [1, 0, 0, 0]);
  assert.deepEqual([...deactivateLookupTableIx({ table, authority }).data], [3, 0, 0, 0]);
  const close = closeLookupTableIx({ table, authority, recipient: addr(4) });
  assert.deepEqual([...close.data], [4, 0, 0, 0]);
  assert.deepEqual(close.accounts.map((a) => [a.address, a.role]), [[table, 1], [authority, 2], [addr(4), 1]]);

  // a table account: type 1, deactivation u64, last extended u64 + start u8, Option<Pubkey>, u16 pad, then the keys
  const raw = new Uint8Array(LOOKUP_TABLE_META_SIZE + 64);
  const dv = new DataView(raw.buffer);
  dv.setUint32(0, 1, true);
  dv.setBigUint64(4, LOOKUP_TABLE_ACTIVE_SLOT, true);
  dv.setBigUint64(12, 500n, true);
  raw[20] = 1;
  raw[21] = 1;
  raw.set(addressToBytes(authority), 22);
  raw.set(addressToBytes(addr(9)), 56);
  raw.set(addressToBytes(addr(10)), 88);
  const t = decodeLookupTable(raw);
  assert.deepEqual(t, { deactivationSlot: LOOKUP_TABLE_ACTIVE_SLOT, lastExtendedSlot: 500n, lastExtendedSlotStartIndex: 1, authority, addresses: [addr(9), addr(10)] });
  raw[21] = 0;
  assert.equal(decodeLookupTable(raw).authority, null, 'frozen');
  assert.throws(() => decodeLookupTable(raw.subarray(0, 60)), /not a lookup table/);
  dv.setUint32(0, 2, true);
  assert.throws(() => decodeLookupTable(raw), /account type 2/);

  assert.equal(lookupTableStatus(t, 500n), 'warming');
  assert.equal(lookupTableStatus(t, 501n), 'active');
  const off = { deactivationSlot: 1_000n, lastExtendedSlot: 500n };
  assert.equal(lookupTableStatus(off, 1_000n + LOOKUP_TABLE_CLOSE_COOLDOWN_SLOTS - 1n), 'deactivating');
  assert.equal(lookupTableStatus(off, 1_000n + LOOKUP_TABLE_CLOSE_COOLDOWN_SLOTS), 'closeable');

  const sh = new Uint8Array(LOOKUP_TABLE_RECENT_SLOT_SLICE.offset + 8);
  new DataView(sh.buffer).setBigUint64(LOOKUP_TABLE_RECENT_SLOT_SLICE.offset, 777n, true);
  assert.equal(recentSlotFromSlotHashes(sh), 777n, 'the whole SlotHashes account: entry 8');
  assert.equal(recentSlotFromSlotHashes(sh.subarray(LOOKUP_TABLE_RECENT_SLOT_SLICE.offset)), 777n, 'the 8-byte slice');
  assert.throws(() => recentSlotFromSlotHashes(new Uint8Array(20)), /holds no entry 8/);

  const [want] = await getProgramDerivedAddress({ programAddress: ADDRESS_LOOKUP_TABLE_PROGRAM_ID, seeds: [addressToBytes(authority), u64le(0x0102n)] });
  assert.equal((await findAddressLookupTable(authority, 0x0102n)).address, want);

  const d = buildDeactivateLookupTable({ authority, table });
  assert.equal(d.cuLimit, LOOKUP_TABLE_CU.deactivate);
  assert.deepEqual([...d.main.data], [3, 0, 0, 0]);
  const c = buildCloseLookupTable({ authority, table });
  assert.equal(c.cuLimit, LOOKUP_TABLE_CU.close);
  assert.equal(c.main.accounts[2]!.address, authority, 'the rent goes to the authority by default');
});
