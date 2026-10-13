// The wasm-bindgen exports the quoter wrapper calls (`QuoteFns` in `src/quote/quoter.ts`). One list, read by
// `sync-wasm.mjs` (fails on a glue missing any) and by `test/wasm.test.ts` (fails when `QuoteFns` and this list drift).
export const REQUIRED_EXPORTS = [
  'quote_swap',
  'amounts_for_liquidity',
  'quote_liquidity',
  'fees_owed',
  'sqrt_price_at_tick',
  'tick_at_sqrt_price',
  'tick_at_price_ui',
  'price_ui_at_tick',
  'version',
];

/**
 * What makes a glue a `wasm-bindgen --target web` module: a DEFAULT export (the async init `initQuote` calls) and
 * `initSync`. wasm-bindgen 0.2.128 emits one statement, `export { initSync, __wbg_init as default };`; each entry accepts
 * either spelling.
 */
export const WEB_TARGET_MARKERS = [
  ['a default export (the async init)', /export\s+default\s+\w+|export\s*\{[^}]*\bas\s+default\b[^}]*\}/],
  ['initSync', /export\s+(?:function\s+)?initSync\b|export\s*\{[^}]*\binitSync\b[^}]*\}/],
];
