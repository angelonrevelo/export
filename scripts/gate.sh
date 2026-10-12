#!/usr/bin/env bash
# Local gate for the Rust Export crates. Clippy denies warnings.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== clippy"
cargo clippy --workspace --all-targets -- -D warnings

echo "== test"
cargo test --workspace

echo "== wasm lib"
cargo build --release -p export-kernel --lib --target wasm32-unknown-unknown
cargo build --release -p export-subtitle --lib --target wasm32-unknown-unknown

echo "OVERALL: PASS"
