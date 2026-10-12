#!/bin/sh
# Read the shifted 32x32 pair, quarter-pel (-16, 0). The crate does not call this script.
# ffmpeg is not used.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin decodeshiftright32
mkdir -p target/check
./target/release/decodeshiftright32 target/check/decodeshiftright32.yuv target/check/decodeshiftright32-rec.yuv | tee target/check/decodeshiftright32-time.txt
python3 - <<'PY'
dec = open("target/check/decodeshiftright32.yuv", "rb").read()
rec = open("target/check/decodeshiftright32-rec.yuv", "rb").read()
frame = 32 * 32 + 16 * 16 * 2
if len(dec) != frame * 2 or dec != rec:
    raise SystemExit(f"recon_match=False dec={len(dec)} rec={len(rec)}")
print("recon_match=True")
print("pictures=2")
PY
