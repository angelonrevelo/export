#!/bin/sh
# Decode the 160x160 I-frame plus one zero-motion P-frame.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin ipframe160
mkdir -p target/check
./target/release/ipframe160 target/check/ip160.264 target/check/ip160-src.yuv target/check/ip160-rec.yuv | tee target/check/ip160-time.txt
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/ip160.264 -f rawvideo -pix_fmt yuv420p target/check/ip160-dec.yuv
python3 - <<'PY'
frame = 160 * 160 + 80 * 80 * 2
src = open("target/check/ip160-src.yuv", "rb").read()
dec = open("target/check/ip160-dec.yuv", "rb").read()
rec = open("target/check/ip160-rec.yuv", "rb").read()
if len(src) != len(dec) or len(src) != len(rec) or len(src) != frame * 2:
    raise SystemExit(f"size mismatch src={len(src)} dec={len(dec)} rec={len(rec)}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

delta = mae(rec, dec)
if delta != 0.0:
    raise SystemExit(f"ffmpeg does not match the encoder reconstruction ({delta:.4f})")
print(f"mae={mae(src, dec):.4f}")
print(f"mae_i={mae(src[:frame], dec[:frame]):.4f}")
print(f"mae_p={mae(src[frame:], dec[frame:]):.4f}")
print(f"recon_delta={delta:.4f}")
PY
