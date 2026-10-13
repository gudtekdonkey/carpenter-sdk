#!/usr/bin/env bash
# Build and test carpenter-amm with the `upstream-variant` feature against a local
# jupiter-amm-interface checkout carrying swap-variant.patch.
#
#   upstream/check.sh [interface-checkout]
#
# With no argument the pinned commit is cloned into a temporary directory and the patch applied.
# The interface is substituted with a cargo `--config` [patch] for this run only: Cargo.toml is not
# edited, Cargo.lock is restored afterwards, and the build uses its own target directory.
set -euo pipefail

REV=95bd18485e580625a39eb92a0c0ad2061579daad
URL=https://github.com/jup-ag/jupiter-amm-interface
here="$(cd "$(dirname "$0")" && pwd)"
crate="$(dirname "$here")"

if [[ $# -ge 1 ]]; then
  iface="$(cd "$1" && pwd)"
else
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' EXIT
  git clone -q "$URL" "$tmp/jupiter-amm-interface"
  git -C "$tmp/jupiter-amm-interface" checkout -q "$REV"
  git -C "$tmp/jupiter-amm-interface" apply "$here/swap-variant.patch"
  iface="$tmp/jupiter-amm-interface"
fi
grep -q 'Carpenter {' "$iface/interface/src/swap.rs" || { echo "$iface does not carry swap-variant.patch" >&2; exit 1; }

cd "$crate"
mkdir -p target
cp Cargo.lock target/.Cargo.lock.upstream-variant.bak
restore() { cp "$crate/target/.Cargo.lock.upstream-variant.bak" "$crate/Cargo.lock"; }
trap 'restore; [[ -n "${tmp:-}" ]] && rm -rf "$tmp"' EXIT

# Both the interface and the test kit come from the same git source; the kit's path dependency on
# `../interface` is replaced along with it. `tests/conformance.rs` reads the same patches from
# CARPENTER_AMM_CARGO_CONFIG for its `cargo metadata` check.
c1="patch.'$URL'.jupiter-amm-interface.path='$iface/interface'"
c2="patch.'$URL'.jupiter-amm-test-kit.path='$iface/test-kit'"
export CARPENTER_AMM_CARGO_CONFIG="$c1
$c2"
CARGO_TARGET_DIR="$crate/target/upstream-variant" cargo test --features upstream-variant \
  --config "$c1" --config "$c2" "${@:2}"
