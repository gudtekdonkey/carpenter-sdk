# Changelog

Notable changes to the package and crates in this repository. Versions follow [Semantic Versioning](https://semver.org);
while the version is `0.x`, a minor release may change the API.

## Unreleased

### Changed

- The package is published as `@floorlaunchpad/carpenter-sdk` (the `@carpenter` npm scope belongs to another account).

### Added

- `integrations/defillama`: DefiLlama TVL, volume and fees adapters.
- Continuous integration.

### Fixed

- The npm package now ships `NOTICE` and `THIRD_PARTY.md` beside the quoter wasm it bundles.

## 0.1.1

### Fixed

- `buildInitializePool` (and so `planCreatePool`) set a 120,000 CU limit, below what `initialize_pool` uses (106k – 129k
  measured on devnet), so creating a pool failed with `ComputationalBudgetExceeded`. `CU_LIMITS.initializePool` is now
  195,000. The Rust crates are unchanged from 0.1.0.

## 0.1.0

First release: `@floorlaunchpad/carpenter-sdk`; the `carpenter-types`, `carpenter-clmm-math` and `carpenter-quote` crates; the quoter
wasm; `carpenter-amm`.
