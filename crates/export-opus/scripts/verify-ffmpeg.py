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
    return opus_ogg_frames([packet])


def opus_ogg_frames(packets: list[bytes]) -> bytes:
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
    pages = [
        ogg_page(0x02, 0, serial, 0, head),
        ogg_page(0x00, 0, serial, 1, tags),
    ]
    for index, packet in enumerate(packets):
        flags = 0x04 if index == len(packets) - 1 else 0x00
        granule = 960 * (index + 1)
        pages.append(ogg_page(flags, granule, serial, 2 + index, packet))
    return b"".join(pages)


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


def check_two_frames(tmp_path: Path) -> int:
    first_path = tmp_path / "sine-two-1.opus"
    second_path = tmp_path / "sine-two-2.opus"
    expected_path = tmp_path / "sine-two.pcm"
    alone_path = tmp_path / "sine-two-alone.pcm"
    ogg_path = tmp_path / "sine-two.ogg"
    decoded_path = tmp_path / "sine-two-ffmpeg.pcm"
    subprocess.run(
        [
            "cargo",
            "run",
            "--release",
            "--example",
            "dump-packet",
            "--",
            "sine-two",
            str(first_path),
            str(second_path),
            str(expected_path),
            str(alone_path),
        ],
        cwd=CRATE,
        check=True,
    )
    packets = [first_path.read_bytes(), second_path.read_bytes()]
    ogg_path.write_bytes(opus_ogg_frames(packets))
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
        print("ffmpeg could not decode the two-frame file", file=sys.stderr)
        return 1
    expected = pcm_samples(expected_path.read_bytes())
    got = pcm_samples(decoded_path.read_bytes())
    if got != expected:
        print(
            f"sine-two sample mismatch: ffmpeg={len(got)} crate={len(expected)}",
            file=sys.stderr,
        )
        return 1
    if len(got) < 1920:
        print(f"sine-two short decode: {len(got)} samples", file=sys.stderr)
        return 1
    rms, peak, onset = frame_stats(got[960:1920])
    print(
        f"sine-two ffmpeg matched {len(got)} samples "
        f"frame2 rms={rms:.1f} peak={peak} onset_peak={onset}"
    )
    if rms < 1000:
        print("second frame is near silence", file=sys.stderr)
        return 1
    alone_ogg = tmp_path / "sine-two-alone.ogg"
    alone_decoded = tmp_path / "sine-two-alone-ffmpeg.pcm"
    alone_ogg.write_bytes(opus_ogg(packets[1]))
    alone_result = subprocess.run(
        [
            "ffmpeg",
            "-y",
            "-v",
            "error",
            "-i",
            str(alone_ogg),
            "-f",
            "s16le",
            "-acodec",
            "pcm_s16le",
            "-ar",
            "48000",
            "-ac",
            "1",
            str(alone_decoded),
        ],
        capture_output=True,
    )
    if alone_result.returncode != 0:
        sys.stderr.write(alone_result.stderr.decode("utf-8", "replace"))
        print("ffmpeg could not decode the second packet alone", file=sys.stderr)
        return 1
    alone_expected = pcm_samples(alone_path.read_bytes())
    alone_got = pcm_samples(alone_decoded.read_bytes())
    if alone_got != alone_expected:
        print("lone second packet sample mismatch", file=sys.stderr)
        return 1
    alone_rms, alone_peak, alone_onset = frame_stats(alone_got)
    print(
        f"sine-two-alone ffmpeg matched {len(alone_got)} samples "
        f"rms={alone_rms:.1f} peak={alone_peak} onset_peak={alone_onset}"
    )
    return 0


def main() -> int:
    with tempfile.TemporaryDirectory(prefix="export-opus-") as tmp:
        tmp_path = Path(tmp)
        status = 0
        for kind in ("silence", "sine"):
            status |= check_frame(tmp_path, kind)
        status |= check_two_frames(tmp_path)
        return status


if __name__ == "__main__":
    raise SystemExit(main())
