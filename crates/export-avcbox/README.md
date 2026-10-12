# export-avcbox

One Annex B H.264 I-frame, boxed as a non-fragmented MP4. `ftyp`, `moov`, and `mdat`. One `avc1` sample. No dependencies. Threading is off.

This is one boxed frame, not a shrink of a real video. It does not open a movie. It does not encode H.264. A truncated Annex B buffer returns an error. A P-slice, a B-slice, CABAC, an interlaced picture, and more than one slice group return an error. The slice is walked only far enough to reject a cut-off buffer. Pixels are not reconstructed here. The library never calls ffmpeg.

The 272-byte input is the 32×32 Constrained Baseline I-frame, copied in as a byte array.

## One frame

Release build (`opt-level = 3`, `lto = "thin"`, `codegen-units = 1`), one thread. The timed loop is 200 calls after 20 warmup.

- input: 272 bytes
- output: 888 bytes
- release p50: 34 µs, p99: 48 µs on the run taken immediately after linking. Four later runs on that binary were p50 31–33 µs and p99 34–44 µs

`scripts/check-identify.sh` is a host check. It may run ffmpeg only to identify the MP4 the `avcbox` binary already wrote. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 32×32, one frame. `scripts/gate.sh` is clippy (`-D warnings`), tests, and `cargo build --target wasm32-unknown-unknown --release --lib`. That wasm library build finished on this machine.

## 160×160

Same release profile and one thread. The input is the 6,432-byte 160×160 I-frame, copied in as bytes. One `avc1` sample. 200 calls after 20 warmup.

- input: 6,432 bytes
- output: 7,048 bytes
- release p50: 611 µs, p99: 774 µs on the run taken immediately after linking. Four later runs on that binary were p50 653–974 µs and p99 727–1,294 µs

`scripts/check-identify-160.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 160×160, one frame.
