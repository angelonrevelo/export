#!/bin/sh
# Decode the 1280x720 I-frame plus one split P-frame.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin splitframe720
mkdir -p target/check
./target/release/splitframe720 target/check/split720.264 target/check/split720-src.yuv target/check/split720-rec.yuv | tee target/check/split720-time.txt
set +e
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/split720.264 -f rawvideo -pix_fmt yuv420p target/check/split720-dec.yuv
status=$?
set -e
if [ "$status" -ne 0 ]; then
    echo "ffmpeg_decoded=no"
    exit 0
fi
python3 - <<'PY'
frame = 1280 * 720 + 640 * 360 * 2
src = open("target/check/split720-src.yuv", "rb").read()
dec = open("target/check/split720-dec.yuv", "rb").read()
rec = open("target/check/split720-rec.yuv", "rb").read()
got = len(dec) // frame if frame else 0
if len(dec) != frame * 2 or len(rec) != frame * 2:
    raise SystemExit(f"ffmpeg_decoded=no pictures={got} rec={len(rec)}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

delta = mae(rec, dec)
print("ffmpeg_decoded=yes")
print("pictures=2")
print(f"recon_match={delta == 0.0}")
print(f"recon_delta={delta:.4f}")
print(f"mae={mae(src, dec):.4f}")
print(f"mae_i={mae(src[:frame], dec[:frame]):.4f}")
print(f"mae_p={mae(src[frame:], dec[frame:]):.4f}")
PY
