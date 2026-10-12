#!/bin/sh
# Decode the 160x160 I-frame with ffmpeg and print mean absolute error.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin iframe160
mkdir -p target/check
./target/release/iframe160 target/check/frame160.264 target/check/frame160-src.yuv target/check/frame160-rec.yuv | tee target/check/frame160-time.txt
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/frame160.264 -f rawvideo -pix_fmt yuv420p target/check/frame160-dec.yuv
python3 - <<'PY'
src = open("target/check/frame160-src.yuv", "rb").read()
dec = open("target/check/frame160-dec.yuv", "rb").read()
rec = open("target/check/frame160-rec.yuv", "rb").read()
expect = 160 * 160 + 80 * 80 * 2
if len(src) != len(dec) or len(src) != len(rec) or len(src) != expect:
    raise SystemExit(f"size mismatch src={len(src)} dec={len(dec)} rec={len(rec)}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

delta = mae(rec, dec)
if delta != 0.0:
    raise SystemExit(f"ffmpeg does not match the encoder reconstruction ({delta:.4f})")
y = 160 * 160
c = 80 * 80
print(f"mae={mae(src, dec):.4f}")
print(f"mae_y={mae(src[:y], dec[:y]):.4f}")
print(f"mae_u={mae(src[y:y + c], dec[y:y + c]):.4f}")
print(f"mae_v={mae(src[y + c:], dec[y + c:]):.4f}")
print(f"recon_delta={delta:.4f}")
PY
