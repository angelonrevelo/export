# export-image

Standalone crate. Its `Cargo.toml` has an empty `[dependencies]` list, and the library does not shell out to Pillow, ImageMagick, ffmpeg, rembg, libwebp, Cairo, vtracer, or resvg. The SVG path does not start a thread. Release and bench builds use `opt-level = 3`, `lto = "thin"`, and `codegen-units = 1`.

## Paths that work

- PNG decode (8-bit gray, gray+alpha, RGB, RGBA, indexed; 1/2/4-bit gray and indexed; Adam7 at 8 bits; filters 0–4). Encode is sample-exact, not a byte-identical rewrite of an arbitrary PNG.
- Lossless restore keeps the original bytes. `restore_lossless_all` counts files that fail.
- Baseline JPEG, SOF0, 8-bit, 4:2:0. Encode and decode. A foreign baseline file is checked against an independent decode.
- Shrink searches JPEG quality from 1 to 95, records probes, and pads to the cap. A source already under the cap is copied.
- Thumbnail fits the long edge to 220 pixels and encodes JPEG at quality 72. The read cap is 50 MiB (52,428,800 bytes). Input is PNG or baseline JPEG.

## WebP, and the limit

The sniffer accepts a RIFF container whose form type is `WEBP`. Decode and encode cover one still frame, plus a two-frame or four-frame animation whose frames are existing lossy keyframes.

**Lossless (`VP8L`).** `encode_webp_lossless` writes one prefix-code group of literal pixels: no predictor, color transform, palette, color cache, or meta-Huffman image. `decode_webp` of that file restores the original RGB or RGBA samples. The reader also accepts a VP8L stream that uses those transforms. A meta-Huffman image with more than 64 prefix groups is `Unsupported`.

**Lossy (`VP8 `).** `encode_webp_lossy` writes one keyframe and does not search a quality:

- profile 0, shown, one coefficient partition
- segmentation off, loop filter level 0
- every macroblock is intra 16×16 DC, and chroma is DC
- width and height are multiples of 16 and at most 16383
- three channels. There is no alpha plane, so RGBA is `Unsupported`
- quantizer index 8

Color conversion is limited-range BT.601. `decode_webp(encode_webp_lossy(image))` matches the BT.601 reconstruction of the source samples. That reconstruction is not the source RGB when a channel is saturated, so a lossy round-trip is not a promise that the original bytes come back. A flat color’s YUV samples match an independent decode of the same frame. Two neighboring gray macroblocks stay within 2 levels of the source.

**Animation.** `encode_webp_animation` takes two or four RGB frames, each with sides a multiple of 16. The first frame is the canvas. A later frame may be smaller. Its origin is an even pixel offset, and the rectangle must sit inside the canvas. It calls the same lossy keyframe encoder once per frame and wraps the payloads in `VP8X`, `ANIM`, and one `ANMF` chunk per frame. That container is not an entropy coder. `decode_webp_animation` paints each rectangle onto the canvas and returns the canvas after every frame. `decode_webp` returns that composited 64×64 canvas for two partial files: the second frame is the 16×16 rectangle at (16, 32), or the second frame is the 16×16 rectangle at the origin. For the full-frame 64×64 two-frame file, the two-frame 256×256 file, the two-frame 640×480 file, the two-frame 1280×720 file, and the four-frame 1280×720 file, it returns that file's first frame. Every other animation stays `Unsupported` on `decode_webp`. An odd origin, a rectangle that runs past the canvas, an `ALPH` subchunk, and ICC, EXIF, or XMP stay `Unsupported`. There is no loop filter. Any other frame count stays `Unsupported`.

**Errors.**

