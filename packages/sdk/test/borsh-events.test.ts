// Borsh writer/reader round trips (strict: trailing bytes refused) and the event decoders —
// every discriminator equals sha256("event:<Name>")[..8] computed with node:crypto, an encoded
// `Swapped` / `LaunchCreated` decodes field for field, and every schema in `EVENT_SCHEMAS` equals
// the built IDL's field list for that event (name, order and type).

import assert from 'node:assert/strict';
import { existsSync, readFileSync } from 'node:fs';
import { test } from './node-test.ts';

import { BorshReader, BorshWriter,     } from '../src/index.ts';
import {

  decodeEventPayload,

  

  EVENT_NAMES,
  EVENT_SCHEMAS,
  eventDiscriminator,
  type EventFieldKind,
  type ProgramKey,
} from '../src/events.ts';
import { addr, idlPath as idlPathOf } from './helpers.ts';

test('BorshWriter/BorshReader round-trip every primitive, and finish() refuses trailing bytes', () => {
  const w = new BorshWriter()
    .u8(255)
    .u16(65_535)
    .u32(0xdeadbeef)
    .i32(-2)
    .u64((1n << 64n) - 1n)
    .i64(-(1n << 63n))
    .u128((1n << 128n) - 1n)
    .i128(-(1n << 127n))
    .bool(true)
    .pubkey(addr(7))
    .vecU8(new Uint8Array([1, 2, 3]))
    .string('héllo')
    .optionU32(null)
    .optionU32(9);
  const r = new BorshReader(w.toBytes());
  assert.equal(r.u8(), 255);
  assert.equal(r.u16(), 65_535);
  assert.equal(r.u32(), 0xdeadbeef);
  assert.equal(r.i32(), -2);
  assert.equal(r.u64(), (1n << 64n) - 1n);
  assert.equal(r.i64(), -(1n << 63n));
  assert.equal(r.u128(), (1n << 128n) - 1n);
  assert.equal(r.i128(), -(1n << 127n));
  assert.equal(r.bool(), true);
  assert.equal(r.pubkey(), addr(7));
  assert.deepEqual([...r.vecU8()], [1, 2, 3]);
  assert.equal(r.string(), 'héllo');
  assert.equal(r.option(() => r.u32()), null);
  assert.equal(r.option(() => r.u32()), 9);
  r.finish();
  const extra = new BorshReader(new Uint8Array([1, 0]));
  extra.u8();
  assert.throws(() => extra.finish(), /trailing/);
  assert.throws(() => new BorshReader(new Uint8Array([2])).bool(), /bool/);
  assert.throws(() => new BorshReader(new Uint8Array([1])).u16(), /past end/);
  assert.throws(() => new BorshWriter().u64(-1n), RangeError);
  assert.throws(() => new BorshWriter().i32(2 ** 31), RangeError);
  assert.throws(() => new BorshWriter().fixed(new Uint8Array(31), 32), RangeError);
});

function swappedPayload(): Uint8Array {
  const body = new BorshWriter()
    .pubkey(addr(1))
    .pubkey(addr(2))
    .bool(true)
    .u64(1_000n)
    .u64(999n)
    .i128(30n)
    .i128(0n)
    .u64(3n)
    .optionU32(2500)
    .u128(1n << 64n)
    .i32(0)
    .u128((1n << 64n) - 5n)
    .i32(-1)
    .u8(3)
    .bool(true)
    .u64(5n)
    .u64(6n)
    //  `protocol_fee_total`, `protocol_fee_currency`,
    // INSERTED before the splits (which stay the last 80 bytes).
    .u64(9n)
    .u8(1)
    .u64(70n)
    .u64(71n)
    .u64(72n)
    .u64(73n)
    .u64(74n)
    .u64(80n)
    .u64(81n)
    .u64(82n)
    .u64(83n)
    .u64(84n)
    // `walk_sqrt_price_x64`, `walk_tick` — APPENDED after the splits (the last 20 B).
    .u128((1n << 64n) - 9n)
    .i32(-2)
    .toBytes();
  return new Uint8Array([...eventDiscriminator('Swapped'), ...body]);
}

