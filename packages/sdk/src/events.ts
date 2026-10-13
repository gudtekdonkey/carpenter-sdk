/**
 * `carpenter_amm`'s `emit_cpi!` event schemas and the decoders. A hook's SDK reuses the decoder with its own programs'
 * schemas added (a hooked transaction emits both).
 *
 * `emit_cpi!` is a self-CPI whose instruction data is `EVENT_IX_TAG (8 B) ‖ sha256("event:<Name>")[..8] ‖ borsh(event)`,
 * signed by the program's `__event_authority` PDA; `decodeInnerInstructionEvents` reads those. A hook program's swap
 * callbacks emit the same way (their `#[event_cpi]` pair is part of the hook slice), so a hooked swap's events are all
 * inner instructions. `emit!` (`sol_log_data`) logs `disc ‖ borsh(event)` as a base64 `Program data:` line;
 * `decodeLogEvents` reads those, for a program that still logs an event that way.
 * The field lists transcribe the program's `#[event]` structs; `test/borsh-events.test.ts` holds `EVENT_SCHEMAS` field for
 * field against the shipped IDL.
 * Decode strictly: trailing bytes are an error (they mean the schema here is stale).
 * A discriminator proves NOTHING about a field list — it is `sha256("event:<Name>")[..8]`, so it matches whatever
 *    fields the schema carries. A widened field that keeps the total length unchanged decodes without error; the IDL
 *    comparison is what catches it.
 */

import { sha256 } from '@noble/hashes/sha2.js';
import type { Address } from '@solana/kit';

import { BorshReader } from './borsh.ts';
import { base64ToBytes, equalBytes } from './bytes.ts';

/** The field kinds an event schema can carry; `test/borsh-events.test.ts` maps each to an IDL type. */
export type EventFieldKind =
  | 'pubkey' | 'bool' | 'u8' | 'u16' | 'u32' | 'i32' | 'u64' | 'i64' | 'u128' | 'i128' | 'string' | 'bytes32' | 'u64x5' | 'u64x3' | 'u16x4' | 'option_u32' | 'vec_pubkey'
  //  `[Pubkey; 5]` — `VestingCreated.schedules`, the five locks in order.
  | 'pubkey5'
  // MULTI-RECIPIENT, `PayeeSlotsSet.creator /.team:
  // Vec<SlotArg>` — each `{ bps u16, platform u8, key [u8; 32] }` (35 B), decoded to `{ bps, platform, key }`.
  | 'vec_slot_args';
// `F` / `Schema` are short aliases of `EventFieldKind` / `EventSchema`; both spellings are exported.
export type F = EventFieldKind;
export type EventSchema = readonly (readonly [string, F])[];
export type Schema = EventSchema;

export type ProgramKey = 'carpenter_amm';

