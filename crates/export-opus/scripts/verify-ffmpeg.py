#!/usr/bin/env python3
"""Decode the packet this crate wrote with ffmpeg and compare PCM samples.

The library does not call ffmpeg. This host script does, and only to decode.
"""

import struct
import subprocess
import sys
import tempfile
from pathlib import Path

CRATE = Path(__file__).resolve().parents[1]
OGG_CRC_POLY = 0x04C11DB7


def ogg_crc(data: bytes) -> int:
    crc = 0
    for byte in data:
        crc ^= byte << 24
        for _ in range(8):
            if crc & 0x80000000:
                crc = ((crc << 1) ^ OGG_CRC_POLY) & 0xFFFFFFFF
            else:
                crc = (crc << 1) & 0xFFFFFFFF
    return crc


def ogg_page(header_type: int, granule: int, serial: int, seq: int, payload: bytes) -> bytes:
    segments = []
    remaining = len(payload)
    while remaining >= 255:
        segments.append(255)
        remaining -= 255
    segments.append(remaining)
    head = bytearray()
    head += b"OggS"
    head += bytes([0, header_type])
    head += struct.pack("<QII", granule, serial, seq)
    head += b"\x00\x00\x00\x00"
    head += bytes([len(segments)])
    head += bytes(segments)
    page = bytes(head) + payload
    crc = ogg_crc(page)
    return page[:22] + struct.pack("<I", crc) + page[26:]


def opus_ogg(packet: bytes) -> bytes:
    head = (
        b"OpusHead"
        + bytes([1, 1])
        + struct.pack("<H", 0)
        + struct.pack("<I", 48000)
        + struct.pack("<h", 0)
        + bytes([0])
    )
    vendor = b"export-opus"
    tags = b"OpusTags" + struct.pack("<I", len(vendor)) + vendor + struct.pack("<I", 0)
    serial = 0x0E190001
    return b"".join(
        [
            ogg_page(0x02, 0, serial, 0, head),
            ogg_page(0x00, 0, serial, 1, tags),
            ogg_page(0x04, 960, serial, 2, packet),
        ]
    )


def pcm_samples(blob: bytes) -> list[int]:
    if len(blob) % 2 != 0:
        raise SystemExit(f"odd pcm length {len(blob)}")
    return list(struct.unpack("<" + "h" * (len(blob) // 2), blob))


def frame_stats(samples: list[int]) -> tuple[float, int, int]:
    energy = sum(sample * sample for sample in samples)
    rms = (energy / len(samples)) ** 0.5
    peak = max(abs(sample) for sample in samples)
    onset = max(abs(sample) for sample in samples[:8])
    return rms, peak, onset


def check_frame(tmp_path: Path, kind: str) -> int:
    packet_path = tmp_path / f"{kind}.opus"
    expected_path = tmp_path / f"{kind}.pcm"
    ogg_path = tmp_path / f"{kind}.ogg"
    decoded_path = tmp_path / f"{kind}-ffmpeg.pcm"
    subprocess.run(
        [
            "cargo",
            "run",
            "--release",
            "--example",
            "dump-packet",
            "--",
            kind,
            str(packet_path),
            str(expected_path),
        ],
        cwd=CRATE,
        check=True,
    )
    packet = packet_path.read_bytes()
    ogg_path.write_bytes(opus_ogg(packet))
    result = subprocess.run(
        [
            "ffmpeg",
            "-y",
            "-v",
            "error",
            "-i",
            str(ogg_path),
            "-f",
            "s16le",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "48000",
            "-ac",
            "1",
            str(decoded_path),
        ],
        capture_output=True,
    )
    if result.returncode != 0:
        sys.stderr.write(result.stderr.decode("utf-8", "replace"))
        print(f"ffmpeg could not decode the {kind} packet", file=sys.stderr)
        return 1
    expected = pcm_samples(expected_path.read_bytes())
    got = pcm_samples(decoded_path.read_bytes())
    if got != expected:
        print(
            f"{kind} sample mismatch: ffmpeg={len(got)} crate={len(expected)}",
            file=sys.stderr,
        )
        preview = min(8, len(got), len(expected))
        print(f"ffmpeg[:{preview}]={got[:preview]}", file=sys.stderr)
        print(f"crate[:{preview}]={expected[:preview]}", file=sys.stderr)
        return 1
    rms, peak, onset = frame_stats(got)
    print(f"{kind} ffmpeg matched {len(got)} samples rms={rms:.1f} peak={peak} onset_peak={onset}")
    if kind == "sine" and rms < 1000:
        print("sine decode is near silence; not claiming the tone", file=sys.stderr)
        return 1
    return 0


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="export-opus-") as tmp:
        tmp_path = Path(tmp)
        status = 0
        for kind in ("silence", "sine"):
            status |= check_frame(tmp_path, kind)
        return status


if __name__ == "__main__":
    raise SystemExit(main())