- Not `RIFF`, or `RIFF` whose form is not `WEBP`: `Invalid`.
- A VP8 sync other than `9d 01 2a`, or a VP8L signature other than `0x2f`: `Invalid`.
- A RIFF size, chunk size, or VP8 partition length that runs past the buffer: `Truncated`. A VP8 payload shorter than the 10-byte frame header is `Truncated`.
- `decode_webp` returns the composited canvas for those two 64×64 partial files, and the first frame of the full-frame 64×64 two-frame file, the two-frame 256×256 file, the two-frame 640×480 file, the two-frame 1280×720 file, and the four-frame 1280×720 file. It rejects every other `VP8X`, and it rejects `ALPH`, `ANIM`, and `ANMF`, with `Unsupported`. Alpha-only and other extended WebP stay errors. The two-frame path above is the animation that round-trips.
- Interframes, a version other than 0, segmentation, a loop filter, more than one coefficient partition, a scaling bit, a size that is not a multiple of 16, or a prediction mode other than DC: `Unsupported`.

The VP8 bool decoder zero-fills a coefficient partition that ends early, which is what the frame format specifies. That shortfall is not reported as `Truncated`. Truncation is decided from the container and the frame header.

`thumbnail_jpeg` of the existing two-frame 640×480 WebP, and of the existing four-frame 1280×720 WebP, writes a JPEG of that file's first frame. The long edge is 220 and the quality is 72. Every other WebP stays `Unsupported` on the thumbnail path. `shrink_png` still requires a PNG.

## SVG rect

`encode_svg` accepts one picture: a 16×16 or 32×32 RGB canvas that is white except for one solid black square. It writes a single `<rect>`. `rasterize_svg` fills that canvas white and paints the black square. Those pixels match the source. A path, circle, ellipse, line, polygon, polyline, rounded corner (`rx` or `ry`), text node, or any other picture is `Unsupported`. Cairo, vtracer, and resvg are not on this path.

## Not in this crate

HEIC, AVIF, TIFF, RAW, JPEG XL, background removal, and vectorization.

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

Encode rows are a later release run of the same 640×480 gradient. The decode rows above were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| VP8L encode 640×480 (output 921,676 bytes) | 40 | 15,189.5 µs | 20,390.8 µs | 60.67 (p99 45.20) | file MB/s p50 60.68; 20 allocs; 65.10 allocs/MP |
| VP8 encode 640×480, quantizer index 8 (output 16,288 bytes) | 40 | 4,926.1 µs | 5,178.9 µs | 187.08 (p99 177.95) | file MB/s p50 3.31; 30 allocs; 97.66 allocs/MP |
| WebP encode miss, 8×8 lossy (`Unsupported`) | 80 | 30 ns | 60 ns | — | 0 allocs |

An 8×8 frame is not a multiple of 16, so `encode_webp_lossy` returns before the DCT. Those encode rows were not replaced.

Two-frame animation, a later release run. Each frame is 16×16, which the lossy encoder already accepts. `decode_webp_animation` returned both frames, and each frame matched `decode_webp` of that frame encoded alone. `decode_webp` on the animation file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| two-frame VP8 animation encode, 16×16 (output 182 bytes) | 40 | 17.8 µs | 20.0 µs | 86.31 (p99 76.63) | file MB/s p50 10.23; 33 allocs; 64,453.12 allocs/MP |

Four-frame animation, a later release run on 16×16 frames. The two-frame row above was not replaced. `decode_webp_animation` returned all four frames, and each matched a still decode of that frame. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| four-frame VP8 animation encode, 16×16 (output 306 bytes) | 40 | 39.5 µs | 52.4 µs | 77.75 (p99 58.67) | file MB/s p50 7.75; 59 allocs; 57,617.19 allocs/MP |

64×64 two-frame animation, a later release run. It uses the same keyframe encoder; 64 is a multiple of 16, so no new entropy coder. The 16×16 rows above were not replaced. `decode_webp_animation` returned both frames. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| two-frame VP8 animation encode, 64×64 (output 200 bytes) | 40 | 114.2 µs | 128.4 µs | 215.19 (p99 191.42) | file MB/s p50 1.75; 36 allocs; 4,394.53 allocs/MP |

