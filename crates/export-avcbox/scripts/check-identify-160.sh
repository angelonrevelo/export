#!/bin/sh
# Identify the boxed 160x160 I-frame. The crate does not call this script.
# ffmpeg runs only to name the codec and the picture size. It does not encode.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin avcbox160
mkdir -p target/check
./target/release/avcbox160 target/check/frame160.mp4 | tee target/check/time160.txt
log=target/check/ffmpeg160.txt
ffmpeg -hide_banner -i target/check/frame160.mp4 -f null - >"$log" 2>&1 || true
cat "$log"
grep -E 'Video: h264' "$log" >/dev/null
grep -E '160x160' "$log" >/dev/null
