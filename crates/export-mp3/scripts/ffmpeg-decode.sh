#!/usr/bin/env bash
# Decode the one frame this crate writes. The library does not call ffmpeg.
#
# ffmpeg 6.1's MP3 demuxer seeks to the next frame and must read four header
# bytes there. A file that ends on the frame returns
# "Invalid frame size (N): Could not seek to N." The script repeats the same
# frame so the demuxer can lock, then decodes those bytes.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cargo run --quiet --release --manifest-path "$root/Cargo.toml" --example write-frame -- "$tmp/one.mp3" >/dev/null
bytes="$(wc -c < "$tmp/one.mp3" | tr -d ' ')"
if [[ "$bytes" -lt 4 ]]; then
  echo "frame is shorter than a header ($bytes bytes)" >&2
  exit 1
fi

# Bare frame: demuxer refusal. Suffix: opens without a second spectral frame.
set +e
one_err="$(ffmpeg -hide_banner -v error -i "$tmp/one.mp3" -f f32le "$tmp/one.pcm" 2>&1)"
one_rc=$?
set -e

cargo run --quiet --release --manifest-path "$root/Cargo.toml" --example write-ffmpeg-openable -- "$tmp/openable.mp3" >/dev/null
openable_bytes="$(wc -c < "$tmp/openable.mp3" | tr -d ' ')"
set +e
openable_err="$(ffmpeg -hide_banner -v error -i "$tmp/openable.mp3" -f f32le "$tmp/openable.pcm" 2>&1)"
openable_rc=$?
set -e
if [[ "$openable_rc" -ne 0 ]]; then
  echo "ffmpeg openable file failed rc=$openable_rc" >&2
  echo "$openable_err" >&2
  exit 1
fi

cat "$tmp/one.mp3" "$tmp/one.mp3" > "$tmp/two.mp3"
ffmpeg -y -hide_banner -v error -i "$tmp/two.mp3" -f f32le "$tmp/two.pcm" 2>"$tmp/ff.err"
if [[ -s "$tmp/ff.err" ]]; then
  echo "ffmpeg reported errors decoding the frame:" >&2
  cat "$tmp/ff.err" >&2
  exit 1
fi
pcm_bytes="$(wc -c < "$tmp/two.pcm" | tr -d ' ')"
if [[ "$pcm_bytes" -lt 4 ]]; then
  echo "ffmpeg wrote no PCM" >&2
  exit 1
fi

python3 - "$tmp/two.pcm" << 'PY'
import struct, sys
data = open(sys.argv[1], "rb").read()
n = len(data) // 4
samples = struct.unpack("<" + "f" * n, data[: n * 4])
peak = max(abs(s) for s in samples)
print(f"ffmpeg_pcm_samples={n} peak={peak:.8g}")
if peak == 0.0:
    raise SystemExit("ffmpeg decoded silence from a non-silent frame")
PY

echo "ffmpeg decoded the crate frame (presented twice for the demuxer)"
echo "single_frame_demuxer_rc=${one_rc}"
echo "ffmpeg_openable_bytes=${openable_bytes} openable_rc=${openable_rc}"
