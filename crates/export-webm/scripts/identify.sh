#!/bin/sh
# Host-side check. The library does not call ffmpeg.
# ffmpeg/ffprobe must identify the remuxed stream as matroska or webm.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
out=$(mktemp "${TMPDIR:-/tmp}/export-webm.XXXXXX.webm")
trap 'rm -f "$out"' EXIT INT TERM

cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$out"

# `ffmpeg -i` exits 1 when no output is requested. Rejection is a missing
# container line, not that exit code.
log=$(ffmpeg -hide_banner -i "$out" 2>&1 || true)
printf '%s\n' "$log"
printf '%s\n' "$log" | grep -E 'Input #0, (matroska|webm)' >/dev/null
echo "identified $(printf '%s\n' "$log" | grep -E 'Input #0, (matroska|webm)' | head -n 1)"