const SCHEMAS: Readonly<Record<ProgramKey, Readonly<Record<string, Schema>>>> = {
  carpenter_amm: {
    ConfigInitialized: [['config', 'pubkey'], ['authority', 'pubkey']],
    ConfigAuthorityChanged: [['config', 'pubkey'], ['previous', 'pubkey'], ['authority', 'pubkey'], ['accepted', 'bool']],
    HookApproved: [
      ['hookProgram', 'pubkey'], ['hookAuthority', 'pubkey'], ['flags', 'u16'], ['maxRounds', 'u8'], ['callbackAccounts', 'u8'],
      ['allowPublicPools', 'bool'], ['allowExclusivity', 'bool'], ['programdataSlot', 'u64'], ['name', 'bytes32'],
    ],
    HookPaused: [['hookProgram', 'pubkey'], ['paused', 'bool']],
    // A hook re-pinned after its program was upgraded.
    HookReapproved: [
      ['hookProgram', 'pubkey'], ['previousProgramdataSlot', 'u64'], ['programdataSlot', 'u64'], ['approvedSlot', 'u64'],
    ],
    // The quote ROSTER. Field ORDER is the wire contract -- a discriminator is
    // sha256("event:<Name>")[..8] and proves nothing about a field list, so the IDL comparison in
    // `test/borsh-events.test.ts` is what holds these to the program.
    QuoteMintListed: [['mint', 'pubkey'], ['caps', 'u8'], ['decimals', 'u8'], ['authorityDigest', 'bytes32']],
    QuoteMintRetired: [['mint', 'pubkey'], ['retiredSlot', 'u64']],
    // The way back from a retire. `caps` / `decimals` /
    // `authorityDigest` are RE-READ at relist, never copied from the original listing.
    QuoteMintRelisted: [
      ['mint', 'pubkey'], ['caps', 'u8'], ['decimals', 'u8'], ['authorityDigest', 'bytes32'], ['retiredSlot', 'u64'],
    ],
    QuoteMintDelisted: [
      ['mint', 'pubkey'], ['wasRetired', 'bool'], ['listedSlot', 'u64'], ['rentRefunded', 'u64'],
    ],
    // The governance ADMISSION MASK moved (`set_quote_caps_admitted`, SET not
    // OR — `previous` is what it was, `admitted` what it is; a withdrawn bit stops NEW listings and
    // NEW pools on the quote side and touches no live pool).
    QuoteCapsAdmittedSet: [['previous', 'u8'], ['admitted', 'u8']],
    // The config authority's per-hook switch, and a hook's own quote listing.
    HookQuotesSet: [['hookProgram', 'pubkey'], ['previous', 'bool'], ['allowed', 'bool']],
    QuoteMintHookListed: [['mint', 'pubkey'], ['hookProgram', 'pubkey'], ['caps', 'u8'], ['decimals', 'u8']],
    // The config authority's per-pool pause.
    PoolPauseSet: [['pool', 'pubkey'], ['paused', 'bool']],
    // The pause guardian. `pausedUntil` is unix seconds; the pause lapses on its own and
    // no event marks the end (a `PoolPauseSet` / `HookPaused` with `paused: false` is the authority ending it early).
    PauseGuardianSet: [['previous', 'pubkey'], ['guardian', 'pubkey']],
    GuardianPoolPaused: [['pool', 'pubkey'], ['guardian', 'pubkey'], ['pausedUntil', 'i64']],
    GuardianHookPaused: [['hookProgram', 'pubkey'], ['guardian', 'pubkey'], ['pausedUntil', 'i64']],
    PoolInitialized: [
      ['pool', 'pubkey'], ['mint0', 'pubkey'], ['mint1', 'pubkey'], ['fee', 'u32'], ['tickSpacing', 'u16'], ['hook', 'pubkey'],
      ['sqrtPriceX64', 'u128'], ['tick', 'i32'],
    ],
    LiquidityModified: [
      ['pool', 'pubkey'], ['owner', 'pubkey'], ['salt', 'u64'], ['tickLower', 'i32'], ['tickUpper', 'i32'], ['liquidityDelta', 'i128'],
      ['amount0', 'i128'], ['amount1', 'i128'], ['fees0', 'u64'], ['fees1', 'u64'],
      // the position's key, appended.
      ['position', 'pubkey'],
    ],
    FeesTruncated: [['position', 'pubkey'], ['currency', 'u8']],
    PositionClosed: [['position', 'pubkey'], ['owner', 'pubkey']],
    // A position carries its owner as a FIELD, so it can be transferred.
    PositionTransferred: [['position', 'pubkey'], ['pool', 'pubkey'], ['from', 'pubkey'], ['to', 'pubkey']],
    Swapped: [
      ['pool', 'pubkey'], ['trader', 'pubkey'], ['zeroForOne', 'bool'], ['amountIn', 'u64'], ['amountOut', 'u64'],
      ['hookDelta0', 'i128'], ['hookDelta1', 'i128'], ['lpFeeTotal', 'u64'], ['feeOverride', 'option_u32'],
      ['preSqrtPriceX64', 'u128'], ['preTick', 'i32'], ['postSqrtPriceX64', 'u128'], ['postTick', 'i32'], ['signals', 'u8'],
      // The hook's in-swap actions: `bid_replaced` / `absorbed` / `nudge_spent` (the program's `ActionTotals`),
      // then the two fee arrays, `[skim, creator, holder, platform]` per currency. `bid_replaced` is a BOOL: a table
      // that reads it as a wider field shifts every following byte and decodes plausible garbage without an error.
      ['bidReplaced', 'bool'], ['absorbed', 'u64'], ['nudgeSpent', 'u64'],
      // The fee the TRADER's walk booked owed
      // (`Pool.protocol_fees_owed0/1`) and the currency (0 / 1) it is in — on a quote-side pool the quote,
      // off the input on a buy and off the OUTPUT on a sell (`amountOut` is already net of it); 0 on a pool
      // at a 0 rate. Like `lpFeeTotal`, the trader's walk only (the nudge's fee is inside
      // `nudgeSpent`). The two fee arrays stay the payload's last bytes.
      ['protocolFeeTotal', 'u64'], ['protocolFeeCurrency', 'u8'],
      // [skim, creator, holder, platform, team] per currency (`team` is a CHARGE on top, `platform` stays inside
      // `skim`) — a body of any other shape throws: never a silently mis-sized event.
      ['feeSplit0', 'u64x5'], ['feeSplit1', 'u64x5'],
      // The price the TRADER's own walk reached,
      // before the hook's in-swap bid re-place and nudge moved the pool (`post*` is the pool AFTER them) — on a
      // sell into the bid, the dip the nudge hides. A candle closes here (the indexer's `trade_close_sqrt_x64`).
      // APPENDED: an older program version emits the 21-field body, which `LEGACY_PREFIXES` decodes WITHOUT these keys — `walkSqrtPriceX64 === undefined` is "not carried",
      // never `postSqrtPriceX64`.
      ['walkSqrtPriceX64', 'u128'], ['walkTick', 'i32'],
    ],
    Donated: [['pool', 'pubkey'], ['donor', 'pubkey'], ['amount0', 'u64'], ['amount1', 'u64']],
    HookDeposited: [['pool', 'pubkey'], ['currency', 'u8'], ['credited', 'u64']],
    HookWithdrew: [['pool', 'pubkey'], ['currency', 'u8'], ['amount', 'u64'], ['destination', 'pubkey']],
    MintExclusivityRegistered: [['mint', 'pubkey'], ['hookProgram', 'pubkey'], ['protocolFee', 'u32']],
    MintExclusivityRevoked: [['mint', 'pubkey'], ['hookProgram', 'pubkey']],
    LpFeeUpdated: [['pool', 'pubkey'], ['lpFee', 'u32']],
    // The config authority named, ONCE, the transfer-hook program a pool of `hookProgram` may carry ARMED on its base mint.
    BaseTransferHookSet: [['hookProgram', 'pubkey'], ['transferHookProgram', 'pubkey']],
    // An empty tick array closed by its rent owner. `lamports` is everything the close moved to `rentOwner` (the rent plus anything credited since).
    TickArrayClosed: [['pool', 'pubkey'], ['tickArray', 'pubkey'], ['startIndex', 'i32'], ['rentOwner', 'pubkey'], ['lamports', 'u64']],
    // The protocol treasury's four events:
    //   · `ProtocolTreasuryGovernanceChanged` — `initialize_protocol_treasury` (`previous` the zero key,
    //     `accepted` true) and both handover steps (`accepted` false = a proposal, zero `governance` = a cancel).
    //   · `ProtocolFeesSwept` — the PERMISSIONLESS `sweep_protocol_fees` moved `amount` of `pool`'s owed in
    //     `currency` into the treasury's ATA; never emitted for an empty sweep. The indexer's re-read cue.
    //   · `ProtocolFeesWithdrawn` — governance moved `amount` out; `remaining` is read back off the source.
    //   · `ProtocolFeeSet` — `set_protocol_fee` moved the setting NEW pools copy (both v4-PACKED; `previous`
    //     is what `initialize_pool` would have copied, the default 2,048,500 on a never-written config). No
    //     live pool changes.
    ProtocolTreasuryGovernanceChanged: [['treasury', 'pubkey'], ['previous', 'pubkey'], ['governance', 'pubkey'], ['accepted', 'bool']],
    ProtocolFeesSwept: [['pool', 'pubkey'], ['currency', 'u8'], ['mint', 'pubkey'], ['destination', 'pubkey'], ['amount', 'u64'], ['sweeper', 'pubkey']],
    ProtocolFeesWithdrawn: [['mint', 'pubkey'], ['source', 'pubkey'], ['destination', 'pubkey'], ['governance', 'pubkey'], ['amount', 'u64'], ['remaining', 'u64']],
    ProtocolFeeSet: [['previous', 'u32'], ['protocolFee', 'u32']],
  },
};

