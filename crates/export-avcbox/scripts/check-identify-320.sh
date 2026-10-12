#!/bin/sh
# Identify the boxed 320x240 I-frame. The crate does not call this script.
# ffmpeg runs only to name the codec and the picture size. It does not encode.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin avcbox320
mkdir -p target/check
./target/release/avcbox320 target/check/frame320.mp4 | tee target/check/time320.txt
log=target/check/ffmpeg320.txt
ffmpeg -hide_banner -i target/check/frame320.mp4 -f null - >"$log" 2>&1 || true
cat "$log"
grep -E 'Video: h264' "$log" >/dev/null
grep -E '320x240' "$log" >/dev/null
