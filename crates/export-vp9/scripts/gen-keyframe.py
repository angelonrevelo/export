#!/usr/bin/env python3
"""Write one IVF keyframe this crate decodes.

The picture is 8x8, 16x16, 32x32, or 64x64, profile 0, 8-bit 4:2:0, lossless, DC predicted.
On the 8x8 and 16x16 frames, each 8x8 block's first luma 4x4 carries a
positive FOUR token. The 32x32 frame is one keyframe as well: four of its
8x8 blocks carry that token and the other twelve are skipped. The 64x64
frame is one keyframe: four blocks are coded and the rest are skipped. Loop filter,
segmentation, and probability updates are off. Default size 8 keeps the
original file bytes.
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


def write_dc_tokens(encoder: BoolEncoder) -> None:
    # Band 0, context 0 luma probabilities, and the Pareto values for node >= 2
    # at probability 183. Later end-of-block contexts are 1, 1, 0.
    # Chroma ends at context 0.
    y0 = (195, 29, 183)
    y_eob = {0: 195, 1: 84, 3: 8}
    uv0 = 214
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


def write_block(encoder: BoolEncoder) -> None:
    # 8x8 partition none uses context 0. Skip and DC modes use the no-neighbor
    # probabilities on the 8x8 and 16x16 frames.
    encoder.write(158, 0)
    encoder.write(192, 0)
    encoder.write(137, 0)
    encoder.write(144, 0)
    write_dc_tokens(encoder)


def write_coded(encoder: BoolEncoder, skip_prob: int, part_prob: int = 158) -> None:
    encoder.write(part_prob, 0)
    encoder.write(skip_prob, 0)
    encoder.write(137, 0)
    encoder.write(144, 0)
    write_dc_tokens(encoder)


def write_skipped(encoder: BoolEncoder, skip_prob: int, part_prob: int = 158) -> None:
    encoder.write(part_prob, 0)
    encoder.write(skip_prob, 1)
    encoder.write(137, 0)
    encoder.write(144, 0)


def write_split(encoder: BoolEncoder, probs: tuple[int, int, int]) -> None:
    for prob in probs:
        encoder.write(prob, 1)


KF_PARTITION = (
    (158, 97, 94),
    (93, 24, 99),
    (85, 119, 44),
    (62, 59, 67),
    (149, 53, 53),
    (94, 20, 48),
    (83, 53, 24),
    (52, 18, 18),
    (150, 40, 39),
    (78, 12, 26),
    (67, 33, 11),
    (24, 7, 5),
    (174, 35, 49),
    (68, 11, 27),
    (57, 15, 9),
    (12, 3, 3),
)
SKIP_PROB = (192, 128, 64)
# Top-left 8x8 of each 32x32 quadrant.
CODED_64 = {(0, 0), (0, 4), (4, 0), (4, 4)}


def write_tile_64(encoder: BoolEncoder) -> None:
    """One intra 64x64. Splits follow the same context rules as the decoder."""
    above = [0] * 8
    left = [0] * 8
    skips = [[0] * 8 for _ in range(8)]
    level = {64: 3, 32: 2, 16: 1, 8: 0}
    wide = {64: 8, 32: 4, 16: 2, 8: 1}
    child = {64: 32, 32: 16, 16: 8}

    def partition_ctx(bsize: int, row: int, col: int) -> int:
        num8 = wide[bsize]
        bsl = level[bsize]
        boffset = 3 - bsl
        above_acc = 0
        left_acc = 0
        for i in range(num8):
            above_acc |= above[col + i]
            left_acc |= left[row + i]
        above_bit = 1 if above_acc & (1 << boffset) else 0
        left_bit = 1 if left_acc & (1 << boffset) else 0
        return bsl * 4 + left_bit * 2 + above_bit

    def walk(row: int, col: int, bsize: int) -> None:
        if row >= 8 or col >= 8:
            return
        num8 = wide[bsize]
        half = num8 >> 1
        has_rows = row + half < 8
        has_cols = col + half < 8
        if bsize == 8:
            part = KF_PARTITION[partition_ctx(bsize, row, col)][0]
            skip_ctx = 0
            if row > 0:
                skip_ctx += skips[row - 1][col]
            if col > 0:
                skip_ctx += skips[row][col - 1]
            prob = SKIP_PROB[skip_ctx]
            if (row, col) in CODED_64:
                write_coded(encoder, prob, part)
                skips[row][col] = 0
            else:
                write_skipped(encoder, prob, part)
                skips[row][col] = 1
            above[col] = 7
            left[row] = 7
            return
        if has_rows and has_cols:
            write_split(encoder, KF_PARTITION[partition_ctx(bsize, row, col)])
        elif has_rows or has_cols:
            raise RuntimeError("one-sided partition is not generated")
        walk(row, col, child[bsize])
        walk(row, col + half, child[bsize])
        walk(row + half, col, child[bsize])
        walk(row + half, col + half, child[bsize])

    walk(0, 0, 64)


def tile(size: int) -> bytes:
    encoder = BoolEncoder()
    encoder.write(128, 0)
    if size == 32:
        # The 64x64 split is implied. Context 8 splits the 32x32. The four
        # 16x16 splits then use contexts 4, 6, 5, and 7.
        # One coded 8x8 in each quadrant. The other twelve blocks are skipped.
        # Skip probabilities follow the neighbor skip flags.
        write_split(encoder, (150, 40, 39))
        quadrants = (
            ((149, 53, 53), ((0, 192), (1, 192), (1, 192), (1, 64))),
            ((83, 53, 24), ((0, 128), (1, 192), (1, 128), (1, 64))),
            ((94, 20, 48), ((0, 128), (1, 128), (1, 192), (1, 64))),
            ((52, 18, 18), ((0, 64), (1, 128), (1, 128), (1, 64))),
        )
        for probs, blocks in quadrants:
            write_split(encoder, probs)
            for skipped, skip_prob in blocks:
                if skipped:
                    write_skipped(encoder, skip_prob)
                else:
                    write_coded(encoder, skip_prob)
    elif size == 64:
        write_tile_64(encoder)
    elif size == 16:
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
        if len(args) < 2 or args[1] not in ("8", "16", "32", "64"):
            raise SystemExit("usage: gen-keyframe.py [--size 8|16|32|64] [path]")
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
