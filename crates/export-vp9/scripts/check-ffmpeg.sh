#!/bin/sh
# Host-only check. The library does not link or spawn ffmpeg.
set -eu
cd "$(dirname "$0")/.."

generated="$(mktemp)"
ffmpeg_yuv="$(mktemp)"
library_yuv="$(mktemp)"
trap 'rm -f "$generated" "$ffmpeg_yuv" "$library_yuv"' EXIT

python3 scripts/gen-keyframe.py "$generated"
cmp testdata/keyframe.ivf "$generated"

ffmpeg -y -hide_banner -loglevel error \
    -i testdata/keyframe.ivf \
    -f rawvideo -pix_fmt yuv420p \
    "$ffmpeg_yuv"

cmp testdata/ffmpeg.yuv "$ffmpeg_yuv"

cargo run --quiet --release --example decode -- testdata/keyframe.ivf "$library_yuv"
cmp "$ffmpeg_yuv" "$library_yuv"

python3 - "$ffmpeg_yuv" "$library_yuv" << 'PY'
import pathlib, sys
ffmpeg = pathlib.Path(sys.argv[1]).read_bytes()
library = pathlib.Path(sys.argv[2]).read_bytes()
if len(ffmpeg) != len(library) or not ffmpeg:
    raise SystemExit("yuv length mismatch")
max_abs = max(abs(a - b) for a, b in zip(ffmpeg, library))
print(f"max_abs_error={max_abs}")
print(f"error_bound=0")
if max_abs > 0:
    raise SystemExit(f"pixels differ by {max_abs}")
PY
