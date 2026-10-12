#!/bin/sh
# Decode eight 32x32 pictures with ffmpeg and print mean absolute error.
# The crate does not call this script.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin seqframe
mkdir -p target/check
./target/release/seqframe target/check/seq.264 target/check/seq-src.yuv target/check/seq-rec.yuv | tee target/check/seq-time.txt
ffmpeg -y -xerror -hide_banner -loglevel error -f h264 -i target/check/seq.264 -f rawvideo -pix_fmt yuv420p target/check/seq-dec.yuv
python3 - <<'PY'
frame = 32 * 32 + 16 * 16 * 2
src = open("target/check/seq-src.yuv", "rb").read()
dec = open("target/check/seq-dec.yuv", "rb").read()
rec = open("target/check/seq-rec.yuv", "rb").read()
if len(src) != len(dec) or len(src) != len(rec) or len(src) != frame * 8:
    raise SystemExit(f"size mismatch src={len(src)} dec={len(dec)} rec={len(rec)}")

def mae(x, y):
    return sum(abs(p - q) for p, q in zip(x, y)) / len(x)

delta = mae(rec, dec)
if delta != 0.0:
    raise SystemExit(f"ffmpeg does not match the encoder reconstruction ({delta:.4f})")
print(f"mae={mae(src, dec):.4f}")
print(f"mae_i={mae(src[:frame], dec[:frame]):.4f}")
for i in range(7):
    a = frame * (i + 1)
    b = a + frame
    print(f"mae_p{i + 1}={mae(src[a:b], dec[a:b]):.4f}")
print(f"recon_delta={delta:.4f}")
PY
