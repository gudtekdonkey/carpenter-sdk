#!/usr/bin/env bash
# Build `carpenter-quote-wasm` for wasm32-unknown-unknown and bind it with `wasm-bindgen --target web`.
#
#   bash wasm/build.sh                 # → dist/wasm/carpenter_quote{_bg.wasm,.js,.d.ts,_bg.wasm.d.ts}
#   CARPENTER_WASM_OUT=<dir> bash wasm/build.sh
#
# Then `npm run sync-wasm` (packages/sdk) copies the output into the package.
#
# The build runs on a COPY of the sources in a temporary directory, so the output depends on the sources only:
# - the wasm crate's `///` and `//!` lines are deleted in the copy (wasm-bindgen copies doc comments into the JS glue);
# - paths are remapped: the copy's root to `.`, `$CARGO_HOME` to `/cargo`, `$RUSTUP_HOME` to `/rustup`;
# - the result is refused if it is 400,000 bytes or more, or if the wasm or its glue names a local path.
# Needs rustc 1.98.1 with the wasm32-unknown-unknown target and wasm-bindgen-cli 0.2.128 (the crate pins the
# library to the same version).
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
root="$(cd "$here/.." && pwd)"
out="${CARPENTER_WASM_OUT:-$root/dist/wasm}"
max=400000
want_cli="0.2.128"

cli="$(wasm-bindgen --version | awk '{print $2}')"
[[ "$cli" == "$want_cli" ]] || { echo "build: wasm-bindgen-cli is $cli, the crate pins $want_cli" >&2; exit 3; }

cargo_home="${CARGO_HOME:-$HOME/.cargo}"
rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"
# A FIXED build directory: crate hashes (and so the code layout) depend on the source path, so the same path gives the
# same bytes on any machine.
work="${CARPENTER_WASM_WORK:-/tmp/carpenter-quote-wasm-build}"
rm -rf "$work"
mkdir -p "$work"
trap 'rm -rf "$work"' EXIT
src="$work/src"
mkdir -p "$src/wasm"
cp "$root/Cargo.toml" "$root/rust-toolchain.toml" "$src/"
cp -r "$root/crates" "$src/crates"
cp -r "$here/carpenter-quote-wasm" "$src/wasm/carpenter-quote-wasm"
rm -rf "$src/wasm/carpenter-quote-wasm/target"
find "$src/wasm/carpenter-quote-wasm/src" -name '*.rs' -exec sed -i -E '/^[[:space:]]*\/\/[\/!]/d' {} +

export CARGO_TARGET_DIR="$work/target"
export RUSTFLAGS="--remap-path-prefix=$src=. --remap-path-prefix=$cargo_home=/cargo --remap-path-prefix=$rustup_home=/rustup"
(cd "$src/wasm/carpenter-quote-wasm" && cargo build --release --locked --target wasm32-unknown-unknown)
artifact="$work/target/wasm32-unknown-unknown/release/carpenter_quote_wasm.wasm"
[[ -f "$artifact" ]] || { echo "build: expected $artifact" >&2; exit 6; }

mkdir -p "$out"
wasm-bindgen --target web --out-dir "$out" --out-name carpenter_quote "$artifact"

wasm="$out/carpenter_quote_bg.wasm"
bytes="$(stat -c %s "$wasm")"
(( bytes < max )) || { echo "build: $wasm is $bytes B, limit $max B" >&2; exit 7; }
for f in "$wasm" "$out/carpenter_quote.js" "$out/carpenter_quote.d.ts"; do
  if grep -a -o -E '/(tmp|root|home|build)/[^"[:space:]]*|[Cc]laude' "$f" | head -3 | grep -q .; then
    echo "build: $f names a local path or a forbidden word:" >&2
    grep -a -o -E '/(tmp|root|home|build)/[^"[:space:]]*|[Cc]laude' "$f" | head -3 >&2
    exit 8
  fi
done
echo "build: $wasm  $bytes B  sha256 $(sha256sum "$wasm" | awk '{print $1}')  (wasm-bindgen $cli)"
