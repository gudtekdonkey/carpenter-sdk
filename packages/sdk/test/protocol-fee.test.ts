// the SDK's side:
//   · the RATE helpers (`src/protocolFee.ts`) against `crates/carpenter-types/src/protocol_fee.rs`'s own vectors;
//   · the carves (`AmmConfig` @177..@184, `Pool.protocol_fee_currency` @538) and the `["protocol_treasury"]` account;
//   · the six builders (`src/tx/protocolTreasury.ts`): open, the permissionless sweep, the uncapped withdraw, the
//     two-step handover and the capped setter — each naming carpenter_amm's treasury, never floor_hook's;
//   · the treasury's four events.
// The CHARGED amounts are the quoter's (the program's crate as wasm) and the chain's; the program repository's
// tests hold those. What is pinned here is what a client builds, reads and shows: the setter's bytes, the
// read-back of a config written to (250, 750), a 0 setting (expressible, and a pool at 0 shows NO fee), a value
// above the cap (1,001, refused by name before a signature), and the setter's only signer slot (the treasury's
// governance — never `AmmConfig.authority`).

import assert from 'node:assert/strict';
import { test } from './node-test.ts';

import {
  BorshWriter,
  readUintLE,

  MAX_PROTOCOL_FEE,
  PROTOCOL_FEE_DEFAULT,
  PROTOCOL_FEE_DEFAULT_PACKED,
  PROTOCOL_FEE_ON_CURRENCY0,
  PROTOCOL_FEE_ON_CURRENCY1,
  PROTOCOL_FEE_ON_INPUT,

  

  
  AMM_CONFIG,

  MINT_EXCLUSIVITY,
  POOL,

  decodeAmmConfig,
  decodeMintExclusivity,
  decodePool,

  encodeAccount,

  
  composeSwapFee,
  isValidProtocolFee,
  launchProtocolFee,
  packProtocolFee,
  protocolFeeForNewPool,
  protocolFeeCurrency,
  protocolFeeDirectional,
  protocolFeeOf,
  protocolFeeOnOutput,
  validateProtocolFee,
} from '../src/index.ts';
import { decodeEventPayload, eventDiscriminator } from '../src/events.ts';
import { addr } from './helpers.ts';

// ---- the rate helpers, vector for vector against protocol_fee.rs's tests -------------------------------

test('the default is 500 each way, packed 2,048,500, capped at 1,000; v4 composes 500 on 2,500 to 2,999', () => {
  assert.equal(PROTOCOL_FEE_DEFAULT, 500);
  assert.equal(MAX_PROTOCOL_FEE, 1_000);
  assert.equal(PROTOCOL_FEE_DEFAULT_PACKED, 2_048_500);
  assert.equal(packProtocolFee(500, 500), PROTOCOL_FEE_DEFAULT_PACKED);
  assert.equal(protocolFeeDirectional(PROTOCOL_FEE_DEFAULT_PACKED, true), 500);
  assert.equal(protocolFeeDirectional(PROTOCOL_FEE_DEFAULT_PACKED, false), 500);
  assert.ok(isValidProtocolFee(PROTOCOL_FEE_DEFAULT_PACKED));
  assert.equal(composeSwapFee(500, 2_500), 2_999, 'FLOOR 0.25 % + 0.05 %: the protocol share comes off first');
  assert.equal(composeSwapFee(500, 0), 500, 'no LP fee: the composed rate IS the protocol rate');
  assert.equal(composeSwapFee(0, 2_500), 2_500);
});

test("v4's nibble order, and the cap is inclusive; nothing above the two nibbles is valid", () => {
  const p = packProtocolFee(1_000, 2_000);
  assert.equal(p & 0xfff, 1_000, 'getZeroForOneFee = self & 0xfff');
  assert.equal((p >>> 12) & 0xfff, 2_000, 'getOneForZeroFee = self >> 12');
  assert.equal(protocolFeeDirectional(p, true), 1_000);
  assert.equal(protocolFeeDirectional(p, false), 2_000);
  assert.ok(isValidProtocolFee(packProtocolFee(1_000, 1_000)));
  assert.ok(isValidProtocolFee(packProtocolFee(0, 0)), '0 is a setting');
  assert.ok(!isValidProtocolFee(packProtocolFee(1_001, 0)));
  assert.ok(!isValidProtocolFee(packProtocolFee(0, 1_001)));
  assert.ok(!isValidProtocolFee(packProtocolFee(4_095, 4_095)));
  assert.ok(!isValidProtocolFee(1 << 24), 'a bit above the two nibbles');
});

