#!/usr/bin/env python3
"""Byte-compare oracle for raw DEFLATE. The crate does not link zlib.

    python3 zlib_oracle.py decompress < compressed > raw
    python3 zlib_oracle.py compress   < raw        > compressed

`compress` uses the stdlib zlib wrapper at wbits=-15 (no zlib header).
"""

import sys
import zlib


def main() -> None:
    if len(sys.argv) != 2 or sys.argv[1] not in ("compress", "decompress"):
        sys.stderr.write("usage: zlib_oracle.py compress|decompress\n")
        sys.exit(2)
    data = sys.stdin.buffer.read()
    if sys.argv[1] == "decompress":
        sys.stdout.buffer.write(zlib.decompress(data, wbits=-15))
    else:
        encoder = zlib.compressobj(level=6, wbits=-15)
        sys.stdout.buffer.write(encoder.compress(data) + encoder.flush())


if __name__ == "__main__":
    main()
