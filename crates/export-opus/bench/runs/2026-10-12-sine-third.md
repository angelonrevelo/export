# 2026-10-12 — third sine packet encode

`cargo bench --workspace --bench sine-third` from `crates/export-opus`.

This file does not replace the silence encode row (p50 350 ns) or the first sine encode rows (p50 52 ns and 49 ns). No speedup against those rows is claimed.

Threading off. Clock is `std::time::Instant`. Nearest-rank p50 and p99 over 20,000 calls after 1,000 warmup calls. Profile matches release: `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`.

Timed call: encode of the third 20 ms half-scale 440 Hz packet only. Decode is not in this bench. There is no fourth frame.

FFmpeg figures for that packet stay as recorded: third frame in the three-frame file RMS 11606.2, lone packet RMS 1455.1. The library did not call ffmpeg.

## Runs on this machine

| run | third-encode p50 | third-encode p99 |
|---|---:|---:|
| 1 | 34 ns | 36 ns |
| 2 | 31 ns | 34 ns |

Encode p99 moved from 36 ns to 34 ns. Treat that as timer noise, not a stable figure.
