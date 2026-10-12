#!/bin/sh
# Host-side check. The library does not call ffmpeg.
# ffmpeg must identify the PCM copy as a fragmented MP4, and the H.264 copy
# as h264. The library does not decode that bitstream.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
pcm=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-pcm.XXXXXX.mp4")
h264=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-h264.XXXXXX.mp4")
h264x47=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-h264x47.XXXXXX.mp4")
pcm_log=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-pcm.XXXXXX.log")
h264_log=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-h264.XXXXXX.log")
h264x47_log=$(mktemp "${TMPDIR:-/tmp}/export-fmp4-h264x47.XXXXXX.log")
trap 'rm -f "$pcm" "$h264" "$h264x47" "$pcm_log" "$h264_log" "$h264x47_log"' EXIT INT TERM

cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$pcm" pcm
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$h264" h264
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$h264x47" h264x47

# `ffmpeg -i` exits 1 when no output is requested. Rejection is a missing
# container line, not that exit code.
ffmpeg -hide_banner -i "$pcm" >"$pcm_log" 2>&1 || true
ffmpeg -hide_banner -i "$h264" >"$h264_log" 2>&1 || true
# Decode to a null mux. A rejected access unit exits non-zero or prints no frame.
ffmpeg -hide_banner -i "$h264x47" -f null - >"$h264x47_log" 2>&1
cat "$pcm_log"
cat "$h264_log"
cat "$h264x47_log"

grep -F 'Input #0, mov,mp4,m4a,3gp,3g2,mj2' "$pcm_log" >/dev/null
grep -F 'Input #0, mov,mp4,m4a,3gp,3g2,mj2' "$h264_log" >/dev/null
grep -E 'Video: h264' "$h264_log" >/dev/null
grep -E 'Video: h264 \(Constrained Baseline\)' "$h264x47_log" >/dev/null
grep -E 'frame=[[:space:]]*47' "$h264x47_log" >/dev/null
if grep -F 'no frame!' "$h264x47_log" >/dev/null; then
    echo "ffmpeg saw no frame in the 47-sample copy" >&2
    exit 1
fi

echo "identified pcm $(grep -F 'Input #0, mov,mp4,m4a,3gp,3g2,mj2' "$pcm_log" | head -n 1)"
echo "identified h264 $(grep -E 'Video: h264' "$h264_log" | head -n 1)"
echo "identified h264x47 $(grep -E 'frame=[[:space:]]*47' "$h264x47_log" | head -n 1)"
