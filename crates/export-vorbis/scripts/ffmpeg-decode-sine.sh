#!/bin/sh
# Host-side check. The library does not call ffmpeg.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
cargo run --manifest-path "$root/Cargo.toml" --quiet --release --example write_sine -- "$out/sine.ogg"
ffmpeg -hide_banner -v error -i "$out/sine.ogg" -f f32le -ac 1 -ar 48000 "$out/sine.pcm"
python3 - "$out/sine.pcm" << 'PY'
import math
import struct
import sys
from pathlib import Path

raw = Path(sys.argv[1]).read_bytes()
if len(raw) < 4 or len(raw) % 4:
    raise SystemExit("ffmpeg produced no PCM")
samples = struct.unpack("<" + "f" * (len(raw) // 4), raw)
if len(samples) != 32:
    raise SystemExit(f"expected 32 samples, got {len(samples)}")
peak = max(abs(sample) for sample in samples)
if peak < 1e-4:
    raise SystemExit("decoded frame is still silence")
error = 0.0
for n, sample in enumerate(samples):
    want = 0.5 * math.sin(2 * math.pi * 440 * n / 48000)
    error += (sample - want) ** 2
rms = (error / len(samples)) ** 0.5
print(f"ffmpeg decoded {len(samples)} samples peak={peak:.5f} rms={rms:.5f}")
if rms > 0.02:
    raise SystemExit("decoded frame is not the half-scale sine")
PY
