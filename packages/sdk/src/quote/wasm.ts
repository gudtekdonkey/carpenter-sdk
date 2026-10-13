/**
 * `initQuote(bytes)` over `wasm/carpenter_quote_bg.wasm`.
 *
 * Load the wasm bytes yourself and hand them over, never through a bundler import:
 *   const q = await initQuote(await (await fetch(url)).arrayBuffer());
 *   const q = await initQuote(readFileSync(require.resolve('@carpenter/sdk/carpenter_quote_bg.wasm')));
 * `initQuote` passes the bytes to wasm-bindgen's `--target web` glue (`wasm/carpenter_quote.js`); the glue's own
 * URL-fetching path is never taken because the bytes are always given. `WASM_SHA256` is the hash of the shipped wasm.
 * The glue is a module-level singleton: the first `initQuote` wins, and a second call returns a quoter over the FIRST
 * instance. One wasm per page.
 */

import * as glue from '../../wasm/carpenter_quote.js';
import init from '../../wasm/carpenter_quote.js';

import { WASM_BYTES, WASM_SHA256 } from '../generated/wasm.ts';
import { createQuoter, type QuoteFns, type Quoter } from './quoter.ts';

export { WASM_BYTES, WASM_SHA256 };

/** The wasm-bindgen exports, held to `QuoteFns` at type-check time (a renamed export fails `tsc`). */
const fns: QuoteFns = glue;

let ready: Promise<Quoter> | null = null;

/** Instantiate the quoter from the wasm bytes (an `ArrayBuffer`, a `Uint8Array`, or a compiled `WebAssembly.Module`). Idempotent. */
export function initQuote(source: ArrayBuffer | Uint8Array | WebAssembly.Module): Promise<Quoter> {
  if (!ready) {
    ready = (async () => {
      await init({ module_or_path: source as BufferSource | WebAssembly.Module });
      return createQuoter(fns);
    })().catch((e: unknown) => {
      ready = null;
      throw e;
    });
  }
  return ready;
}