/** `sha256(preimage)[..8]`. */
export function anchorDiscriminator(preimage: string): Uint8Array {
  return sha256(new TextEncoder().encode(preimage)).slice(0, 8);
}

/**
 * An event NAME must be unique across the five programs: `eventDiscriminator(name)` and an unfiltered
 * `decodeEventPayload` resolve by name / discriminator alone. `floor_vesting.Claimed` is the nearest
 * neighbour (`CreatorClaimed`, `ReferralPaid`, `HolderClaimed` are all distinct names), and this
 * module refuses to load if two programs ever declare the same one.
 */
{
  const seen = new Map<string, string>();
  for (const [program, events] of Object.entries(SCHEMAS)) {
    for (const name of Object.keys(events)) {
      const other = seen.get(name);
      if (other) throw new Error(`events.ts: ${name} is declared by both ${other} and ${program} — an unfiltered decode would be ambiguous`);
      seen.set(name, program);
    }
  }
}

/**
 * Anchor's `EVENT_IX_TAG_LE`: the u64 `0x1d9acb512ea545e4` (= `sha256("anchor:event")[..8]` read
 * big-endian) written LITTLE-endian, as `emit_cpi!` puts it on the wire — i.e. the sha prefix
 * reversed: `e4 45 a5 2e 51 cb 9a 1d`. Not the sha prefix itself (`test/borsh-events.test.ts`
 * derives it from node:crypto).
 */
