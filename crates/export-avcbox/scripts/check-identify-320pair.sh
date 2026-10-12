#!/bin/sh
# Identify the boxed 320x240 I-frame and P-frame. The crate does not call this script.
# ffmpeg runs only to name the codec, the picture size, and the decoded frame count.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin avcbox320pair
mkdir -p target/check
./target/release/avcbox320pair target/check/pair320.mp4 | tee target/check/time320pair.txt
log=target/check/ffmpeg320pair.txt
ffmpeg -hide_banner -i target/check/pair320.mp4 -f null - >"$log" 2>&1 || true
cat "$log"
grep -E 'Video: h264' "$log" >/dev/null
grep -E '320x240' "$log" >/dev/null
frames=$(grep -oE 'frame=[[:space:]]*[0-9]+' "$log" | tail -n 1 | grep -oE '[0-9]+' || true)
echo "ffmpeg_frames=${frames:-unknown}"