test('the quote-side rule ("SOL / quote side") and the input fallback', () => {
  // a FLOOR pool: the quote is currency 0 — a buy pays in what it puts in, a sell in what it receives
  assert.equal(protocolFeeCurrency(PROTOCOL_FEE_ON_CURRENCY0, true), 0);
  assert.equal(protocolFeeCurrency(PROTOCOL_FEE_ON_CURRENCY0, false), 0);
  assert.ok(!protocolFeeOnOutput(PROTOCOL_FEE_ON_CURRENCY0, true));
  assert.ok(protocolFeeOnOutput(PROTOCOL_FEE_ON_CURRENCY0, false));
  assert.equal(protocolFeeCurrency(PROTOCOL_FEE_ON_CURRENCY1, true), 1);
  assert.equal(protocolFeeCurrency(PROTOCOL_FEE_ON_CURRENCY1, false), 1);
  assert.ok(protocolFeeOnOutput(PROTOCOL_FEE_ON_CURRENCY1, true));
  assert.ok(!protocolFeeOnOutput(PROTOCOL_FEE_ON_CURRENCY1, false));
  for (const code of [PROTOCOL_FEE_ON_INPUT, 7]) {
    assert.equal(protocolFeeCurrency(code, true), 0);
    assert.equal(protocolFeeCurrency(code, false), 1);
    assert.ok(!protocolFeeOnOutput(code, true) && !protocolFeeOnOutput(code, false));
  }
});

test("a pool's own rate: composed on a FLOOR buy, off the output on a sell, nothing on a pool at 0", () => {
  const floorPool = { protocolFee: PROTOCOL_FEE_DEFAULT_PACKED, protocolFeeCurrency: PROTOCOL_FEE_ON_CURRENCY0 };
  assert.deepEqual(protocolFeeOf(floorPool, true, 2_500), { pips: 500, currency: 0, onOutput: false, stepFee: 2_999 });
  assert.deepEqual(protocolFeeOf(floorPool, false, 2_500), { pips: 500, currency: 0, onOutput: true, stepFee: 2_500 });
  // "Only launches created after the change.": a pool created before it reads 0 and SHOWS no fee (never "0%")
  assert.deepEqual(protocolFeeOf({ protocolFee: 0, protocolFeeCurrency: PROTOCOL_FEE_ON_INPUT }, false, 3_000), { pips: 0, currency: 1, onOutput: false, stepFee: 3_000 });
  // a pool created at a 0 SETTING ("try also 0 fee") is the same: nothing charged, nothing shown
  assert.equal(protocolFeeOf({ protocolFee: 0, protocolFeeCurrency: PROTOCOL_FEE_ON_CURRENCY0 }, true, 2_500).pips, 0);
  // a pool created after "set another value" (250, 750) charges ITS rate each way
  const moved = { protocolFee: packProtocolFee(250, 750), protocolFeeCurrency: PROTOCOL_FEE_ON_INPUT };
  assert.equal(protocolFeeOf(moved, true).pips, 250);
  assert.equal(protocolFeeOf(moved, false).pips, 750);
});

test('validateProtocolFee: 0..=1,000 each way, refused ProtocolFeeTooLarge above it, by name', () => {
  assert.equal(validateProtocolFee(500, 500), PROTOCOL_FEE_DEFAULT_PACKED);
  assert.equal(validateProtocolFee(1_000, 1_000), packProtocolFee(1_000, 1_000));
  assert.equal(validateProtocolFee(250, 750), 250 | (750 << 12), 'zero_for_one is the LOW nibble');
  assert.equal(validateProtocolFee(0, 0), 0, 'try also 0 fee');
  assert.throws(() => validateProtocolFee(1_001, 0), /ProtocolFeeTooLarge/);
  assert.throws(() => validateProtocolFee(0, 1_001), /ProtocolFeeTooLarge/);
  assert.throws(() => validateProtocolFee(-1, 0), /not a u16/);
  assert.throws(() => validateProtocolFee(0.5, 0), /not a u16/);
});

// ---- the carves and the treasury account --------------------------------------------------------------

test('AmmConfig: pad2 @177 (padding), protocol_fee @180 (account 188); read as written', () => {
  const at = (n: string) => AMM_CONFIG.fields.find((f) => f.name === n)!;
  assert.deepEqual([at('quote_caps_admitted').offset, at('pad2').offset, at('protocol_fee').offset, at('reserved').offset], [176, 177, 180, 184]);
  assert.ok(!AMM_CONFIG.fields.some((f) => f.name === 'protocol_fee_written'), 'the flag is padding');
  assert.equal(AMM_CONFIG.size, 208, 'size unchanged');
  const written = encodeAccount(AMM_CONFIG, { protocolFee: packProtocolFee(250, 750) });
  assert.equal(readUintLE(written, 8 + 180, 4), BigInt(packProtocolFee(250, 750)));
  const c = decodeAmmConfig(written);
  assert.deepEqual([c.protocolFee, c.currentProtocolFee], [packProtocolFee(250, 750), packProtocolFee(250, 750)]);
  // what `initialize_config` writes on a fresh deploy
  assert.equal(decodeAmmConfig(encodeAccount(AMM_CONFIG, { protocolFee: PROTOCOL_FEE_DEFAULT_PACKED })).currentProtocolFee, PROTOCOL_FEE_DEFAULT_PACKED);
  // an explicit 0 written by governance is 0 ("try also 0 fee") — and so are zero bytes: nothing reads a default any more
  assert.equal(decodeAmmConfig(encodeAccount(AMM_CONFIG, { protocolFee: 0 })).currentProtocolFee, 0);
});