// The SDK decodes FIVE-slot splits only. A four-slot body (16 B shorter) THROWS - never a silently
// mis-sized event. The body an older program version emits without the walk fields — the current one less its
// last 20 bytes — decodes as the 21-field generation, WITHOUT the walk keys (never `post*` passed off as the walk). A body
// ending inside the walk is neither shape and throws.
test('the pre-walk Swapped body decodes without the walk keys', () => {
  const now = swappedPayload();
  const pre = now.subarray(0, now.length - 20);
  const ev = decodeEventPayload(pre);
  assert.ok(ev);
  assert.equal(ev.name, 'Swapped');
  assert.equal(ev.data.walkSqrtPriceX64, undefined);
  assert.equal(ev.data.walkTick, undefined);
  assert.equal(ev.data.postSqrtPriceX64, (1n << 64n) - 5n);
  assert.deepEqual(ev.data.feeSplit1, [80n, 81n, 82n, 83n, 84n]);
  assert.throws(() => decodeEventPayload(now.subarray(0, now.length - 4)), 'the walk cut short is no shape');
});

test('a four-slot (pre-team-fee) Swapped body throws', () => {
  const five = swappedPayload().subarray(0, swappedPayload().length - 20); // without the walk
  const four = new Uint8Array([...five.subarray(0, five.length - 80), ...five.subarray(five.length - 80, five.length - 48), ...five.subarray(five.length - 40, five.length - 8)]);
  assert.equal(four.length, five.length - 16);
  assert.throws(() => decodeEventPayload(four));
});

// `protocol_fee_total` / `protocol_fee_currency` came with the five-slot split, so an older body without them is
// 25 B shorter — no protocol fields AND four slots — and it throws exactly as the four-slot body does (a reader
// that needs it keeps its own decode). A body with the protocol fields but four slots throws too.
test('a Swapped body without the Carpenter fee fields and with four slots throws', () => {
  const now = swappedPayload().subarray(0, swappedPayload().length - 20); // without the walk
  const tail = now.length - 80; // the splits
  const head = now.subarray(0, tail - 9); // everything before the two inserted fields
  const fourSlots = new Uint8Array([...now.subarray(tail, tail + 32), ...now.subarray(tail + 40, tail + 72)]);
  const legacy = new Uint8Array([...head, ...fourSlots]);
  assert.equal(legacy.length, now.length - 25);
  assert.throws(() => decodeEventPayload(legacy));
  const noFeeFields = new Uint8Array([...head, ...now.subarray(tail)]);
  assert.equal(noFeeFields.length, now.length - 9);
  assert.throws(() => decodeEventPayload(noFeeFields), 'five slots without the Carpenter fee fields is no shape either');
});

// ---- the schemas against the built IDL ---------------------------------------------------------
//
// The ONE table the SDK transcribes from Rust that has no Rust-source twin to parse: the events
// are the program's `#[event]` structs, and `anchor build` publishes them in
// the IDL's `events[]` (names + discriminators) and `types[]` (the field lists). A discriminator is
// `sha256("event:<Name>")[..8]` and says nothing about the fields, so without this case a field
// added, reordered or widened on the Rust side would mis-decode every trade a client reads, SILENTLY —
// a same-width change (`u64` → `i64`, two `u64`s swapped) never even trips `finish()`.

interface IdlType {
  readonly defined?: { readonly name: string };
  readonly array?: readonly [string, number];
  readonly option?: string;
  readonly vec?: string;
}
interface Idl {
  events?: readonly { readonly name: string }[];
  types?: readonly { readonly name: string; readonly type: { readonly kind: string; readonly fields?: readonly { readonly name: string; readonly type: string | IdlType }[] } }[];
}

/** `pending_recipient_id` → `pendingRecipientId` (Anchor's own IDL → TS convention). */
const camel = (s: string): string => s.replace(/_([a-z0-9])/g, (_, c: string) => c.toUpperCase());

