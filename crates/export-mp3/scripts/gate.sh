#!/usr/bin/env bash
# Local gate. GitHub Actions may be off; this is the check.
# clippy, tests, then the wasm32-unknown-unknown library.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release --target wasm32-unknown-unknown

mkdir -p bench/runs
# bench/runs/release.txt keeps the recorded silence (p50 948 ns), truncated
# (p50 33 ns), older pulse (p50 515 µs), and 16-line (p50 559 µs) rows.
# Do not overwrite that file.
cargo run --release --example bench | tee bench/runs/latest.txt
./scripts/ffmpeg-decode.sh | tee bench/runs/ffmpeg.txt
