#!/bin/sh
# Identify the boxed four 1280x720 pictures. The crate does not call this script.
# ffmpeg runs only to name the codec, the picture size, and the decoded frame count.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --bin avcbox720four
mkdir -p target/check
./target/release/avcbox720four target/check/four720.mp4 | tee target/check/time720four.txt
log=target/check/ffmpeg720four.txt
ffmpeg -hide_banner -i target/check/four720.mp4 -f null - >"$log" 2>&1 || true
cat "$log"
grep -E 'Video: h264' "$log" >/dev/null
grep -E '1280x720' "$log" >/dev/null
frames=$(grep -oE 'frame=[[:space:]]*[0-9]+' "$log" | tail -n 1 | grep -oE '[0-9]+' || true)
echo "ffmpeg_frames=${frames:-unknown}"
test "${frames:-0}" = 4
