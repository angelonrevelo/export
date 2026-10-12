#!/bin/sh
# Host-side check. The library does not call ffmpeg.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
cargo run --manifest-path "$root/Cargo.toml" --quiet --release --example write_packet -- "$out/frame.ogg"
ffmpeg -hide_banner -v error -flags2 +skip_manual -i "$out/frame.ogg" -f f32le -ac 1 -ar 48000 "$out/frame.pcm"
python3 - "$out/frame.pcm" << 'PY'
import struct
import sys
from pathlib import Path

raw = Path(sys.argv[1]).read_bytes()
if len(raw) < 4 or len(raw) % 4:
    raise SystemExit("ffmpeg produced no PCM")
samples = struct.unpack("<" + "f" * (len(raw) // 4), raw)
if any(abs(sample) > 1e-5 for sample in samples):
    raise SystemExit("decoded frame is not silence")
print(f"ffmpeg decoded {len(samples)} samples")
PY
