#!/bin/sh
# Host-side check. The library does not call ffmpeg.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
cargo run --manifest-path "$root/Cargo.toml" --quiet --release --example write_sine -- "$out/sine.ogg"
cargo run --manifest-path "$root/Cargo.toml" --quiet --release --example write_sine_frame -- "$out/frame.ogg"
ffmpeg -hide_banner -v error -i "$out/sine.ogg" -f f32le -ac 1 -ar 48000 "$out/sine.pcm"
ffmpeg -hide_banner -v error -i "$out/frame.ogg" -f f32le -ac 1 -ar 48000 "$out/frame-default.pcm"
ffmpeg -hide_banner -v error -flags2 +skip_manual -i "$out/frame.ogg" -f f32le -ac 1 -ar 48000 "$out/frame.pcm"
python3 - "$out/sine.pcm" "$out/frame.pcm" "$out/frame-default.pcm" << 'PY'
import math
import struct
import sys
from pathlib import Path

def load(path):
    raw = Path(path).read_bytes()
    if len(raw) % 4:
        raise SystemExit(f"{path} is not f32le")
    if not raw:
        return ()
    return struct.unpack("<" + "f" * (len(raw) // 4), raw)

def report(label, samples):
    count = len(samples)
    if count == 0:
        print(f"{label} samples=0")
        return
    peak = max(abs(sample) for sample in samples)
    error = 0.0
    for n, sample in enumerate(samples):
        want = 0.5 * math.sin(2 * math.pi * 440 * n / 48000)
        error += (sample - want) ** 2
    rms = (error / count) ** 0.5
    print(f"{label} samples={count} peak={peak:.5f} rms={rms:.5f}")

file_samples = load(sys.argv[1])
print(f"two-frame file samples={len(file_samples)}")
if len(file_samples) < 64:
    report("two-frame second", file_samples)
else:
    report("two-frame second", file_samples[-32:])
report("packet alone default", load(sys.argv[3]))
report("packet alone skip_manual", load(sys.argv[2]))
PY
