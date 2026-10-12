#!/usr/bin/env bash
# Local gate for the standalone export-opus crate.
#
# Clippy, tests, the wasm32 lib, then the host ffmpeg decode check.
# The library does not call ffmpeg. The last step is a host script.
#
#   bash scripts/gate.sh

set -euo pipefail
cd "$(dirname "$0")/.."

step() {
  echo
  echo "== $*"
}

step "clippy (0 warnings is the gate)"
cargo clippy --workspace --all-targets -- -D warnings

step "tests"
cargo test --workspace

step "wasm32-unknown-unknown lib"
cargo build --release --workspace --target wasm32-unknown-unknown --lib

step "ffmpeg decodes the packet this crate wrote"
python3 scripts/verify-ffmpeg.py

echo
echo "OVERALL: PASS"
