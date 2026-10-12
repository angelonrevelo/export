# export-pack

Streaming store-zip, a CBZ-to-PDF path, and a TTF/OTF container copy. No dependencies.

An archive is a structured file: local headers, store-method payload, a CRC-32 of those bytes, a central directory. The writer puts that in `path.tmp` in the same directory, fsyncs, and renames onto `path`. A crash leaves the temporary file, which is not the archive. A general-purpose compressor wrapped around the finished zip is not this design.

## What works

| Format | Read | Write |
|---|---|---|
| Zip, store (method 0) | yes | yes |
| Zip, deflate stored blocks | yes | no (writer stays store) |
| Zip, deflate Huffman | refused | no |
| CBZ of PNG/JPEG to a one-image-per-page PDF | yes | the PDF |
| TTF ↔ OTF container copy | yes | yes |
| Zip64, encrypted, split | refused | no |
| WOFF, WOFF2 | refused | no |

Entry payloads are copied in 64 KiB chunks. The central directory is the only zip structure kept in memory, and it is bounded by 65535 entries. Deflate read accepts stored blocks only. A fixed or dynamic Huffman block returns `fixed huffman` or `dynamic huffman` and writes nothing. CBZ conversion holds one image at a time while decoding, then the page list while the PDF is written.

`Entry::name` is the raw central-directory name. Join it onto a directory only after `safe_name()` succeeds.

## Build

From this directory, so this `Cargo.toml` is the crate root and the release profile applies:

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo build --release --bin pack-bench
./target/release/pack-bench
```

A debug bench prints `NOT JUDGED` and does not report a speed.

## Measured

Release profile (`opt-level = 3`, thin LTO, `codegen-units = 1`). `MB/s` counts 1e6 bytes. `MiB/s` counts 2^20.

### Store zip

These numbers are the first-cut measurement and are not replaced by later runs of the same binary. One 32 MiB store entry (`payload.bin`), CRC included. Median of 5 measured passes, each after one warmup pass in the same process. Write is the payload stream before fsync. Finish is the central directory, fsync, and rename. Read follows write in-process, so the page cache may hold the file. Allocations are process allocations during the timed call, including the chunk buffer.

| | median |
|---|---|
| Write | 384.6 MB/s (366.8 MiB/s) |
| Read | 427.7 MB/s (407.9 MiB/s) |
| Finish | 52.7 ms |
| Write allocations | 0.188 / MiB, 2056.7 bytes / MiB |
| Read allocations | 0.125 / MiB, 4098.8 bytes / MiB |

Write spread across the five passes was 364.7–388.4 MB/s. Read spread was 420.8–437.5 MB/s. Allocation counts did not move.

### New paths

Median of 5 measured passes after one warmup, from `./target/release/pack-bench` on this tree. Allocation counts did not move across those passes.

| | median | spread |
|---|---|---|
| Stored-block inflate, 32 MiB | 135.9 MB/s (129.6 MiB/s) | 101.1–141.6 MB/s |
| CBZ → PDF, 8×2048×1024 RGB | 66.6 MB/s (63.5 MiB/s) | 63.4–67.8 MB/s |
| Tiny font TTF→OTF, 492 bytes × 5000 | 615.3 MB/s (586.9 MiB/s) | 552.7–649.8 MB/s |

Inflate allocations were 0.6 / MiB and 2,098,214.0 bytes / MiB. That byte count is the output `Vec` growing to the uncompressed size. The stream is `zlib_store` (CMF/FLG `0x78 0x01`, stored deflate blocks), including the Adler-32 check. Huffman is not this rate: CPython’s `zlib.compress(b"hello")` is a fixed-Huffman zlib stream, and inflating it returns `fixed huffman` with no output bytes.

PDF rate counts decoded RGB bytes. The timed call is `cbz_to_pdf`: zip read, stored-block PNG inflate, PDF write, fsync, and rename. The CBZ stays in the page cache after warmup. Allocations were 8.6 / MiB and 8,736,069.9 bytes / MiB, because each page is held decoded and the PDF is written from that buffer.

The font rate is a repeated tiny-font container copy, not a bulk font benchmark. The working set is 492 bytes. Allocations were 70,331.3 / MiB and 6,419,331.1 bytes / MiB, which is the per-copy directory and table rebuild spread over a few hundred bytes.

### Sixteen pages

Same 2048×1024 RGB image as the eight-page row, stored-zlib PNG, sixteen copies in one CBZ. 100 measured passes after one warmup. `cbz_to_pdf` includes zip read, stored-block inflate, PDF write, fsync, and rename. The CBZ stays in the page cache after warmup. Percentiles are nearest-rank on MB/s sorted ascending, so p99 is the fast tail. The eight-page row above is unchanged.

| | |
|---|---|
| Pages | 16 |
| RGB bytes | 100,663,296 |
| p50 | 84.9 MB/s |
| p99 | 87.6 MB/s |

### Thirty-two pages

Same 2048×1024 RGB image, stored-zlib PNG, thirty-two copies in one CBZ. 100 measured passes after one warmup. `cbz_to_pdf` includes zip read, stored-block inflate, PDF write, fsync, and rename. The CBZ stays in the page cache after warmup. Percentiles are nearest-rank on MB/s sorted ascending, so p99 is the fast tail. The eight-page and sixteen-page rows above are unchanged.

| | |
|---|---|
| Pages | 32 |
| RGB bytes | 201,326,592 |
| p50 | 84.7 MB/s |
| p99 | 86.3 MB/s |

### Font round-trip

The existing 492-byte `tiny_ttf` copied TTF → OTF → TTF. 100 measured passes after one warmup, release build, `EXPORT_PACK_BENCH=font-roundtrip`. Each sample is one round-trip. The bytes matched on every pass. Percentiles are nearest-rank on microseconds sorted ascending, so p99 is the slow tail. The CBZ rows above are unchanged.

| | |
|---|---|
| Bytes in | 492 |
| Bytes out | 492 |
| p50 | 1.687 µs |
| p99 | 1.845 µs |

## Honest limits

- The writer emits store only. Deflate read is stored blocks. Fixed and dynamic Huffman return an error and do not emit bytes. Zip64, encryption, and split archives return an error.
- One archive is at most 4 GiB, one entry at most 4 GiB − 2 bytes, at most 65535 entries.
- The end-of-central-directory record must end the file. Trailing bytes are refused.
- On checksum mismatch, `copy_entry` may already have written payload bytes. Discard them. `extract_entry` writes `dest.tmp` and renames only after the CRC matches.
- Timestamps in the headers are fixed at 1980-01-01. The CRC covers payload bytes, not the header.
- PNG decode is 8-bit, non-interlaced, color types 0/2/3/4/6. JPEG is passed through with SOF0/SOF1/SOF2 dimensions and is not decoded to pixels.
- A CBZ with no images errors and does not publish the PDF. Directories, dotfiles, and `__MACOSX` are skipped.
- Font conversion copies SFNT tables and rewrites the scaler (`0x00010000` or `OTTO`) and `head.checkSumAdjustment`. It does not rewrite outlines, and it does not wrap WOFF or WOFF2.