export const EVENT_IX_TAG: Uint8Array = anchorDiscriminator('anchor:event').reverse();

export interface DecodedEvent {
  readonly program: ProgramKey;
  readonly name: string;
  readonly data: Readonly<Record<string, unknown>>;
}

interface Entry {
  program: ProgramKey;
  name: string;
  disc: Uint8Array;
  schema: Schema;
}

const ENTRIES: Entry[] = Object.entries(SCHEMAS).flatMap(([program, events]) =>
  Object.entries(events).map(([name, schema]) => ({ program: program as ProgramKey, name, disc: anchorDiscriminator(`event:${name}`), schema })),
);

/**
 * The decoders’ READ SEQUENCE, in order, per program. `decodeEventPayload` walks exactly this
 * table, so it IS the wire schema this SDK believes in — exported so `test/borsh-events.test.ts`
 * can hold it, field by field and IN ORDER, to the shipped IDL and map each field kind to an IDL type.
 * Borsh carries no field names: ORDER is the whole contract.
 * Read-only: decode through `decodeEventPayload`, never by walking this.
 */
export const EVENT_SCHEMAS: Readonly<Record<ProgramKey, Readonly<Record<string, EventSchema>>>> = SCHEMAS;

/**
 * **EARLIER GENERATIONS OF AN EVENT THAT AN OLDER PROGRAM VERSION EMITS**, each
 * as a PREFIX LENGTH of the current schema — so a legacy shape is the current one's first N fields BY
 * CONSTRUCTION and cannot drift from it (fields are only ever APPENDED; Borsh has no names, so an
 * inserted field would break every decoder and is not a generation). Here: `carpenter_amm.Swapped`'s
 * 21-field shape, ending at `feeSplit1`, before the trade's own walk was appended.
 * A legacy decode is TRIED ONLY AFTER the current schema refuses, and each shape is still decoded
 * STRICTLY (`finish`): a body that ends INSIDE the appended tail, or runs past it, is neither shape
 * and throws exactly as before. The data of a legacy decode simply LACKS the appended keys —
 * `quoteMint === undefined` means "this event predates the field", never wSOL and never zero.
 */
const LEGACY_PREFIXES: Readonly<Partial<Record<ProgramKey, Readonly<Record<string, readonly number[]>>>>> = {
  // `Swapped`: the 21-field shape ending at `feeSplit1`.
  carpenter_amm: { Swapped: [21] },
};

/** The legacy shapes `decodeEventPayload` accepts, derived from `LEGACY_PREFIXES` (read-only, for tests). */
export const EVENT_LEGACY_SCHEMAS: Readonly<Partial<Record<ProgramKey, Readonly<Record<string, readonly EventSchema[]>>>>> =
  Object.fromEntries(
    Object.entries(LEGACY_PREFIXES).map(([program, events]) => [
      program,
      Object.fromEntries(
        Object.entries(events ?? {}).map(([name, lens]) => [
          name,
          lens.map((n) => SCHEMAS[program as ProgramKey][name]!.slice(0, n)),
        ]),
      ),
    ]),
  );

export const EVENT_NAMES: Readonly<Record<ProgramKey, readonly string[]>> = {
  carpenter_amm: Object.keys(SCHEMAS.carpenter_amm),
};

export function eventDiscriminator(name: string): Uint8Array {
  const e = ENTRIES.find((x) => x.name === name);
  if (!e) throw new Error(`unknown event ${name}`);
  return e.disc;
}

