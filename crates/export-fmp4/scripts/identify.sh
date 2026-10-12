#!/bin/sh
# Host-side check. The library does not call ffmpeg.
# ffmpeg must identify the copied stream as a fragmented MP4.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
file=$(mktemp "${TMPDIR:-/tmp}/export-fmp4.XXXXXX.mp4")
trap 'rm -f "$file"' EXIT INT TERM

cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$file"

# `ffmpeg -i` exits 1 when no output is requested. Rejection is a missing
# container line, not that exit code.
log=$(ffmpeg -hide_banner -i "$file" 2>&1 || true)
printf '%s\n' "$log"
printf '%s\n' "$log" | grep -F 'Input #0, mov,mp4,m4a,3gp,3g2,mj2' >/dev/null
echo "identified $(printf '%s\n' "$log" | grep -F 'Input #0, mov,mp4,m4a,3gp,3g2,mj2' | head -n 1)"
