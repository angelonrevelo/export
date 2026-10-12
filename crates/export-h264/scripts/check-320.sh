#!/bin/sh
# Decode one 320x240 I-frame with ffmpeg.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin iframe320
mkdir -p target/check
./target/release/iframe320 target/check/frame320.264 target/check/frame320-src.yuv target/check/frame320-rec.yuv | tee target/check/frame320-time.txt
set +e
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/frame320.264 -f rawvideo -pix_fmt yuv420p target/check/frame320-dec.yuv
status=$?
set -e
if [ "$status" -ne 0 ]; then
    echo "ffmpeg_decoded=no"
    exit 0
fi
python3 - <<'PY'
expect = 320 * 240 + 160 * 120 * 2
src = open("target/check/frame320-src.yuv", "rb").read()
dec = open("target/check/frame320-dec.yuv", "rb").read()
rec = open("target/check/frame320-rec.yuv", "rb").read()
if len(dec) != expect:
    raise SystemExit(f"ffmpeg_decoded=no size={len(dec)}")
print("ffmpeg_decoded=yes")
print(f"recon_match={rec == dec}")
PY