test('Pool: protocol_fee_currency @538 (account 546); protocol_fee @16 and the owed pair @344/@352 did not move', () => {
  const at = (n: string) => POOL.fields.find((f) => f.name === n)!;
  assert.deepEqual([at('protocol_fee').offset, at('protocol_fees_owed0').offset, at('protocol_fees_owed1').offset, at('base_hook_side').offset, at('protocol_fee_currency').offset, at('reserved').offset], [16, 344, 352, 537, 538, 539]);
  assert.equal(POOL.size, 632);
  const bytes = encodeAccount(POOL, { protocolFee: PROTOCOL_FEE_DEFAULT_PACKED, protocolFeeCurrency: PROTOCOL_FEE_ON_CURRENCY0, protocolFeesOwed0: 7n });
  assert.equal(bytes[8 + 538], 1);
  const p = decodePool(bytes);
  assert.deepEqual([p.protocolFee, p.protocolFeeCurrency, p.protocolFeesOwed0, p.protocolFeesOwed1], [PROTOCOL_FEE_DEFAULT_PACKED, 1, 7n, 0n]);
  assert.deepEqual(protocolFeeOf(p, true, 2_500), { pips: 500, currency: 0, onOutput: false, stepFee: 2_999 });
});

// ---- 

test('a launch pays the rate its mint recorded at creation; a direct pool the setting now', () => {
  // the record round-trips at the program's offsets: has_protocol_fee @65, protocol_fee @68 (struct)
  const bytes = encodeAccount(MINT_EXCLUSIVITY, { mint: addr(1), hookProgram: addr(2), bump: 254, hasProtocolFee: 1, protocolFee: packProtocolFee(300, 700) });
  assert.equal(bytes.length, 80);
  assert.equal(bytes[8 + 65], 1);
  assert.equal(readUintLE(bytes, 8 + 68, 4), BigInt(packProtocolFee(300, 700)));
  const e = decodeMintExclusivity(bytes);
  assert.equal(e.protocolFee, packProtocolFee(300, 700));

  const now = { protocolFee: packProtocolFee(1_000, 250) };
  assert.equal(protocolFeeForNewPool(now, null), now.protocolFee, 'no entry: the setting now');
  assert.equal(protocolFeeForNewPool(now, { hasProtocolFee: 0, protocolFee: 0 }), now.protocolFee, 'an entry that recorded nothing is not a 0 rate');
  assert.equal(protocolFeeForNewPool(now, null, e), packProtocolFee(300, 700), 'the recorded rate, not the setting');
  assert.equal(protocolFeeForNewPool(now, { hasProtocolFee: 1, protocolFee: 0 }), 0, 'a recorded 0 is a rate');
  assert.equal(
    protocolFeeForNewPool(now, { hasProtocolFee: 1, protocolFee: packProtocolFee(200, 900) }, { hasProtocolFee: 1, protocolFee: packProtocolFee(600, 100) }),
    packProtocolFee(200, 100),
    'both recorded: the lower per direction (the program\'s rate_for_new_pool)',
  );

  // a sale shows its recorded rate; a graduated launch its pool's own; neither → unknown, never the setting
  assert.equal(launchProtocolFee(null, e), packProtocolFee(300, 700));
  assert.equal(launchProtocolFee({ protocolFee: packProtocolFee(300, 700) }, e), packProtocolFee(300, 700));
  assert.equal(launchProtocolFee(null, null), null);
  assert.equal(launchProtocolFee(null, { hasProtocolFee: 0, protocolFee: 0 }), null);

  // the event carries the recorded rate (appended)
  const ev = decodeEventPayload(new Uint8Array([
    ...eventDiscriminator('MintExclusivityRegistered'),
    ...new BorshWriter().pubkey(addr(1)).pubkey(addr(2)).u32(PROTOCOL_FEE_DEFAULT_PACKED).toBytes(),
  ]));
  assert.deepEqual(ev?.data, { mint: addr(1), hookProgram: addr(2), protocolFee: PROTOCOL_FEE_DEFAULT_PACKED });
});
