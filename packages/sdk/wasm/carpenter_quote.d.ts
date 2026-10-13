/* tslint:disable */
/* eslint-disable */

export function amounts_for_liquidity(sqrt_price_x64: string, tick_current: number, tick_lower: number, tick_upper: number, liquidity_delta: string): string;

export function fees_owed(pool_key: Uint8Array, pool: Uint8Array, position: Uint8Array, lower_array: Uint8Array, upper_array: Uint8Array): string;

export function price_ui_at_tick(tick: number, decimals0: number, decimals1: number): number;

export function quote_liquidity(sqrt_price_x64: string, tick_lower: number, tick_upper: number, amount0_max: string, amount1_max: string): string;

export function quote_swap(pool_key: Uint8Array, pool: Uint8Array, bitmap: Uint8Array, tick_arrays: Uint8Array, zero_for_one: boolean, amount_specified: string, sqrt_price_limit_x64: string): string;

export function sqrt_price_at_tick(tick: number): string;

export function tick_at_price_ui(price_ui: number, decimals0: number, decimals1: number): string;

export function tick_at_sqrt_price(sqrt_price_x64: string): string;

export function version(): string;

export type InitInput = RequestInfo | URL | Response | BufferSource | WebAssembly.Module;

export interface InitOutput {
    readonly memory: WebAssembly.Memory;
    readonly amounts_for_liquidity: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number) => void;
    readonly fees_owed: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number) => void;
    readonly price_ui_at_tick: (a: number, b: number, c: number) => number;
    readonly quote_liquidity: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number) => void;
    readonly quote_swap: (a: number, b: number, c: number, d: number, e: number, f: number, g: number, h: number, i: number, j: number, k: number, l: number, m: number, n: number) => void;
    readonly sqrt_price_at_tick: (a: number, b: number) => void;
    readonly tick_at_price_ui: (a: number, b: number, c: number, d: number) => void;
    readonly tick_at_sqrt_price: (a: number, b: number, c: number) => void;
    readonly version: (a: number) => void;
    readonly __wbindgen_add_to_stack_pointer: (a: number) => number;
    readonly __wbindgen_export: (a: number, b: number) => number;
    readonly __wbindgen_export2: (a: number, b: number, c: number, d: number) => number;
    readonly __wbindgen_export3: (a: number, b: number, c: number) => void;
}

export type SyncInitInput = BufferSource | WebAssembly.Module;

/**
 * Instantiates the given `module`, which can either be bytes or
 * a precompiled `WebAssembly.Module`.
 *
 * @param {{ module: SyncInitInput }} module - Passing `SyncInitInput` directly is deprecated.
 *
 * @returns {InitOutput}
 */
export function initSync(module: { module: SyncInitInput } | SyncInitInput): InitOutput;

/**
 * If `module_or_path` is {RequestInfo} or {URL}, makes a request and
 * for everything else, calls `WebAssembly.instantiate` directly.
 *
 * @param {{ module_or_path: InitInput | Promise<InitInput> }} module_or_path - Passing `InitInput` directly is deprecated.
 *
 * @returns {Promise<InitOutput>}
 */
export default function __wbg_init (module_or_path?: { module_or_path: InitInput | Promise<InitInput> } | InitInput | Promise<InitInput>): Promise<InitOutput>;
