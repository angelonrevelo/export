#!/bin/sh
# Identify the boxed 32x32 I-frame. The crate does not call this script.
# ffmpeg runs only to name the codec. It does not encode.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin avcbox
mkdir -p target/check
./target/release/avcbox target/check/frame.mp4 | tee target/check/time.txt
log=target/check/ffmpeg.txt
ffmpeg -hide_banner -i target/check/frame.mp4 -f null - >"$log" 2>&1 || true
cat "$log"
grep -E 'Video: h264' "$log" >/dev/null
