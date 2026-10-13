/**
 * Uniswap v4's PROTOCOL fee on Carpenter pools — as the RATE arithmetic a client shows and checks. The TS twin of the
 * RATE half of `crates/carpenter-types/src/protocol_fee.rs` (`directional`, `pack`, `is_valid`,
 * `compose_swap_fee`, `fee_currency`, `on_output`, `current_setting`); `test/protocol-fee.test.ts` pins
 * each to that file's own vectors.
 * NOT the walk. What a swap is CHARGED (the per-step `input_share`, the one `output_share` off a sell's
 *    output, the exact-out gross-up) is the quoter's, and the quoter is the program's own crate compiled to
 *    wasm (`SwapQuote.fees.protocol`). Never re-derive a charged amount here from a
 *    rate: the program rounds PER STEP, so a total recomputed from the pool's rate is not what it booked.
 * The rules this module implements:
 * - The fee is charged per direction ON TOP of the LP fee (default 500 pips = 0.05 %), composed as v4 does,
 *   `p + l − p·l/1e6`, when the fee rides the input (0.25 % LP + 0.05 % walks at 2,999 pips, not 3,000).
 * - Governance may set 0..=1,000 pips per direction ({@link validateProtocolFee}).
 * - `AmmConfig`'s CURRENT SETTING is COPIED into `Pool.protocol_fee` at `initialize_pool`; a live pool never changes.
 *   So a figure for a POOL reads THAT pool's `protocolFee` / `protocolFeeCurrency` ({@link protocolFeeOf}); "what a
 *   new pool pays" reads the config (`AmmConfig.protocolFee`, as written); neither is ever the constant.
 *   A hook launch whose pool is created later (a FLOOR launch's, at graduation) has its rate fixed when the launch is
 *   created — recorded on its mint's `MintExclusivity` — and that pool charges THAT rate. So a figure for a sale (no pool
 *   yet) reads {@link launchProtocolFee}, never the config's current setting.
 * - Charged on the pool's ONE listed quote (SOL / the quote side) on buys AND sells (off what a seller receives); a pool
 *   of two plain tokens (or of two quotes) pays in its input coin (v4's rule).
 * - 0 is a real setting: a pool at 0 charges nothing, and a client SHOWS no fee (no row, never "0%").
 */

import {
  MAX_PROTOCOL_FEE,
  PIPS_DENOMINATOR,
  PROTOCOL_FEE_ON_CURRENCY0,
  PROTOCOL_FEE_ON_CURRENCY1,
} from './constants.ts';

/** v4 `getZeroForOneFee` / `getOneForZeroFee`: the LOW 12-bit nibble for `zeroForOne`, the next for the other way. */
export function protocolFeeDirectional(packed: number, zeroForOne: boolean): number {
  return zeroForOne ? packed & 0xfff : (packed >>> 12) & 0xfff;
}

/** The inverse of {@link protocolFeeDirectional}: `zeroForOne | oneForZero << 12`, each masked to its nibble. */
export function packProtocolFee(zeroForOne: number, oneForZero: number): number {
  return ((zeroForOne & 0xfff) | ((oneForZero & 0xfff) << 12)) >>> 0;
}

/** v4 `isValidProtocolFee` with the cap: each direction ≤ `MAX_PROTOCOL_FEE`, nothing above the two nibbles. */
export function isValidProtocolFee(packed: number): boolean {
  return (
    Number.isInteger(packed) &&
    packed >= 0 &&
    packed >>> 24 === 0 &&
    protocolFeeDirectional(packed, true) <= MAX_PROTOCOL_FEE &&
    protocolFeeDirectional(packed, false) <= MAX_PROTOCOL_FEE
  );
}

/**
 * V4 `calculateSwapFee`: `p + l − floor(p·l / 1e6)` — the rate a step is charged when the protocol fee
 * rides the INPUT. 500 on FLOOR's 2,500 LP fee is 2,999: the protocol's share comes off first.
 * Exact in a JS number: `p·l < 2^12 · 2^24`.
 */
export function composeSwapFee(protocolPips: number, lpFee: number): number {
  const p = protocolPips & 0xfff;
  const l = lpFee & 0xff_ffff;
  return p + l - Math.floor((p * l) / PIPS_DENOMINATOR);
}

/** The currency (0 / 1) a swap in this direction pays its protocol fee in, from the pool's `protocolFeeCurrency` code. */
export function protocolFeeCurrency(code: number, zeroForOne: boolean): 0 | 1 {
  if (code === PROTOCOL_FEE_ON_CURRENCY0) return 0;
  if (code === PROTOCOL_FEE_ON_CURRENCY1) return 1;
  return zeroForOne ? 0 : 1; // PROTOCOL_FEE_ON_INPUT, and any other byte: v4's input rule
}

/** True when this swap's protocol fee comes off its OUTPUT (the listed quote is what it receives — a FLOOR sell). */
export function protocolFeeOnOutput(code: number, zeroForOne: boolean): boolean {
  return protocolFeeCurrency(code, zeroForOne) !== (zeroForOne ? 0 : 1);
}

