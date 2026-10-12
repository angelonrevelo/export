#!/bin/sh
# Identify the boxed 1280x720 I-frame. The crate does not call this script.
# ffmpeg runs only to name the codec and the picture size. It does not encode.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin avcbox720
mkdir -p target/check
./target/release/avcbox720 target/check/frame720.mp4 | tee target/check/time720.txt
log=target/check/ffmpeg720.txt
ffmpeg -hide_banner -i target/check/frame720.mp4 -f null - >"$log" 2>&1 || true
cat "$log"
grep -E 'Video: h264' "$log" >/dev/null
grep -E '1280x720' "$log" >/dev/null
