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
cargo run --release --example bench | tee bench/runs/release.txt
./scripts/ffmpeg-decode.sh | tee bench/runs/ffmpeg.txt
