#!/bin/sh
# Host-side check. The library does not call ffmpeg.
# ffmpeg/ffprobe must identify the remuxed stream as matroska or webm.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
small=$(mktemp "${TMPDIR:-/tmp}/export-webm-2x2.XXXXXX.webm")
large=$(mktemp "${TMPDIR:-/tmp}/export-webm-large.XXXXXX.webm")
vp8=$(mktemp "${TMPDIR:-/tmp}/export-webm-copy.XXXXXX.webm")
trap 'rm -f "$small" "$large" "$vp8"' EXIT INT TERM

cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$small" 2x2
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$large" large
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example copy_file -- "$root/fixtures/vp8.webm" "$vp8"

# `ffmpeg -i` exits 1 when no output is requested. Rejection is a missing
# container line, not that exit code.
identify() {
  label=$1
  file=$2
  log=$(ffmpeg -hide_banner -i "$file" 2>&1 || true)
  printf '%s\n' "$log"
  printf '%s\n' "$log" | grep -E 'Input #0, (matroska|webm)' >/dev/null
  echo "identified $label $(printf '%s\n' "$log" | grep -E 'Input #0, (matroska|webm)' | head -n 1)"
}

identify 2x2 "$small"
identify large "$large"

vp8_log=$(ffmpeg -hide_banner -i "$vp8" 2>&1 || true)
printf '%s\n' "$vp8_log"
printf '%s\n' "$vp8_log" | grep -E 'Video: vp8[, ]' >/dev/null
echo "codec $(printf '%s\n' "$vp8_log" | grep -E 'Video: vp8[, ]' | head -n 1)"
