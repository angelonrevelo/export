#!/bin/sh
# Decode the 32x32 Annex B fixture with ffmpeg and print mean absolute error
# against the source planes. The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin iframe
mkdir -p target/check
./target/release/iframe target/check/frame.264 target/check/src.yuv | tee target/check/time.txt
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/frame.264 -f rawvideo -pix_fmt yuv420p target/check/dec.yuv
python3 - <<'PY'
a = open("target/check/src.yuv", "rb").read()
b = open("target/check/dec.yuv", "rb").read()
if len(a) != len(b) or len(a) != 32 * 32 + 16 * 16 * 2:
    raise SystemExit(f"size mismatch src={len(a)} dec={len(b)}")
s = sum(abs(x - y) for x, y in zip(a, b))
print(f"mae={s / len(a):.4f}")
PY
