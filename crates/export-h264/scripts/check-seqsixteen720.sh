#!/bin/sh
# Decode sixteen 1280x720 pictures. Each P-frame uses the left/right motion split.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin seqsixteen720
mkdir -p target/check
./target/release/seqsixteen720 target/check/seqsixteen720.264 target/check/seqsixteen720-src.yuv target/check/seqsixteen720-rec.yuv | tee target/check/seqsixteen720-time.txt
set +e
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/seqsixteen720.264 -f rawvideo -pix_fmt yuv420p target/check/seqsixteen720-dec.yuv
status=$?
set -e
if [ "$status" -ne 0 ]; then
    echo "ffmpeg_decoded=no"
    exit 0
fi
python3 - <<'PY'
frame = 1280 * 720 + 640 * 360 * 2
src = open("target/check/seqsixteen720-src.yuv", "rb").read()
dec = open("target/check/seqsixteen720-dec.yuv", "rb").read()
rec = open("target/check/seqsixteen720-rec.yuv", "rb").read()
got = len(dec) // frame if frame else 0
if len(dec) != frame * 16 or len(rec) != frame * 16:
    raise SystemExit(f"ffmpeg_decoded=no pictures={got} rec={len(rec)}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

delta = mae(rec, dec)
print("ffmpeg_decoded=yes")
print("pictures=16")
print(f"recon_match={delta == 0.0}")
print(f"recon_delta={delta:.4f}")
print(f"mae={mae(src, dec):.4f}")
print(f"mae_i={mae(src[:frame], dec[:frame]):.4f}")
print(f"mae_p={mae(src[frame:], dec[frame:]):.4f}")
PY
