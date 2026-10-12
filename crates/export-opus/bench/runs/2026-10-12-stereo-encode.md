# 2026-10-12 — stereo packet encode from PCM

`cargo bench --workspace --bench stereo` from `crates/export-opus`.

This file does not replace the 160-byte stereo decode row (left RMS 10376.7, right RMS 5107.9) or the mono third-packet encode rows (p50 34 ns and 31 ns). No speedup against those rows is claimed.

Threading off. Clock is `std::time::Instant`. Nearest-rank p50 and p99 over 20,000 calls after 1,000 warmup calls. Profile matches release: `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`.

Timed call: `encode_stereo_frame` of one 20 ms frame. Left is a half-scale 440 Hz sine. Right is the same sine at quarter scale. The packet is 160 bytes. Decode is not in this bench.

FFmpeg, from the host script only, still matches that packet: left RMS 10376.7, peak 17000; right RMS 5107.9, peak 8303. The library did not call ffmpeg.

## Runs on this machine

| run | stereo-encode p50 | stereo-encode p99 |
|---|---:|---:|
| 1 | 67 ns | 82 ns |
| 2 | 77 ns | 91 ns |

Encode p50 moved from 67 ns to 77 ns, and p99 from 82 ns to 91 ns. Treat that as timer noise, not a stable figure.
