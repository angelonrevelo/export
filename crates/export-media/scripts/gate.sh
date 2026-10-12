#!/bin/sh
# Local gate for this crate. The library does not shell out.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"
cargo clippy --all-targets -- -D warnings
cargo test
cargo check --target wasm32-unknown-unknown --lib
echo "gate ok"
