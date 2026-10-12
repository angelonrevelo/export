#!/bin/sh
# Host-only check. The library does not link or spawn ffmpeg.
set -eu
cd "$(dirname "$0")/.."

generated="$(mktemp)"
ffmpeg_yuv="$(mktemp)"
library_yuv="$(mktemp)"
trap 'rm -f "$generated" "$ffmpeg_yuv" "$library_yuv"' EXIT

check_one() {
    size="$1"
    ivf="$2"
    yuv="$3"
    if [ "$size" = 8 ]; then
        python3 scripts/gen-keyframe.py "$generated"
    else
        python3 scripts/gen-keyframe.py --size "$size" "$generated"
    fi
    cmp "$ivf" "$generated"

    ffmpeg -y -hide_banner -loglevel error \
        -i "$ivf" \
        -f rawvideo -pix_fmt yuv420p \
        "$ffmpeg_yuv"

    cmp "$yuv" "$ffmpeg_yuv"

    cargo run --quiet --release --example decode -- "$ivf" "$library_yuv"
    cmp "$ffmpeg_yuv" "$library_yuv"

    python3 - "$ffmpeg_yuv" "$library_yuv" "$ivf" << 'PY'
import pathlib, sys
ffmpeg = pathlib.Path(sys.argv[1]).read_bytes()
library = pathlib.Path(sys.argv[2]).read_bytes()
ivf = pathlib.Path(sys.argv[3]).read_bytes()
if len(ffmpeg) != len(library) or not ffmpeg:
    raise SystemExit("yuv length mismatch")
max_abs = max(abs(a - b) for a, b in zip(ffmpeg, library))
payload = int.from_bytes(ivf[32:36], "little")
print(f"file_bytes={len(ivf)}")
print(f"frame_bytes={payload}")
print(f"max_abs_error={max_abs}")
print(f"error_bound=0")
if max_abs > 0:
    raise SystemExit(f"pixels differ by {max_abs}")
PY
}

check_one 8 testdata/keyframe.ivf testdata/ffmpeg.yuv
check_one 16 testdata/keyframe-16.ivf testdata/ffmpeg-16.yuv
check_one 32 testdata/keyframe-32.ivf testdata/ffmpeg-32.yuv
check_one 64 testdata/keyframe-64.ivf testdata/ffmpeg-64.yuv
check_one 128 testdata/keyframe-128.ivf testdata/ffmpeg-128.yuv
check_one 256 testdata/keyframe-256.ivf testdata/ffmpeg-256.yuv
check_one 512 testdata/keyframe-512.ivf testdata/ffmpeg-512.yuv
check_one 1024 testdata/keyframe-1024.ivf testdata/ffmpeg-1024.yuv
