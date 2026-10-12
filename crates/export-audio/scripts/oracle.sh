#!/bin/sh
# Host byte-compare. The library never calls ffmpeg.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cargo run --release --manifest-path "$root/Cargo.toml" --example audio_oracle_fixture -- \
  "$tmp/in.wav" "$tmp/in.pcm" "$tmp/out.flac"
ffmpeg -hide_banner -v error -i "$tmp/out.flac" -f s16le -acodec pcm_s16le "$tmp/ff.pcm"
cmp "$tmp/in.pcm" "$tmp/ff.pcm"
echo "oracle ok"
