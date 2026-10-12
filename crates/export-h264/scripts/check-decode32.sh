#!/bin/sh
# Read the 272-byte 32x32 I-frame and compare pixels to the encoder reconstruction.
# The crate does not call this script. ffmpeg is not used.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin decode32
mkdir -p target/check
./target/release/decode32 target/check/decode32.yuv target/check/decode32-src.yuv target/check/decode32-rec.yuv | tee target/check/decode32-time.txt
python3 - <<'PY'
src = open("target/check/decode32-src.yuv", "rb").read()
dec = open("target/check/decode32.yuv", "rb").read()
rec = open("target/check/decode32-rec.yuv", "rb").read()
frame = 32 * 32 + 16 * 16 * 2
if len(src) != frame or len(dec) != frame or len(rec) != frame:
    raise SystemExit(f"size mismatch src={len(src)} dec={len(dec)} rec={len(rec)}")
if dec != rec:
    raise SystemExit("recon_match=False")
mae = sum(abs(p - q) for p, q in zip(src, dec)) / frame
print("recon_match=True")
print(f"mae={mae:.4f}")
print(f"matches_measured={abs(mae - 2.4974) < 0.00005}")
PY
