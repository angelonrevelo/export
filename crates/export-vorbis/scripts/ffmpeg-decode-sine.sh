#!/bin/sh
# Host-side check. The library does not call ffmpeg.
set -eu
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
out=$(mktemp -d)
trap 'rm -rf "$out"' EXIT
cargo run --manifest-path "$root/Cargo.toml" --quiet --release --example write_sine -- "$out/sine.ogg"
cargo run --manifest-path "$root/Cargo.toml" --quiet --release --example write_sine_frame -- "$out/frame.ogg"
cargo run --manifest-path "$root/Cargo.toml" --quiet --release --example write_sine_pair -- "$out/pair.ogg"
ffmpeg -hide_banner -v error -i "$out/sine.ogg" -f f32le -ac 1 -ar 48000 "$out/sine.pcm"
ffmpeg -hide_banner -v error -i "$out/pair.ogg" -f f32le -ac 1 -ar 48000 "$out/pair.pcm"
ffmpeg -hide_banner -v error -i "$out/frame.ogg" -f f32le -ac 1 -ar 48000 "$out/frame-default.pcm"
ffmpeg -hide_banner -v error -flags2 +skip_manual -i "$out/frame.ogg" -f f32le -ac 1 -ar 48000 "$out/frame.pcm"
python3 - "$out/sine.pcm" "$out/frame.pcm" "$out/frame-default.pcm" "$out/pair.pcm" << 'PY'
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

def report(label, samples, n0):
    count = len(samples)
    if count == 0:
        print(f"{label} samples=0")
        return
    peak = max(abs(sample) for sample in samples)
    error = 0.0
    for index, sample in enumerate(samples):
        n = n0 + index
        want = 0.5 * math.sin(2 * math.pi * 440 * n / 48000)
        error += (sample - want) ** 2
    rms = (error / count) ** 0.5
    print(f"{label} samples={count} peak={peak:.5f} rms={rms:.5f} n0={n0}")

file_samples = load(sys.argv[1])
print(f"sine file samples={len(file_samples)}")
if len(file_samples) >= 64:
    report("two-frame second", file_samples[32:64], 0)
if len(file_samples) >= 128:
    report("continuation", file_samples[96:128], 32)
elif len(file_samples) >= 96:
    report("continuation", file_samples[64:96], 32)
report("packet alone default", load(sys.argv[3]), 0)
report("packet alone skip_manual", load(sys.argv[2]), 0)
pair = load(sys.argv[4])
print(f"pair file samples={len(pair)}")
if len(pair) >= 64:
    report("pair second", pair[32:64], 32)
if len(pair) >= 96:
    report("pair third", pair[64:96], 64)
PY