/** The IDL type of one field → the `EventFieldKind` the SDK's reader must use for it. */
function idlKind(t: string | IdlType, where: string): EventFieldKind {
  if (typeof t === 'string') {
    if (['pubkey', 'bool', 'u8', 'u16', 'u32', 'i32', 'u64', 'i64', 'u128', 'i128', 'string'].includes(t)) return t as EventFieldKind;
    throw new Error(`${where}: no event field kind for IDL type ${t}`);
  }
  if (t.array && t.array[0] === 'u8' && t.array[1] === 32) return 'bytes32';
  // `Swapped.fee_split0 / fee_split1`. FIVE slots (the team fee).
  if (t.array && t.array[0] === 'u64' && t.array[1] === 5) return 'u64x5';
  if (t.array && t.array[0] === 'u64' && t.array[1] === 3) return 'u64x3';
  //  `VestingCreated.schedules`.
  if (t.array && t.array[0] === 'pubkey' && t.array[1] === 5) return 'pubkey5';
  // `TeamBought.vest_days`.
  if (t.array && t.array[0] === 'u16' && t.array[1] === 4) return 'u16x4';
  if (t.option === 'u32') return 'option_u32';
  if (t.vec === 'pubkey') return 'vec_pubkey';
  // `PayeeSlotsSet`'s two `Vec<SlotArg>` lists.
  if (typeof t.vec === 'object' && (t.vec as { defined?: string | { name: string } }).defined !== undefined) {
    const d = (t.vec as { defined: string | { name: string } }).defined;
    if ((typeof d === 'string' ? d : d.name) === 'SlotArg') return 'vec_slot_args';
  }
  throw new Error(`${where}: no event field kind for IDL type ${JSON.stringify(t)}`);
}

test('every EVENT_SCHEMAS entry equals the built IDL: the same events, the same fields, order and types', (t) => {
  let checked = 0;
  let programs = 0;
  for (const program of Object.keys(EVENT_SCHEMAS) as ProgramKey[]) {
    const idlPath = idlPathOf(program);
    if (!existsSync(idlPath)) continue;
    programs++;
    const idl = JSON.parse(readFileSync(idlPath, 'utf8')) as Idl;
    const types = new Map((idl.types ?? []).map((x) => [x.name, x]));
    const idlNames = (idl.events ?? []).map((e) => e.name);
    assert.deepEqual([...idlNames].sort(), [...EVENT_NAMES[program]].sort(), `${program}: the event NAME SET`);
    for (const name of idlNames) {
      const row = types.get(name);
      assert.ok(row, `${program}.${name} has a types[] entry`);
      assert.equal(row.type.kind, 'struct', `${program}.${name} is a struct`);
      // A field whose type is a `defined` STRUCT is written FLAT by Borsh (THE PRESET REGISTRY:
      // `PresetAdded.row: PresetRowArgs`), so it expands into that struct's own fields, in the IDL's order — which is
      // what proves `events.ts`' flattened list IS the struct's order. Any other defined type still throws by name.
      type Field = { readonly name: string; readonly type: string | IdlType };
      const flat = (fs: readonly Field[], where: string): [string, EventFieldKind][] =>
        fs.flatMap((f): [string, EventFieldKind][] => {
          const at = `${where}.${f.name}`;
          if (typeof f.type !== 'string' && f.type.defined) {
            const def = types.get(f.type.defined.name);
            assert.ok(def, `${at}: the IDL has no type ${f.type.defined.name}`);
            assert.equal(def.type.kind, 'struct', `${at}: ${f.type.defined.name} is a ${def.type.kind}, not a struct`);
            return flat(def.type.fields ?? [], at);
          }
          return [[camel(f.name), idlKind(f.type, at)]];
        });
      const want = flat(row.type.fields ?? [], `${program}.${name}`);
      const have = (EVENT_SCHEMAS[program][name] ?? []).map(([k, f]) => [k, f]);
      assert.deepEqual(have, want, `${program}.${name} fields`);
      checked++;
    }
  }
  if (checked === 0) t.diagnostic('no target/idl — run `anchor build` for the IDL half of this check');

  else if (programs === 5) assert.equal(checked, 133, 'every event in the five IDLs was compared');
  else t.diagnostic(`only ${programs} of 5 IDLs are built; ${checked} events compared`);
});
