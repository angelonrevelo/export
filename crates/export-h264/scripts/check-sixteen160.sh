#!/bin/sh
# Decode sixteen 160x160 pictures with ffmpeg.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin seqframe16
mkdir -p target/check
./target/release/seqframe16 target/check/sixteen.264 target/check/sixteen-src.yuv target/check/sixteen-rec.yuv | tee target/check/sixteen-time.txt
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/sixteen.264 -f rawvideo -pix_fmt yuv420p target/check/sixteen-dec.yuv
python3 - <<'PY'
frame = 160 * 160 + 80 * 80 * 2
src = open("target/check/sixteen-src.yuv", "rb").read()
dec = open("target/check/sixteen-dec.yuv", "rb").read()
rec = open("target/check/sixteen-rec.yuv", "rb").read()
if len(dec) != frame * 16:
    raise SystemExit(f"ffmpeg decoded {len(dec) // frame} pictures, expected 16")
if len(src) != len(dec) or len(src) != len(rec):
    raise SystemExit(f"size mismatch src={len(src)} dec={len(dec)} rec={len(rec)}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

delta = mae(rec, dec)
if delta != 0.0:
    raise SystemExit(f"ffmpeg does not match the encoder reconstruction ({delta:.4f})")
print(f"pictures=16")
print(f"mae={mae(src, dec):.4f}")
print(f"mae_i={mae(src[:frame], dec[:frame]):.4f}")
print(f"recon_delta={delta:.4f}")
PY
