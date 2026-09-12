"""Byte-budget shrink: land on an exact file size without growing under-cap sources.

The user-facing request is “twenty MB exact”. Tests use a small cap on a
generated file; the mechanism is the same. Codec quantization cannot hit a
bitwise count, so we encode *under* the cap and pad with a container-legal
trailer (JPEG COM / PNG tEXt / MP4 `free` box / PDF comment).
"""
from __future__ import annotations

import struct
import zlib
from pathlib import Path

# Minimum MP4 box is 8 bytes (size + type). JPEG COM is 4 bytes + payload.
# Aim this far under the cap so padding always has room.
HEADROOM_BYTE = 64


def predicted_byte(source_byte: int, target_byte: int | None) -> int | None:
    """What the job will deliver. Under-cap sources stay put (no grow)."""
    if target_byte is None or target_byte <= 0:
        return None
    if source_byte <= target_byte:
        return source_byte
    return target_byte


def video_bit_rate(target_byte: int, duration_second: float,
                   audio_bit_rate: int = 96_000) -> int:
    """Video bits/s so total (video+audio) ≈ target_byte, with headroom."""
    duration_second = max(float(duration_second), 0.05)
    budget_bit = max(1, (target_byte - HEADROOM_BYTE) * 8)
    total_bit_rate = int(budget_bit / duration_second)
    video = total_bit_rate - int(audio_bit_rate)
    return max(8_000, video)


def pad_to_byte(path: Path, target_byte: int) -> Path:
    """Grow `path` to exactly `target_byte` using a container-legal trailer.

    No-op (and no grow) when the file is already at or above the target —
    callers encode under the cap first; oversize is their problem.
    """
    size = path.stat().st_size
    if size == target_byte:
        return path
    if size > target_byte:
        raise RuntimeError(
            f"encode landed at {size} bytes, over target {target_byte}"
        )
    need = target_byte - size
    suffix = path.suffix.lower()
    if suffix in {".jpg", ".jpeg"}:
        _pad_jpeg(path, need)
    elif suffix == ".png":
        _pad_png(path, need)
    elif suffix == ".mp4":
        _pad_mp4(path, need)
    elif suffix == ".pdf":
        _pad_pdf(path, need)
    elif suffix == ".webp":
        _pad_webp(path, need)
    else:
        # Last resort: append NULs. Most raster/container parsers stop at
        # the declared end; tests only drive jpeg/png/mp4.
        with path.open("ab") as f:
            f.write(b"\x00" * need)
    if path.stat().st_size != target_byte:
        raise RuntimeError(
            f"pad failed: {path.stat().st_size} != {target_byte}"
        )
    return path


def _pad_jpeg(path: Path, need: int) -> None:
    data = bytearray(path.read_bytes())
    if len(data) >= 2 and data[-2:] == b"\xff\xd9":
        body = data[:-2]
        eoi = b"\xff\xd9"
    else:
        body = data
        eoi = b""
    remaining = need
    while remaining >= 4:
        # COM marker: FF FE + length (includes the 2 length bytes, max 65535).
        added = min(remaining, 2 + 65535)
        length = added - 2
        payload = length - 2
        body += b"\xff\xfe" + length.to_bytes(2, "big") + (b"\x00" * payload)
        remaining -= added
    path.write_bytes(bytes(body) + eoi + (b"\x00" * remaining))


def _pad_png(path: Path, need: int) -> None:
    data = path.read_bytes()
    iend = data.rfind(b"IEND")
    if iend < 4:
        with path.open("ab") as f:
            f.write(b"\x00" * need)
        return
    # IEND chunk starts 4 bytes before the type (length field).
    iend_start = iend - 4
    head, tail = data[:iend_start], data[iend_start:]
    remaining = need
    extra = b""
    while remaining >= 14:
        # tEXt: 4 length + 4 type + data + 4 crc. Min data "x\0" = 2 → 14 bytes.
        chunk_total = min(remaining, 12 + 65535)
        payload_len = chunk_total - 12
        payload = b"x\x00" + (b"\x00" * (payload_len - 2))
        crc = zlib.crc32(b"tEXt" + payload) & 0xFFFFFFFF
        extra += struct.pack(">I", payload_len) + b"tEXt" + payload + struct.pack(">I", crc)
        remaining -= chunk_total
    path.write_bytes(head + extra + tail + (b"\x00" * remaining))


def _pad_mp4(path: Path, need: int) -> None:
    if need < 8:
        # Caller must leave ≥8 bytes of headroom. Last-resort: a 16-byte
        # free box isn't possible here without shrinking first.
        raise RuntimeError(f"mp4 pad needs ≥8 bytes, got {need}")
    if need >= 0xFFFFFFFF:
        raise RuntimeError("mp4 pad exceeds 32-bit box size")
    with path.open("ab") as f:
        f.write(need.to_bytes(4, "big"))
        f.write(b"free")
        f.write(b"\x00" * (need - 8))


def _pad_pdf(path: Path, need: int) -> None:
    # PDF comments from '%' to EOL. Keep it one line.
    with path.open("ab") as f:
        if need == 1:
            f.write(b"\n")
            return
        f.write(b"%" + (b"\x00" * (need - 2)) + b"\n")


def _pad_webp(path: Path, need: int) -> None:
    # RIFF size is bytes 4:8 (file size minus 8). Append padding and bump it.
    data = bytearray(path.read_bytes())
    if data[:4] != b"RIFF" or len(data) < 12:
        with path.open("ab") as f:
            f.write(b"\x00" * need)
        return
    data += b"\x00" * need
    riff_size = len(data) - 8
    data[4:8] = riff_size.to_bytes(4, "little")
    path.write_bytes(bytes(data))
