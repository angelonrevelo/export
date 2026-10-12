#!/bin/sh
# Host-side check. The library does not call ffmpeg.
# ffmpeg/ffprobe must identify the remuxed stream as matroska or webm.
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
small=$(mktemp "${TMPDIR:-/tmp}/export-webm-2x2.XXXXXX.webm")
large=$(mktemp "${TMPDIR:-/tmp}/export-webm-large.XXXXXX.webm")
vp8=$(mktemp "${TMPDIR:-/tmp}/export-webm-copy.XXXXXX.webm")
vp8_two=$(mktemp "${TMPDIR:-/tmp}/export-webm-copy2.XXXXXX.webm")
vp8_4096=$(mktemp "${TMPDIR:-/tmp}/export-webm-copy4k.XXXXXX.webm")
vp8_460800=$(mktemp "${TMPDIR:-/tmp}/export-webm-copy460k.XXXXXX.webm")
vp9=$(mktemp "${TMPDIR:-/tmp}/export-webm-copy9.XXXXXX.webm")
vp9_4096=$(mktemp "${TMPDIR:-/tmp}/export-webm-copy9k.XXXXXX.webm")
audio=$(mktemp "${TMPDIR:-/tmp}/export-webm-copya.XXXXXX.webm")
trap 'rm -f "$small" "$large" "$vp8" "$vp8_two" "$vp8_4096" "$vp8_460800" "$vp9" "$vp9_4096" "$audio"' EXIT INT TERM

cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$small" 2x2
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$large" large
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example copy_file -- "$root/fixtures/vp8.webm" "$vp8"
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example copy_file -- "$root/fixtures/vp8-two.webm" "$vp8_two"
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$vp8_4096" vp8-4096
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$vp8_460800" vp8-460800
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example copy_file -- "$root/fixtures/vp9.webm" "$vp9"
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example write_sample -- "$vp9_4096" vp9-4096
cargo run --quiet --manifest-path "$root/Cargo.toml" --release --example copy_file -- "$root/fixtures/opus.webm" "$audio"

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

name_vp8() {
  label=$1
  file=$2
  log=$(ffmpeg -hide_banner -i "$file" 2>&1 || true)
  printf '%s\n' "$log"
  printf '%s\n' "$log" | grep -E 'Video: vp8[, ]' >/dev/null
  echo "codec $label $(printf '%s\n' "$log" | grep -E 'Video: vp8[, ]' | head -n 1)"
}

name_vp8 one "$vp8"
name_vp8 two "$vp8_two"
name_vp8 fourk "$vp8_4096"
name_vp8 payload460800 "$vp8_460800"

vp9_log=$(ffmpeg -hide_banner -i "$vp9" 2>&1 || true)
printf '%s\n' "$vp9_log"
printf '%s\n' "$vp9_log" | grep -E 'Video: vp9[, ]' >/dev/null
echo "codec vp9 $(printf '%s\n' "$vp9_log" | grep -E 'Video: vp9[, ]' | head -n 1)"

vp9_4096_log=$(ffmpeg -hide_banner -i "$vp9_4096" 2>&1 || true)
printf '%s\n' "$vp9_4096_log"
printf '%s\n' "$vp9_4096_log" | grep -E 'Video: vp9[, ]' >/dev/null
echo "codec vp9_4096 $(printf '%s\n' "$vp9_4096_log" | grep -E 'Video: vp9[, ]' | head -n 1)"

audio_log=$(ffmpeg -hide_banner -i "$audio" 2>&1 || true)
printf '%s\n' "$audio_log"
printf '%s\n' "$audio_log" | grep -E 'Audio: opus[, ]' >/dev/null
echo "codec audio $(printf '%s\n' "$audio_log" | grep -E 'Audio: opus[, ]' | head -n 1)"
