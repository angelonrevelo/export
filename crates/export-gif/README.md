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
