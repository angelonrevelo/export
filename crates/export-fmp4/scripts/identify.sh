#!/bin/sh
# Host-side check. The library does not call ffmpeg.
# ffmpeg must identify the PCM copy as a fragmented MP4, and the H.264 copy
# as h264. The library does not decode that bitstream.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
pcm=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-pcm.XXXXXX.mp4")
h264=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-h264.XXXXXX.mp4")
pcm_log=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-pcm.XXXXXX.log")
h264_log=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-h264.XXXXXX.log")
trap 'rm -f "$pcm" "$h264" "$pcm_log" "$h264_log"' EXIT INT TERM

cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$pcm" pcm
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$h264" h264

# `ffmpeg -i` exits 1 when no output is requested. Rejection is a missing
# container line, not that exit code.
ffmpeg -hide_banner -i "$pcm" >"$pcm_log" 2>&1 || true
ffmpeg -hide_banner -i "$h264" >"$h264_log" 2>&1 || true
cat "$pcm_log"
cat "$h264_log"

grep -F 'Input #0, mov,mp4,m4a,3gp,3g2,mj2' "$pcm_log" >/dev/null
grep -F 'Input #0, mov,mp4,m4a,3gp,3g2,mj2' "$h264_log" >/dev/null
grep -E 'Video: h264' "$h264_log" >/dev/null

echo "identified pcm $(grep -F 'Input #0, mov,mp4,m4a,3gp,3g2,mj2' "$pcm_log" | head -n 1)"
echo "identified h264 $(grep -E 'Video: h264' "$h264_log" | head -n 1)"
