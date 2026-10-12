#!/bin/sh
# Identify the boxed eight 1280x720 pictures. The crate does not call this script.
# ffmpeg runs only to name the codec, the picture size, and the decoded frame count.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin avcbox720eight
mkdir -p target/check
./target/release/avcbox720eight target/check/eight720.mp4 | tee target/check/time720eight.txt
log=target/check/ffmpeg720eight.txt
ffmpeg -hide_banner -i target/check/eight720.mp4 -f null - >"$log" 2>&1 || true
cat "$log"
grep -E 'Video: h264' "$log" >/dev/null
grep -E '1280x720' "$log" >/dev/null
frames=$(grep -oE 'frame=[[:space:]]*[0-9]+' "$log" | tail -n 1 | grep -oE '[0-9]+' || true)
echo "ffmpeg_frames=${frames:-unknown}"
test "${frames:-0}" = 8