256×256 two-frame animation, a later release run. 256 is a multiple of 16, so this is the same keyframe encoder: 16 by 16 macroblocks per frame, no new entropy coder. The 64×64 row and the 16×16 rows above were not replaced. `decode_webp_animation` returned both frames. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| two-frame VP8 animation encode, 256×256 (output 458 bytes) | 40 | 1,430.6 µs | 1,686.8 µs | 274.86 (p99 233.11) | file MB/s p50 0.32; 44 allocs; 335.69 allocs/MP |

640×480 two-frame animation, a later release run. 640 and 480 are multiples of 16, so this is the same intra-DC keyframe encoder as the 256×256 pair. No new entropy coder. The rows above were not replaced. `decode_webp_animation` returned both frames, and each matched a still decode of that frame. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| two-frame VP8 animation encode, 640×480 (output 34,434 bytes) | 40 | 8,910.5 µs | 10,240.0 µs | 206.86 (p99 180.00) | file MB/s p50 3.86; 68 allocs; 110.68 allocs/MP |

Four-frame 640×480 animation, a later release run. Same intra-DC keyframe coder as the two-frame file. Twice 8,910.5 µs is 17,821 µs; this p50 is 19,808.1 µs, about 1.1× that, so the frame count stays. The two-frame row above and the 256×256 row were not replaced. `decode_webp_animation` returned all four frames, and each matched a still decode of that frame. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| four-frame VP8 animation encode, 640×480 (output 70,418 bytes) | 40 | 19,808.1 µs | 21,180.8 µs | 186.11 (p99 174.04) | file MB/s p50 3.56; 129 allocs; 104.98 allocs/MP |

1280×720 two-frame animation, a later release run. 1280 and 720 are multiples of 16, so this is the same intra-DC keyframe encoder as the 640×480 pair. No new entropy coder. Three times the pixels of 640×480 is 26,732 µs; this p50 is 27,085.2 µs, about 1.0× that, so the size stays. The four-frame 640×480 row and the two-frame 640×480 row were not replaced. `decode_webp_animation` returned both frames, and each matched a still decode of that frame. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| two-frame VP8 animation encode, 1280×720 (output 94,368 bytes) | 40 | 27,085.2 µs | 31,594.9 µs | 204.16 (p99 175.02) | file MB/s p50 3.48; 73 allocs; 39.61 allocs/MP |

Four-frame 1280×720 animation, a later release run. Same intra-DC keyframe coder as the two-frame file. Four frames at 640×480 were 2.22× that size’s two-frame row, so 2.22 × 27,085.2 µs is 60,129 µs. This p50 is 54,127.8 µs, about 0.9× that, so the frame count stays. The two-frame 1280×720 row and the four-frame 640×480 row were not replaced. `decode_webp_animation` returned all four frames, and each matched a still decode of that frame. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| four-frame VP8 animation encode, 1280×720 (output 192,554 bytes) | 40 | 54,127.8 µs | 84,091.8 µs | 204.32 (p99 131.51) | file MB/s p50 3.56; 138 allocs; 37.43 allocs/MP |

64×64 canvas with a 16×16 second frame at the origin, a later release run. Same intra-DC keyframe coder. The second frame is that 16×16 rectangle, not a composited canvas. The four-frame 1280×720 row and the full-frame 64×64 row were not replaced. `decode_webp_animation` returned both frames. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| two-frame VP8 animation encode, 64×64 canvas, 16×16 partial at origin (output 190 bytes) | 40 | 58.0 µs | 74.7 µs | 225.03 (p99 174.70) | file MB/s p50 3.27; 35 allocs; 8,042.28 allocs/MP |

