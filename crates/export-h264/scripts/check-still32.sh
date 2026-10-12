#!/bin/sh
# Check the 32x32 still against the source. The crate does not call this script.
# This is not a decode. ffmpeg is not used.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin still32
mkdir -p target/check
./target/release/still32 target/check/still32.yuv target/check/still32-src.yuv | tee target/check/still32-time.txt
python3 - <<'PY'
src = open("target/check/still32-src.yuv", "rb").read()
still = open("target/check/still32.yuv", "rb").read()
frame = 32 * 32 + 16 * 16 * 2
if len(src) != frame or len(still) != frame:
    raise SystemExit(f"size mismatch src={len(src)} still={len(still)}")
mae = sum(abs(p - q) for p, q in zip(src, still)) / frame
print(f"raw_bytes={len(still)}")
print(f"mae={mae:.4f}")
print(f"matches_measured={abs(mae - 2.4974) < 0.00005}")
PY