function readValue(r: BorshReader, f: F): unknown {
  switch (f) {
    case 'pubkey':
      return r.pubkey();
    case 'bool':
      return r.bool();
    case 'u8':
      return r.u8();
    case 'u16':
      return r.u16();
    case 'u32':
      return r.u32();
    case 'i32':
      return r.i32();
    case 'u64':
      return r.u64();
    case 'i64':
      return r.i64();
    case 'u128':
      return r.u128();
    case 'i128':
      return r.i128();
    case 'string':
      return r.string();
    case 'bytes32':
      return r.fixed(32);
    // `[u64; 4]`, the fee split `[skim, creator, holder, platform]`;
    // `[u64; 5]`: `[skim, creator, holder, platform, team]`.
    case 'u64x5':
      return [r.u64(), r.u64(), r.u64(), r.u64(), r.u64()];
    // `TraderReferralBooked.s`, the three tiers' slices.
    case 'u64x3':
      return [r.u64(), r.u64(), r.u64()];
    // `TeamBought.vest_days`.
    case 'u16x4':
      return [r.u16(), r.u16(), r.u16(), r.u16()];
    case 'option_u32':
      return r.option(() => r.u32());
    case 'vec_pubkey':
      return r.vec((): Address => r.pubkey());
    case 'pubkey5':
      return [r.pubkey(), r.pubkey(), r.pubkey(), r.pubkey(), r.pubkey()];
    case 'vec_slot_args':
      return r.vec(() => ({ bps: r.u16(), platform: r.u8(), key: r.fixed(32) }));
  }
}

/** Decode `disc ‖ borsh(event)` (the payload of `emit!` and the tail of `emit_cpi!`). */
export function decodeEventPayload(bytes: Uint8Array, program?: ProgramKey): DecodedEvent | null {
  if (bytes.length < 8) return null;
  const head = bytes.subarray(0, 8);
  const e = ENTRIES.find((x) => (program === undefined || x.program === program) && equalBytes(x.disc, head));
  if (!e) return null;
  const body = bytes.subarray(8);
  try {
    return { program: e.program, name: e.name, data: decodeStrict(body, e.schema) };
  } catch (err) {
    // Only an event with a declared earlier generation gets a second try, and only a shape that
    // decodes EXACTLY wins; otherwise the CURRENT schema's error is what the caller sees.
    for (const legacy of EVENT_LEGACY_SCHEMAS[e.program]?.[e.name] ?? []) {
      try {
        return { program: e.program, name: e.name, data: decodeStrict(body, legacy) };
      } catch {
        // not this generation either
      }
    }
    throw err;
  }
}

/** Decode `body` as exactly `schema` — every field, then no trailing byte. */
function decodeStrict(body: Uint8Array, schema: Schema): Record<string, unknown> {
  const r = new BorshReader(body);
  const data: Record<string, unknown> = {};
  for (const [k, f] of schema) data[k] = readValue(r, f);
  r.finish();
  return data;
}

/** Decode one `emit_cpi!` inner-instruction data blob; `null` if it is not an event CPI. */
export function decodeEmitCpiData(ixData: Uint8Array, program?: ProgramKey): DecodedEvent | null {
  if (ixData.length < 16 || !equalBytes(ixData.subarray(0, 8), EVENT_IX_TAG)) return null;
  return decodeEventPayload(ixData.subarray(8), program);
}

/**
 * Decode every FLOOR event in a transaction's inner instructions. `innerInstructions` is the
 * RPC's `meta.innerInstructions` shape, already base58/base64-decoded to bytes by the caller;
 * `programOf(index)` maps an account index to its address. Only CPIs INTO one of our programs,
 * whose data starts with `EVENT_IX_TAG`, are considered.
 */
export function decodeInnerInstructionEvents(
  inner: readonly { readonly instructions: readonly { readonly programIdIndex: number; readonly data: Uint8Array }[] }[],
  accountKeys: readonly Address[],
  // PARTIAL: a caller that does not read `floor_holders` / `floor_vesting` events
  //   passes the programs it does; an absent key simply decodes nothing for that program.
  programs: Readonly<Partial<Record<ProgramKey, Address>>>,
): DecodedEvent[] {
  const byAddress = new Map<string, ProgramKey>(Object.entries(programs).filter(([, v]) => v !== undefined).map(([k, v]) => [v as string, k as ProgramKey]));
  const out: DecodedEvent[] = [];
  for (const group of inner) {
    for (const ix of group.instructions) {
      const program = byAddress.get(accountKeys[ix.programIdIndex] ?? '');
      if (!program) continue;
      const ev = decodeEmitCpiData(ix.data, program);
      if (ev) out.push(ev);
    }
  }
  return out;
}

/** `emit!` logs `Program data: <base64>`; decode each such line. */
export function decodeLogEvents(logs: readonly string[], program?: ProgramKey): DecodedEvent[] {
  const out: DecodedEvent[] = [];
  for (const line of logs) {
    const m = /^Program data: ([A-Za-z0-9+/=]+)$/.exec(line);
    if (!m || m[1] === undefined) continue;
    const ev = decodeEventPayload(base64ToBytes(m[1]), program);
    if (ev) out.push(ev);
  }
  return out;
}
