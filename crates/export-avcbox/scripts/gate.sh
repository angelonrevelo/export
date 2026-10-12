#!/bin/sh
# Host gate for this crate. Clippy, tests, then the wasm32 library.
# ffmpeg is not part of this gate. scripts/check-identify.sh may run it.
set -eu
cd "$(dirname "$0")/.."
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --target wasm32-unknown-unknown --release --lib
