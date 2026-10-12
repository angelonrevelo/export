#!/bin/sh
# Decode four 320x240 pictures. Each P-frame uses the left/right motion split.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin seqsplit320
mkdir -p target/check
./target/release/seqsplit320 target/check/seqsplit320.264 target/check/seqsplit320-src.yuv target/check/seqsplit320-rec.yuv | tee target/check/seqsplit320-time.txt
set +e
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/seqsplit320.264 -f rawvideo -pix_fmt yuv420p target/check/seqsplit320-dec.yuv
status=$?
set -e
if [ "$status" -ne 0 ]; then
    echo "ffmpeg_decoded=no"
    exit 0
fi
python3 - <<'PY'
frame = 320 * 240 + 160 * 120 * 2
src = open("target/check/seqsplit320-src.yuv", "rb").read()
dec = open("target/check/seqsplit320-dec.yuv", "rb").read()
rec = open("target/check/seqsplit320-rec.yuv", "rb").read()
got = len(dec) // frame if frame else 0
if len(dec) != frame * 4 or len(rec) != frame * 4:
    raise SystemExit(f"ffmpeg_decoded=no pictures={got} rec={len(rec)}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

delta = mae(rec, dec)
print("ffmpeg_decoded=yes")
print("pictures=4")
print(f"recon_match={delta == 0.0}")
print(f"recon_delta={delta:.4f}")
print(f"mae={mae(src, dec):.4f}")
for i in range(4):
    a = i * frame
    b = a + frame
    print(f"mae_{i}={mae(src[a:b], dec[a:b]):.4f}")
PY
