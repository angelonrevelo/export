#!/bin/sh
# Local gate for this standalone crate. The library does not call ffmpeg.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cd "$root"

cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --lib --target wasm32-unknown-unknown --release