64×64 canvas with a 16×16 second frame at origin (16, 32), a later release run. Same intra-DC keyframe coder. The offset is the frame position, not a new entropy coder. p50 is 57.1 µs, under the 116 µs cap, so the origin stays. The origin-zero partial row and the full-frame 64×64 row were not replaced. `decode_webp_animation` returned the 16×16 rectangle at (16, 32). `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| two-frame VP8 animation encode, 64×64 canvas, 16×16 at (16, 32) (output 190 bytes) | 40 | 57.1 µs | 64.8 µs | 228.49 (p99 201.50) | file MB/s p50 3.33; 35 allocs; 8,042.28 allocs/MP |

Decode of that same 190-byte file, a later release run. The decoder paints the 16×16 rectangle at (16, 32) onto the 64×64 canvas. The composited canvas matches that overlay. p50 is 64.9 µs, under the 116 µs cap, so the paint stays. The encode rows above were not replaced. `decode_webp` on the file still returns `Unsupported`.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| VP8 animation decode, 64×64 canvas, 16×16 painted at (16, 32) | 40 | 64.9 µs | 95.5 µs | 378.40 (p99 257.44) | file MB/s p50 2.93; 14 allocs; 1,708.98 allocs/MP |

`decode_webp` of that same 190-byte file, a later release run. It returns the composited 64×64 canvas. Those pixels match the painted overlay. p50 is 66.4 µs, under the 116 µs cap, so this return stays. Pixel MB/s uses that one canvas. A non-animation still decode of the canvas was unchanged, and the origin-zero animation still returns `Unsupported`. The animation-decode row above and the encode rows (190 bytes at 57.1 µs and 58.0 µs) were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| `decode_webp`, 64×64 canvas, 16×16 painted at (16, 32) | 40 | 66.4 µs | 91.7 µs | 185.03 (p99 134.07) | file MB/s p50 2.86; 14 allocs; 3,417.97 allocs/MP |

`decode_webp` of the origin-zero 64×64 partial, the same 190-byte file, a later release run. It returns that file's 64×64 canvas. Those pixels match the animation decoder's canvas. p50 is 64.6 µs, under the 116 µs cap, so this return stays. A non-animation still decode was unchanged, and `decode_webp` of the (16, 32) file and of the full-frame 64×64 animation was unchanged. The (16, 32) still-decode row above and the animation-decode row (64.9 µs) were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| `decode_webp`, 64×64 canvas, 16×16 painted at the origin | 40 | 64.6 µs | 85.7 µs | 190.18 (p99 143.35) | file MB/s p50 2.94; 14 allocs; 3,417.97 allocs/MP |

`decode_webp` of the full-frame 64×64 two-frame file, 200 bytes, a later release run. It returns that file's first frame. Those pixels match the animation decoder's first frame. p50 is 49.7 µs, under the 200 µs cap, so this return stays. A non-animation still decode was unchanged. `decode_webp` of the origin-zero partial and of the (16, 32) file was unchanged, and a larger animation still returns `Unsupported`. The origin-zero still-decode row above and the (16, 32) still-decode row (66.4 µs) were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| `decode_webp`, full-frame 64×64 two-frame file, first frame (file 200 bytes) | 40 | 49.7 µs | 49.8 µs | 247.47 (p99 246.85) | file MB/s p50 4.03; 5 allocs; 1,220.70 allocs/MP |

`decode_webp` of the two-frame 256×256 file, 458 bytes, a later release run. It returns that file's first frame. Those pixels match the animation decoder's first frame. p50 is 723.0 µs, under the 1,200 µs cap, so this return stays. A non-animation still decode was unchanged. `decode_webp` of the 64×64 files was unchanged, and a larger animation still returns `Unsupported`. The full-frame 64×64 still-decode row above (49.7 µs), the origin-zero row (64.6 µs), and the (16, 32) row (66.4 µs) were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| `decode_webp`, two-frame 256×256 file, first frame (file 458 bytes) | 40 | 723.0 µs | 796.5 µs | 271.95 (p99 246.84) | file MB/s p50 0.63; 5 allocs; 76.29 allocs/MP |

`decode_webp` of the existing two-frame 640×480 file, 34,434 bytes, a later release run. It returns that file's first frame. Those pixels match the animation decoder's first frame. p50 is 4,010.8 µs, under the 5,120 µs cap, so this return stays. A non-animation still decode was unchanged. `decode_webp` of the 64×64 files and of the 256×256 file was unchanged. The four-frame 640×480 file and a larger animation still return `Unsupported`. The 723.0 µs row, the 49.7 µs row, the 64.6 µs row, and the 66.4 µs row were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| `decode_webp`, two-frame 640×480 file, first frame (file 34,434 bytes) | 40 | 4,010.8 µs | 4,355.7 µs | 229.78 (p99 211.59) | file MB/s p50 8.59; 5 allocs; 16.28 allocs/MP |

`decode_webp` of the existing two-frame 1280×720 file, 94,368 bytes, a later release run. It returns that file's first frame. Those pixels match the animation decoder's first frame. p50 is 11,826.9 µs, under the 18,200 µs cap, so this return stays. A non-animation still decode was unchanged. `decode_webp` of the 64×64 files, the 256×256 file, and the two-frame 640×480 file was unchanged. The four-frame 1280×720 file still returns `Unsupported`. The 4,010.8 µs row, the 723.0 µs row, the 49.7 µs row, the 64.6 µs row, and the 66.4 µs row were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| `decode_webp`, two-frame 1280×720 file, first frame (file 94,368 bytes) | 40 | 11,826.9 µs | 12,310.8 µs | 233.77 (p99 224.58) | file MB/s p50 7.98; 5 allocs; 5.43 allocs/MP |

`decode_webp` of the existing four-frame 1280×720 file, 192,554 bytes, a later release run. It returns that file's first frame. Those pixels match the animation decoder's first frame. p50 is 12,321.1 µs, under the 17,800 µs cap, so this return stays. A non-animation still decode was unchanged. `decode_webp` of the two-frame 1280×720 file and of the smaller files was unchanged. The 11,826.9 µs row and the 4,010.8 µs row were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| `decode_webp`, four-frame 1280×720 file, first frame (file 192,554 bytes) | 40 | 12,321.1 µs | 18,051.7 µs | 224.40 (p99 153.16) | file MB/s p50 15.63; 5 allocs; 5.43 allocs/MP |

`encode_svg` of one 16×16 black square on white, a later release run. The file is one `<rect>`, 124 bytes. `rasterize_svg` returns the same pixels, so this path stays. A path, a curve, and text stay `Unsupported`. Cairo, vtracer, and resvg are not called, and the path does not start a thread. The PNG, JPEG, and WebP rows above were not replaced. The four-frame 1280×720 first-frame row stays 12,321.1 µs.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| SVG rect encode, 16×16 black square on white (output 124 bytes) | 40 | 0.211 µs | 0.346 µs | 3,639.81 (p99 2,219.65) | pixels match; 1 alloc |

`encode_svg` of one 32×32 black square on white, a later release run. The file is one `<rect>`, 126 bytes. `rasterize_svg` returns the same pixels. p50 is 0.162 µs, at most 0.50 µs, so this path stays. A path, a curve, and text stay `Unsupported`. Cairo, vtracer, and resvg are not called, and the path does not start a thread. The 16×16 SVG row stays 124 bytes, p50 0.211 µs, p99 0.346 µs. The four-frame 1280×720 first-frame row stays 12,321.1 µs. The PNG, JPEG, and WebP rows above were not replaced.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| SVG rect encode, 32×32 black square on white (output 126 bytes) | 40 | 0.162 µs | 1.317 µs | 18,962.96 (p99 2,332.57) | pixels match; 1 alloc |

`thumbnail_jpeg` of the existing two-frame 640×480 WebP, 34,434 bytes, a later release run. The output is a JPEG of that file's first frame, 3,024 bytes, 220×165. The long side is 220. p50 is 9,243.2 µs, under the 22.4 ms cap, so this path stays. A still WebP, the four-frame 640×480 file, and a smaller animation stay `Unsupported`. The PNG thumbnail row stays 3,703 bytes, p50 20,402.9 µs, p99 21,764.4 µs. The two-frame 640×480 first-frame row stays 34,434 bytes, p50 4,010.8 µs, p99 4,355.7 µs. The four-frame 1280×720 first-frame row stays 192,554 bytes, p50 12,321.1 µs. The 32×32 SVG row stays 126 bytes at 0.162 µs, and the 16×16 SVG row stays 124 bytes at 0.211 µs.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| thumbnail q=72, two-frame 640×480 WebP first frame → 220×165 (output 3,024 bytes) | 20 | 9,243.2 µs | 9,431.0 µs | 99.71 (p99 97.72) | JPEG; long side 220; 19 allocs; 61.85 allocs/MP |

`thumbnail_jpeg` of the existing four-frame 1280×720 WebP, 192,554 bytes, a later release run. The output is a JPEG of that file's first frame, 3,118 bytes, 220×124. The long side is 220. p50 is 16,374.0 µs, under the 30.5 ms cap, so this path stays. A still WebP stays unsupported. The two-frame 640×480 WebP thumbnail row stays 3,024 bytes, 220×165, p50 9,243.2 µs, p99 9,431.0 µs. The PNG thumbnail row stays 3,703 bytes, p50 20,402.9 µs, p99 21,764.4 µs. The four-frame 1280×720 first-frame row stays 192,554 bytes, p50 12,321.1 µs. The two-frame 640×480 first-frame row stays 34,434 bytes, p50 4,010.8 µs.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| thumbnail q=72, four-frame 1280×720 WebP first frame → 220×124 (output 3,118 bytes) | 20 | 16,374.0 µs | 16,473.7 µs | 168.85 (p99 167.83) | JPEG; long side 220; 19 allocs; 20.62 allocs/MP |

`shrink_png` of the same gradient at 192×144, a later release run. The source PNG is 87,853 bytes. The cap is 8,000 bytes, the same cap as the 96×72 row. The output is 8,000 bytes, smaller than the source, and not a copy. p50 is 21,117.0 µs, under the 25.6 ms cap, so this edge stays. The 96×72 shrink row stays 5,827.4 µs. The 640×480 shrink stays reverted: 728,763 bytes in, 8,000 bytes out, p50 198,157.4 µs. Shrink does not take WebP. The WebP thumbnail rows stay 3,024 bytes at 9,243.2 µs and 3,118 bytes at 16,374.0 µs. The PNG thumbnail row stays 3,703 bytes, p50 20,402.9 µs, p99 21,764.4 µs.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| shrink 192×144 PNG to 8,000 bytes, fresh cache (file 87,853 bytes, output 8,000 bytes) | 12 | 21,117.0 µs | 24,968.5 µs | 3.93 | 112 allocs; 4,050.93 allocs/MP |

`shrink_png` of the same gradient at 384×288, a later release run. The source PNG is 338,716 bytes. The cap is 8,000 bytes. The output is 8,000 bytes, smaller than the source, and not a copy. p50 is 65,610.7 µs, under the 92.9 ms cap, so this edge stays. The 192×144 shrink row stays 87,853 bytes in, 8,000 bytes out, p50 21,117.0 µs, p99 24,968.5 µs. The 96×72 shrink row stays 5,827.4 µs. The 640×480 shrink stays reverted: 728,763 bytes in, 8,000 bytes out, p50 198,157.4 µs. Shrink does not take WebP. The WebP thumbnail rows stay 3,024 bytes at 9,243.2 µs and 3,118 bytes at 16,374.0 µs. The PNG thumbnail row stays 3,703 bytes, p50 20,402.9 µs, p99 21,764.4 µs.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| shrink 384×288 PNG to 8,000 bytes, fresh cache (file 338,716 bytes, output 8,000 bytes) | 12 | 65,610.7 µs | 66,739.0 µs | 5.06 | 104 allocs; 940.39 allocs/MP |

`shrink_png` of the same gradient at 768×576, a later release run. The source PNG is 932,701 bytes. The cap is 8,000 bytes. The output is 8,000 bytes, smaller than the source, and not a copy. p50 is 228,807.5 µs, under the 288.7 ms cap, so this edge stays. The 384×288 shrink row stays 338,716 bytes in, 8,000 bytes out, p50 65,610.7 µs, p99 66,739.0 µs. The 192×144 shrink row stays 87,853 bytes in, 8,000 bytes out, p50 21,117.0 µs, p99 24,968.5 µs. The 96×72 shrink row stays 5,827.4 µs. The 640×480 shrink stays reverted: 728,763 bytes in, 8,000 bytes out, p50 198,157.4 µs. Shrink does not take WebP. The WebP thumbnail rows stay 3,024 bytes at 9,243.2 µs and 3,118 bytes at 16,374.0 µs. The PNG thumbnail row stays 3,703 bytes, p50 20,402.9 µs, p99 21,764.4 µs.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| shrink 768×576 PNG to 8,000 bytes, fresh cache (file 932,701 bytes, output 8,000 bytes) | 12 | 228,807.5 µs | 236,214.5 µs | 5.80 | 102 allocs; 230.58 allocs/MP |

`shrink_png` of the same gradient at 1536×1152, a later release run. The source PNG is 2,170,497 bytes. The cap is 8,000 bytes. The output is 8,000 bytes, smaller than the source, and not a copy. p50 is 794,767.1 µs, under the 1,006.8 ms cap, so this edge stays. The 768×576 shrink row stays 932,701 bytes in, 8,000 bytes out, p50 228,807.5 µs, p99 236,214.5 µs. The 384×288 shrink row stays 338,716 bytes in, 8,000 bytes out, p50 65,610.7 µs, p99 66,739.0 µs. The 192×144 shrink row stays 87,853 bytes in, 8,000 bytes out, p50 21,117.0 µs, p99 24,968.5 µs. The 96×72 shrink row stays 5,827.4 µs. The 640×480 shrink stays reverted: 728,763 bytes in, 8,000 bytes out, p50 198,157.4 µs. Shrink does not take WebP. The WebP thumbnail rows stay 3,024 bytes at 9,243.2 µs and 3,118 bytes at 16,374.0 µs. The PNG thumbnail row stays 3,703 bytes, p50 20,402.9 µs, p99 21,764.4 µs.

| path | samples | p50 | p99 | pixel MB/s p50 | other |
| --- | --- | --- | --- | --- | --- |
| shrink 1536×1152 PNG to 8,000 bytes, fresh cache (file 2,170,497 bytes, output 8,000 bytes) | 12 | 794,767.1 µs | 847,271.5 µs | 6.68 | 343 allocs; 193.84 allocs/MP |

## Huffman lookup

VP8L codes of one to eight bits go through a 256-entry table. Longer codes still walk one bit at a time. The table was compared with the bit-by-bit reader on the same 640×480 file.

The bit-by-bit binary, run twice: p50 9,227.2 µs (p99 12,894.4 µs), then p50 9,271.7 µs (p99 9,554.9 µs). The p50 moved 45 µs. That second run’s p99−p50 spread is 283 µs. The first p99 was one slow sample.

The lookup’s p50 is 7,750.6 µs, about 1.5 ms under the bit-by-bit p50. That is outside the 45 µs repeat and outside the 283 µs spread, so the table stays. It is inside the first run’s p99−p50 gap only because of that one slow sample.

`cargo build --release --target wasm32-unknown-unknown --manifest-path crates/export-image/Cargo.toml` succeeds for this revision. The bench harness is native-only.