// `AmmConfig.protocol_fee` is read AS WRITTEN (`initialize_config` always writes `PROTOCOL_FEE_DEFAULT_PACKED`).

/** What one direction of one pool charges, for a ticket or a pool page. */
export interface PoolProtocolFee {
  /** Pips of 1,000,000 (500 = 0.05 %). 0 = this pool charges no Carpenter Platform fee: SHOW NO ROW, never "0%". */
  readonly pips: number;
  /** The currency (0 / 1) it is charged in. */
  readonly currency: 0 | 1;
  /** True when it comes off the OUTPUT (a sell on a quote-side pool: what the seller receives is net of it). */
  readonly onOutput: boolean;
  /** The rate the walk's steps run at — `composeSwapFee(pips, lpFee)` on the input, the LP fee alone on the output. */
  readonly stepFee: number;
}

/**
 * The Carpenter Platform fee THIS pool charges a swap in this direction — its OWN copied rate
 * (`Pool.protocol_fee`, `Pool.protocol_fee_currency`), never the config's or the constant. `lpFee` is the
 * rate the walk runs at (a hook override, or the pool's `lpFee`); it only feeds `stepFee`.
 * The program's `protocol_fee::walk_fee`, minus the per-step split (that is the quoter's).
 */
export function protocolFeeOf(pool: { protocolFee: number; protocolFeeCurrency: number }, zeroForOne: boolean, lpFee = 0): PoolProtocolFee {
  const pips = protocolFeeDirectional(pool.protocolFee, zeroForOne);
  const currency = protocolFeeCurrency(pool.protocolFeeCurrency, zeroForOne);
  const onOutput = pips !== 0 && protocolFeeOnOutput(pool.protocolFeeCurrency, zeroForOne);
  const stepFee = pips === 0 || onOutput ? lpFee : composeSwapFee(pips, lpFee);
  return { pips, currency, onOutput, stepFee };
}

/**
 *  the packed Carpenter Platform fee a pool will copy at
 * `initialize_pool` — the program's `protocol_fee::rate_for_new_pool`. A FLOOR launch's mint carries a
 * `MintExclusivity` that recorded the setting at the launch's CREATION (`hasProtocolFee` 1): its pool charges
 * that rate whatever `AmmConfig` says by graduation. A mint with no entry (or one that recorded nothing) takes
 * `config.protocolFee`, the setting now. Pass both sides' entries for a pool of two (`null` where absent); a
 * sale's figure passes its mint's entry alone.
 * For a LIVE pool read its own `Pool.protocolFee` ({@link protocolFeeOf}); this is for a pool not yet created.
 */
export function protocolFeeForNewPool(
  config: { protocolFee: number },
  exclusivity0: { hasProtocolFee: number; protocolFee: number } | null,
  exclusivity1: { hasProtocolFee: number; protocolFee: number } | null = null,
): number {
  const r0 = exclusivity0 && exclusivity0.hasProtocolFee === 1 ? exclusivity0.protocolFee : null;
  const r1 = exclusivity1 && exclusivity1.hasProtocolFee === 1 ? exclusivity1.protocolFee : null;
  if (r0 === null && r1 === null) return config.protocolFee;
  if (r0 === null) return r1 as number;
  if (r1 === null) return r0;
  return packProtocolFee(
    Math.min(protocolFeeDirectional(r0, true), protocolFeeDirectional(r1, true)),
    Math.min(protocolFeeDirectional(r0, false), protocolFeeDirectional(r1, false)),
  );
}

/**
 * A FLOOR launch's Carpenter Platform fee, before and after graduation alike: its pool's own rate once the pool
 * exists, else the rate its mint's `MintExclusivity` recorded at `create_launch`. `null` when
 * neither is readable — never the config's current setting, which may have changed since the launch was created.
 */
export function launchProtocolFee(
  pool: { protocolFee: number } | null,
  exclusivity: { hasProtocolFee: number; protocolFee: number } | null,
): number | null {
  if (pool) return pool.protocolFee;
  if (exclusivity && exclusivity.hasProtocolFee === 1) return exclusivity.protocolFee;
  return null;
}

/**
 * `set_protocol_fee`'s rule, client-side: each direction an integer in `0..=MAX_PROTOCOL_FEE` (1,000 pips,
 * 0.1 %), 0 allowed; above it the program refuses `ProtocolFeeTooLarge` (6081) — refused here first, by
 * that name, before a signature is spent. Returns the packed setting the program will store.
 */
export function validateProtocolFee(zeroForOne: number, oneForZero: number): number {
  for (const [side, v] of [['zero_for_one', zeroForOne], ['one_for_zero', oneForZero]] as const) {
    if (!Number.isInteger(v) || v < 0 || v > 0xffff) throw new Error(`set_protocol_fee: ${side} ${v} is not a u16`);
    if (v > MAX_PROTOCOL_FEE) throw new Error(`set_protocol_fee: ${side} ${v} pips is above MAX_PROTOCOL_FEE ${MAX_PROTOCOL_FEE} (0.1 %) — ProtocolFeeTooLarge`);
  }
  return packProtocolFee(zeroForOne, oneForZero);
}
