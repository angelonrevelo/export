#!/bin/sh
# Decode the 320x240 I-frame plus one P-frame.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin ipframe320
mkdir -p target/check
./target/release/ipframe320 target/check/ip320.264 target/check/ip320-src.yuv target/check/ip320-rec.yuv | tee target/check/ip320-time.txt
set +e
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/ip320.264 -f rawvideo -pix_fmt yuv420p target/check/ip320-dec.yuv
status=$?
set -e
if [ "$status" -ne 0 ]; then
    echo "ffmpeg_decoded=no"
    exit 0
fi
python3 - <<'PY'
frame = 320 * 240 + 160 * 120 * 2
src = open("target/check/ip320-src.yuv", "rb").read()
dec = open("target/check/ip320-dec.yuv", "rb").read()
rec = open("target/check/ip320-rec.yuv", "rb").read()
got = len(dec) // frame if frame else 0
if len(dec) != frame * 2:
    raise SystemExit(f"ffmpeg_decoded=no pictures={got}")
print("ffmpeg_decoded=yes")
print("pictures=2")
print(f"recon_match={rec == dec}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

print(f"mae={mae(src, dec):.4f}")
print(f"mae_i={mae(src[:frame], dec[:frame]):.4f}")
print(f"mae_p={mae(src[frame:], dec[frame:]):.4f}")
print(f"recon_delta={mae(rec, dec):.4f}")
PY
