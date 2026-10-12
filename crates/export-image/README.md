# export-image

Standalone crate. Its `Cargo.toml` has an empty `[dependencies]` list, and the library does not shell out to Pillow, ImageMagick, ffmpeg, rembg, or libwebp. Release and bench builds use `opt-level = 3`, `lto = "thin"`, and `codegen-units = 1`.

## Paths that work

- PNG decode (8-bit gray, gray+alpha, RGB, RGBA, indexed; 1/2/4-bit gray and indexed; Adam7 at 8 bits; filters 0–4). Encode is sample-exact, not a byte-identical rewrite of an arbitrary PNG.
- Lossless restore keeps the original bytes. `restore_lossless_all` counts files that fail.
- Baseline JPEG, SOF0, 8-bit, 4:2:0. Encode and decode. A foreign baseline file is checked against an independent decode.
- Shrink searches JPEG quality from 1 to 95, records probes, and pads to the cap. A source already under the cap is copied.
- Thumbnail fits the long edge to 220 pixels and encodes JPEG at quality 72. The read cap is 50 MiB (52,428,800 bytes). Input is PNG or baseline JPEG.

## WebP, and the limit

The sniffer accepts a RIFF container whose form type is `WEBP`. Decode and encode cover one still frame.

**Lossless (`VP8L`).** `encode_webp_lossless` writes one prefix-code group of literal pixels: no predictor, color transform, palette, color cache, or meta-Huffman image. `decode_webp` of that file restores the original RGB or RGBA samples. The reader also accepts a VP8L stream that uses those transforms. A meta-Huffman image with more than 64 prefix groups is `Unsupported`.

**Lossy (`VP8 `).** `encode_webp_lossy` writes one keyframe and does not search a quality:

- profile 0, shown, one coefficient partition
- segmentation off, loop filter level 0
- every macroblock is intra 16×16 DC, and chroma is DC
- width and height are multiples of 16 and at most 16383
- three channels. There is no alpha plane, so RGBA is `Unsupported`
- quantizer index 8

Color conversion is limited-range BT.601. `decode_webp(encode_webp_lossy(image))` matches the BT.601 reconstruction of the source samples. That reconstruction is not the source RGB when a channel is saturated, so a lossy round-trip is not a promise that the original bytes come back. A flat color’s YUV samples match an independent decode of the same frame. Two neighboring gray macroblocks stay within 2 levels of the source.

**Errors.**

- Not `RIFF`, or `RIFF` whose form is not `WEBP`: `Invalid`.
- A VP8 sync other than `9d 01 2a`, or a VP8L signature other than `0x2f`: `Invalid`.
- A RIFF size, chunk size, or VP8 partition length that runs past the buffer: `Truncated`. A VP8 payload shorter than the 10-byte frame header is `Truncated`.
- `VP8X`, `ALPH`, `ANIM`, `ANMF`, and any other chunk: `Unsupported`. Animation, alpha-only, and extended WebP are not round-tripped.
- Interframes, a version other than 0, segmentation, a loop filter, more than one coefficient partition, a scaling bit, a size that is not a multiple of 16, or a prediction mode other than DC: `Unsupported`.

The VP8 bool decoder zero-fills a coefficient partition that ends early, which is what the frame format specifies. That shortfall is not reported as `Truncated`. Truncation is decided from the container and the frame header.

`thumbnail_jpeg` and `shrink_png` do not take WebP. A WebP buffer is `Unsupported` on the thumbnail path, and shrink still requires a PNG.

## Not in this crate

HEIC, AVIF, TIFF, SVG, RAW, JPEG XL, background removal, and vectorization.

Progressive JPEG (SOF2), arithmetic JPEG, and unknown markers stay `Unsupported`. They are not decoded.

## Benchmarks

`cargo bench --manifest-path crates/export-image/Cargo.toml` uses the bench profile. Warmup calls are excluded. Percentiles are nearest-rank on the high side, index `ceil(p * n) - 1`. For these sample counts, p99 is the slowest sample. Pixel MB/s is uncompressed pixel bytes per second.

The PNG, resize, JPEG, shrink, and thumbnail rows below are the numbers already published for those paths. Those bench bodies were not rewritten, and the rows were not replaced with a later run.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| PNG decode 640×480 (file 728,763 bytes) | 80 | 14,929.1 µs | 16,452.1 µs | 61.73 (p99 56.02) | file MB/s p50 48.81; 22 allocs; 71.61 allocs/MP |
| area resize 640×480 → 220×165 | 60 | 945.8 µs | 1,344.4 µs | 974.42 | 1 alloc; 27.55 allocs per output MP |
| bilinear resize 640×480 → 220×165 | 40 | 1,076.4 µs | 1,158.2 µs | 856.19 | 1 alloc; 27.55 allocs per output MP |
| JPEG encode q=60 640×480 (microbenchmark, not the shrink setting; 14,064 bytes) | 20 | 31,081.2 µs | 33,629.1 µs | 29.65 | file MB/s p50 0.45; 15 allocs; 48.83 allocs/MP |
| shrink 96×72 PNG to 8,000 bytes, fresh cache | 12 | 5,827.4 µs | 5,988.8 µs | 3.56 | 93 allocs; 13,454.86 allocs/MP |
| thumbnail q=72, 640×480 → 220×165 (3,703 bytes) | 20 | 20,402.9 µs | 21,764.4 µs | 45.17 | 36 allocs; 117.19 allocs/MP; read limit 52,428,800 |

One shrink call outside the timer: quality 95, 0 scales, 7 probes, 8,000 output bytes, not a copy. Lossless report on two good PNGs and one bad buffer: seen 3, restored 2, failed 1.

WebP rows are appended on the same 640×480 gradient. This is the release bench of the lookup decoder below. The earlier PNG, resize, JPEG, shrink, and thumbnail bodies were not changed, and their reprinted times from this process were not substituted for the table above.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| VP8L decode 640×480 (file 921,676 bytes) | 40 | 7,750.6 µs | 7,896.5 µs | 118.91 (p99 116.71) | file MB/s p50 118.92; 12 allocs; 39.06 allocs/MP |
| VP8 decode 640×480, quantizer index 8 (file 16,288 bytes) | 40 | 4,019.9 µs | 4,230.4 µs | 229.26 (p99 217.85) | file MB/s p50 4.05; 5 allocs; 16.28 allocs/MP |
| WebP decode miss, `RIFF` form `JPEG` (`Invalid`) | 80 | 24 ns | 44 ns | — | 0 allocs |

The miss is a bad signature. It returns before a chunk is decoded.

## Huffman lookup

VP8L codes of one to eight bits go through a 256-entry table. Longer codes still walk one bit at a time. The table was compared with the bit-by-bit reader on the same 640×480 file.

The bit-by-bit binary, run twice: p50 9,227.2 µs (p99 12,894.4 µs), then p50 9,271.7 µs (p99 9,554.9 µs). The p50 moved 45 µs. That second run’s p99−p50 spread is 283 µs. The first p99 was one slow sample.

The lookup’s p50 is 7,750.6 µs, about 1.5 ms under the bit-by-bit p50. That is outside the 45 µs repeat and outside the 283 µs spread, so the table stays. It is inside the first run’s p99−p50 gap only because of that one slow sample.

`cargo build --release --target wasm32-unknown-unknown --manifest-path crates/export-image/Cargo.toml` succeeds for this revision. The bench harness is native-only.
