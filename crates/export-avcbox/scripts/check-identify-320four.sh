#!/bin/sh
# Identify the boxed four 320x240 pictures. The crate does not call this script.
# ffmpeg runs only to name the codec, the picture size, and the decoded frame count.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin avcbox320four
mkdir -p target/check
./target/release/avcbox320four target/check/four320.mp4 | tee target/check/time320four.txt
log=target/check/ffmpeg320four.txt
ffmpeg -hide_banner -i target/check/four320.mp4 -f null - >"$log" 2>&1 || true
cat "$log"
grep -E 'Video: h264' "$log" >/dev/null
grep -E '320x240' "$log" >/dev/null
frames=$(grep -oE 'frame=[[:space:]]*[0-9]+' "$log" | tail -n 1 | grep -oE '[0-9]+' || true)
echo "ffmpeg_frames=${frames:-unknown}"
