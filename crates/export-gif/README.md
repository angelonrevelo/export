# export-gif

GIF87a and GIF89a LZW, with no dependencies. The crate sniffs the magic, hashes the file once, dedups on that SHA-256, and only then decodes. Threading is off.

Video-to-GIF is refused. The function returns an error, does not decode the container, and does not call ffmpeg.

## Limits

- No video decoder and no ffmpeg. Video-to-GIF stays an error.
- Indexed round-trip does not quantize 24-bit color. The RGB path is a uniform 3-3-2 palette. Absolute error is bounded by 18, 18, and 42. An adaptive palette with no bound is not claimed.
- Plain-text extensions are skipped, not drawn.
- Color-resolution and sort bits in the screen descriptor are not preserved.
- A truncated stream or a bad LZW code is an error.
- Release profile is `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`.

## Release bench

`cargo bench --manifest-path crates/export-gif/Cargo.toml --bench throughput`, bench profile (`opt-level = 3`, `lto = "thin"`, `codegen-units = 1`). Fixture is four 128×96 frames, 49,152 indexed pixels, cache-hot. Not a large file.

| path | samples | p50 | p99 | pixel MB/s p50 | note |
|---|---:|---:|---:|---:|---|
| decode | 40 | 114.6 µs | 231.0 µs | 428.89 | bar was 80 MB/s, hit |
| encode | 20 | 302.2 µs | 323.9 µs | 162.62 | |
| video-to-GIF | 80 | 0.024 µs | 0.036 µs | | **miss**: unsupported, 0 pixels |

Decode p99 is 212.79 pixel MB/s. Encode p99 is 151.77 pixel MB/s. The 80 MB/s decode bar was set in the bench before the run. Video-to-GIF is the miss: it returns `video to gif is unsupported` and does not decode the container.

## RGB quantize

`cargo bench --manifest-path crates/export-gif/Cargo.toml --bench quantize`. One generated 96×64 frame, 6,144 RGB pixels. The path quantizes, writes GIF89a, and decodes before scoring. It does not remeasure the LZW bench above.

| | |
|---|---|
| p50 | 78.7 µs |
| p99 | 93.9 µs |
| MAE R, G, B | 9.070, 9.070, 21.164 |
| max abs on the fixture | 18, 18, 42 |
| bound | 18, 18, 42 |

Red and green share the same eight levels, and this fixture visits every byte equally on both, so those two means match. The bound is every channel value 0..=255, not only this image. A truncated quantize file still errors.

The 96×64 row is a small fixture. The same generator at 640×480, same 3-3-2 path, `cargo bench --manifest-path crates/export-gif/Cargo.toml --bench quantize_vga`. 307,200 pixels, 20 samples.

| | |
|---|---|
| p50 | 5333.5 µs |
| p99 | 5494.2 µs |
| MAE R, G, B | 9.070, 9.070, 21.164 |
| max abs | 18, 18, 42 |
| bound | holds |

Both sizes divide the byte range evenly, so the means match the small fixture. The bound still holds. Video-to-GIF is not this path.

## Four-frame encode

`cargo bench --manifest-path crates/export-gif/Cargo.toml --bench quantize_anim`. One GIF89a, four generated 256×256 frames, existing 3-3-2 palette and the existing LZW writer. Not a new codec. The 640×480 row and the indexed four-frame 128×96 row were not remeasured.

| | |
|---|---|
| output bytes | 45,447 |
| p50 | 2519.3 µs |
| p99 | 2817.2 µs |

640×480 has about 4.7 times the pixels of 256×256, so the same encode should sit near 11,800 µs. `cargo bench --manifest-path crates/export-gif/Cargo.toml --bench quantize_anim_vga` is one GIF89a, four generated 640×480 frames, same 3-3-2 palette and LZW writer. The 256×256 row and the 640×480 still row were not remeasured.

| | |
|---|---|
| output bytes | 209,641 |
| p50 | 12348.5 µs |
| p99 | 17947.2 µs |

That p50 is about 1.05 times the linear estimate, so this size stays. Video-to-GIF still returns an error. The library does not call ffmpeg.

## Eight-frame encode

Four frames at 256×256 are 2519.3 µs, so eight frames should sit near 5,039 µs. `cargo bench --manifest-path crates/export-gif/Cargo.toml --bench quantize_anim8` is one GIF89a, eight generated 256×256 frames, same 3-3-2 palette and LZW writer. The four-frame 256×256 row and the four-frame 640×480 row were not remeasured.

| | |
|---|---|
| output bytes | 92,296 |
| p50 | 5055.2 µs |
| p99 | 5134.4 µs |

That p50 is about 1.00 times the linear estimate, so this frame count stays. Video-to-GIF still returns an error. The library does not call ffmpeg.

## Sixteen-frame encode

Eight frames at 256×256 are 5055.2 µs, so sixteen frames should sit between 7,583 µs and 11,121 µs. `cargo bench --manifest-path crates/export-gif/Cargo.toml --bench quantize_anim16` is one GIF89a, sixteen generated 256×256 frames, same 3-3-2 palette and LZW writer. The file round-trips. The eight-frame row and the four-frame 256×256 row were not remeasured.

| | |
|---|---|
| output bytes | 181,739 |
| p50 | 10351.6 µs |
| p99 | 10955.9 µs |

That p50 is inside the window, so this frame count stays. Video-to-GIF still returns an error. The library does not call ffmpeg.

## Thirty-two-frame encode

Sixteen frames at 256×256 are 10351.6 µs, so thirty-two frames should sit between 15,527 µs and 22,774 µs. `cargo bench --manifest-path crates/export-gif/Cargo.toml --bench quantize_anim32` is one GIF89a, thirty-two generated 256×256 frames, same 3-3-2 palette and LZW writer. The file round-trips. The sixteen-frame row and the eight-frame row were not remeasured.

| | |
|---|---|
| output bytes | 362,663 |
| p50 | 21020.9 µs |
| p99 | 21511.0 µs |

That p50 is inside the window, so this frame count stays. Video-to-GIF still returns an error. The library does not call ffmpeg.
