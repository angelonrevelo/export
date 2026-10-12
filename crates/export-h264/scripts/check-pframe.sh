#!/bin/sh
# Decode the I-frame plus one P-frame with ffmpeg and print mean absolute error.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin ipframe
mkdir -p target/check
./target/release/ipframe target/check/ip.264 target/check/ip-src.yuv | tee target/check/ip-time.txt
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/ip.264 -f rawvideo -pix_fmt yuv420p target/check/ip-dec.yuv
python3 - <<'PY'
frame = 32 * 32 + 16 * 16 * 2
a = open("target/check/ip-src.yuv", "rb").read()
b = open("target/check/ip-dec.yuv", "rb").read()
if len(a) != len(b) or len(a) != frame * 2:
    raise SystemExit(f"size mismatch src={len(a)} dec={len(b)}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

print(f"mae={mae(a, b):.4f}")
print(f"mae_i={mae(a[:frame], b[:frame]):.4f}")
print(f"mae_p={mae(a[frame:], b[frame:]):.4f}")
PY
