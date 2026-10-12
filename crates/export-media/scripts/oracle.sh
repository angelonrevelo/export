#!/bin/sh
# Host byte-compare oracle. Not a product path: the library never calls ffmpeg.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cargo run --release --manifest-path "$root/Cargo.toml" --example oracle_fixture -- \
  "$tmp/a.wav" "$tmp/a.pcm" "$tmp/a.mp4"
ffmpeg -hide_banner -v error -i "$tmp/a.wav" -f s16le -acodec pcm_s16le "$tmp/wav.pcm"
cmp "$tmp/a.pcm" "$tmp/wav.pcm"
ffmpeg -hide_banner -v error -i "$tmp/a.mp4" -vn -f s16le -acodec pcm_s16le "$tmp/mp4.pcm"
cmp "$tmp/a.pcm" "$tmp/mp4.pcm"
echo "oracle ok"
