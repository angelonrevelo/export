#!/usr/bin/env bash
# Host gate for this standalone crate. The repository root is not a Cargo
# workspace; this script is the local check.
set -euo pipefail
cd "$(dirname "$0")"

echo "== clippy"
cargo clippy --all-targets -- -D warnings

echo "== test"
cargo test

echo "== wasm32-unknown-unknown lib (parser; socket fetch is cfg-gated off)"
cargo build --target wasm32-unknown-unknown --lib

echo "OVERALL: PASS"
