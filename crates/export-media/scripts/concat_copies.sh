#!/bin/sh
# Host probe. The library does not call ffmpeg.
# Stream-copies two copies of the one-frame audiogram and asks ffprobe
# for the video sample count.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT
cargo run --release --manifest-path "$root/Cargo.toml" --example concat_copies -- \
  "$tmp/out.mp4"
ffprobe -hide_banner -v error -select_streams v:0 \
  -show_entries stream=codec_name,codec_tag_string,width,height,nb_frames \
  -of default=nw=1:nk=0 "$tmp/out.mp4" > "$tmp/probe.txt"
cat "$tmp/probe.txt"
frames=$(awk -F= '$1=="nb_frames" { print $2 }' "$tmp/probe.txt")
width=$(awk -F= '$1=="width" { print $2 }' "$tmp/probe.txt")
height=$(awk -F= '$1=="height" { print $2 }' "$tmp/probe.txt")
codec=$(awk -F= '$1=="codec_name" { print $2 }' "$tmp/probe.txt")
test "$frames" = 2
test "$width" = 640
test "$height" = 120
test "$codec" = rawvideo
echo "concat copies: 2 video samples"
