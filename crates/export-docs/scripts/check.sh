#!/bin/sh
# Build, test, and time export-docs from this crate. Not a workspace command.
set -eu
cd "$(dirname "$0")/.."
cargo clippy --offline --all-targets -- -D warnings
cargo test --offline
cargo build --offline --release
cargo run --offline --release --bin export-docs-bench
python3 host/oracle.py
