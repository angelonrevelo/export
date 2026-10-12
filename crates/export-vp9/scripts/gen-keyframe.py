#!/usr/bin/env python3
"""Write one IVF keyframe this crate decodes.

The picture is 8x8 or 16x16, profile 0, 8-bit 4:2:0, lossless, DC predicted.
Each 8x8 block's first luma 4x4 carries a positive FOUR token. Every other
transform ends immediately. The 16x16 frame is one keyframe: the 64 and 32
superblock partitions are implied splits, and the 16x16 partition is an
explicit split into four 8x8 blocks. Loop filter, segmentation, and
probability updates are off. Default size 8 keeps the original file bytes.
"""

from __future__ import annotations

import pathlib
import sys


class BitWriter:
    def __init__(self) -> None:
        self.bits: list[int] = []

    def f(self, n: int, value: int) -> None:
        for i in range(n - 1, -1, -1):
            self.bits.append((value >> i) & 1)

    def align(self) -> None:
        while len(self.bits) & 7:
            self.bits.append(0)

    def bytes(self) -> bytes:
        out = bytearray()
        for i in range(0, len(self.bits), 8):
            byte = 0
            for bit in self.bits[i : i + 8]:
                byte = (byte << 1) | bit
            out.append(byte)
        return bytes(out)


class BoolEncoder:
    def __init__(self) -> None:
        self.range = 255
        self.bottom = 0
        self.bit_count = 24
        self.out = bytearray()

    def _add_one(self) -> None:
        index = len(self.out)
        while index > 0 and self.out[index - 1] == 255:
            self.out[index - 1] = 0
            index -= 1
        if index == 0:
            raise RuntimeError("carry past the start of the partition")
        self.out[index - 1] += 1

    def write(self, prob: int, bit: int) -> None:
        split = 1 + (((self.range - 1) * prob) >> 8)
        if bit:
            self.bottom += split
            self.range -= split
        else:
            self.range = split
        while self.range < 128:
            self.range <<= 1
            if self.bottom & (1 << 31):
                self._add_one()
            self.bottom = (self.bottom << 1) & 0xFFFFFFFF
            self.bit_count -= 1
            if self.bit_count == 0:
                self.out.append((self.bottom >> 24) & 255)
                self.bottom &= (1 << 24) - 1
                self.bit_count = 8

    def flush(self) -> bytes:
        count = self.bit_count
        value = self.bottom
        if value & (1 << (32 - count)):
            self._add_one()
        value = (value << (count & 7)) & 0xFFFFFFFF
        count >>= 3
        while count > 0:
            count -= 1
            value = (value << 8) & 0xFFFFFFFF
        for _ in range(4):
            self.out.append((value >> 24) & 255)
            value = (value << 8) & 0xFFFFFFFF
        return bytes(self.out)


def compressed_header() -> bytes:
    encoder = BoolEncoder()
    encoder.write(128, 0)
    encoder.write(128, 0)
    encoder.write(252, 0)
    encoder.write(252, 0)
    encoder.write(252, 0)
    return encoder.flush()


def write_block(encoder: BoolEncoder) -> None:
    # 8x8 partition none uses context 0. Skip, DC luma, and DC chroma have no
    # neighbors that change the probability on this frame. Band 0, context 0
    # luma probabilities, and the Pareto values for node >= 2 at probability
    # 183. Later end-of-block contexts are 1, 1, 0. Chroma ends at context 0.
    y0 = (195, 29, 183)
    y_eob = {0: 195, 1: 84, 3: 8}
    uv0 = 214
    encoder.write(158, 0)
    encoder.write(192, 0)
    encoder.write(137, 0)
    encoder.write(144, 0)
    encoder.write(y0[0], 1)
    encoder.write(y0[1], 1)
    encoder.write(y0[2], 1)
    encoder.write(242, 0)
    encoder.write(179, 1)
    encoder.write(195, 1)
    encoder.write(128, 0)
    encoder.write(y_eob[3], 0)
    for ctx in (1, 1, 0):
        encoder.write(y_eob[ctx], 0)
    encoder.write(uv0, 0)
    encoder.write(uv0, 0)


def tile(size: int) -> bytes:
    encoder = BoolEncoder()
    encoder.write(128, 0)
    if size == 16:
        # Keyframe partition context 4, split: three 1-bits.
        encoder.write(149, 1)
        encoder.write(53, 1)
        encoder.write(53, 1)
        for _ in range(4):
            write_block(encoder)
    else:
        write_block(encoder)
    return encoder.flush()


def frame(size: int) -> bytes:
    header = compressed_header()
    writer = BitWriter()
    writer.f(2, 2)
    writer.f(1, 0)
    writer.f(1, 0)
    writer.f(1, 0)
    writer.f(1, 0)
    writer.f(1, 1)
    writer.f(1, 0)
    writer.f(8, 0x49)
    writer.f(8, 0x83)
    writer.f(8, 0x42)
    writer.f(3, 2)
    writer.f(1, 1)
    writer.f(16, size - 1)
    writer.f(16, size - 1)
    writer.f(1, 0)
    writer.f(1, 0)
    writer.f(1, 1)
    writer.f(2, 0)
    writer.f(6, 0)
    writer.f(3, 0)
    writer.f(1, 0)
    writer.f(8, 0)
    writer.f(1, 0)
    writer.f(1, 0)
    writer.f(1, 0)
    writer.f(1, 0)
    writer.f(1, 0)
    writer.f(16, len(header))
    writer.align()
    return writer.bytes() + header + tile(size)


def ivf(payload: bytes, size: int) -> bytes:
    header = bytearray(b"DKIF")
    header += (0).to_bytes(2, "little")
    header += (32).to_bytes(2, "little")
    header += b"VP90"
    header += size.to_bytes(2, "little")
    header += size.to_bytes(2, "little")
    header += (1).to_bytes(4, "little")
    header += (1).to_bytes(4, "little")
    header += (1).to_bytes(4, "little")
    header += (0).to_bytes(4, "little")
    packet = len(payload).to_bytes(4, "little") + (0).to_bytes(8, "little") + payload
    return bytes(header) + packet


def main() -> None:
    args = sys.argv[1:]
    size = 8
    if args and args[0] == "--size":
        if len(args) < 2 or args[1] not in ("8", "16"):
            raise SystemExit("usage: gen-keyframe.py [--size 8|16] [path]")
        size = int(args[1])
        args = args[2:]
    destination = pathlib.Path(args[0]) if args else None
    data = ivf(frame(size), size)
    if destination is None:
        sys.stdout.buffer.write(data)
    else:
        destination.write_bytes(data)


if __name__ == "__main__":
    main()
