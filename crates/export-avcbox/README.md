# export-avcbox

One Annex B H.264 I-frame, boxed as a non-fragmented MP4. `ftyp`, `moov`, and `mdat`. One `avc1` sample. No dependencies. Threading is off.

This is one boxed frame, not a shrink of a real video. It does not open a movie. It does not encode H.264. A truncated Annex B buffer returns an error. A P-slice before the I-frame, a B-slice, CABAC, an interlaced picture, and more than one slice group return an error. P-frames that follow the I-frame are further samples in the same file. The slice is walked only far enough to reject a cut-off buffer. Pixels are not reconstructed here. The library never calls ffmpeg.

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

## 160×160 sequence

Same release profile and one thread. The input is the 7,197-byte Annex B file: the 160×160 I-frame, then seven P-frames, copied in as bytes. Eight `avc1` samples. 200 calls after 20 warmup.

- input: 7,197 bytes
- output: 7,819 bytes
- release p50: 762 µs, p99: 944 µs on the run taken immediately after linking. Four later runs on that binary were p50 758–785 µs and p99 902–923 µs. One more run of that binary, after the wasm library build, was p50 967 µs and p99 1,361 µs

`scripts/check-identify-seq160.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 160×160, and decoded 8 frames.

## 320×240

Same release profile and one thread. The input is the 19,337-byte 320×240 Constrained Baseline I-frame, copied in as bytes. One `avc1` sample. Same level 1.1 slice as the smaller I-frames. 200 calls after 20 warmup.

- input: 19,337 bytes
- output: 19,953 bytes
- release p50: 1,833 µs, p99: 3,082 µs on the run taken immediately after linking. Four later runs on that binary were p50 1,807–1,981 µs and p99 2,016–3,044 µs

`scripts/check-identify-320.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 320×240, one frame.

## 320×240 and one P-frame

Same release profile and one thread. The input is the 29,090-byte Annex B file: the 320×240 I-frame, then one zero-motion P-frame, copied in as bytes. Same level 1.1 path. Two `avc1` samples. 200 calls after 20 warmup.

- input: 29,090 bytes
- output: 29,724 bytes
- release p50: 4,024 µs, p99: 5,398 µs on the run taken immediately after linking. Four later runs on that binary were p50 3,962–4,024 µs and p99 4,244–4,637 µs

`scripts/check-identify-320pair.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 320×240, and decoded 2 frames.

## Four 320×240 pictures

Same release profile and one thread. The input is the 29,559-byte Annex B file: the 320×240 I-frame, then three zero-motion P-frames of the same kind as the pair. Same level 1.1 path. The first two P-frames are references. The leading bytes are the 19,337-byte I-frame. Four `avc1` samples. 200 calls after 20 warmup.

- input: 29,559 bytes
- output: 30,189 bytes
- release p50: 3,983 µs, p99: 4,329 µs on the run taken immediately after linking. Four later runs on that binary were p50 3,959–3,979 µs and p99 4,206–4,882 µs

`scripts/check-identify-320four.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 320×240, and decoded 4 frames.

## Eight 320×240 pictures

Same release profile and one thread. The input is the 30,363-byte Annex B file: the 320×240 I-frame, then seven zero-motion P-frames of the same kind as the four-picture file. Same level 1.1 path. The first six P-frames are references. The leading bytes are the 19,337-byte I-frame. Eight `avc1` samples. 200 calls after 20 warmup.

- input: 30,363 bytes
- output: 30,985 bytes
- release p50: 3,984 µs, p99: 4,162 µs on the run taken immediately after linking. Four later runs on that binary were p50 3,974–4,062 µs and p99 4,139–6,588 µs

`scripts/check-identify-320eight.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 320×240, and decoded 8 frames.

## 1280×720

Same release profile and one thread. The input is the 232,774-byte 1280×720 Constrained Baseline I-frame, level 3.1, copied in as bytes. One `avc1` sample. The slice is the same I-slice the smaller frames already use. 200 calls after 20 warmup.

- input: 232,774 bytes
- output: 233,390 bytes
- release p50: 22,068 µs, p99: 24,769 µs on the run taken immediately after linking. Four later runs on that binary were p50 22,346–24,800 µs and p99 24,512–26,990 µs

`scripts/check-identify-720.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 1280×720, level 3.1, one frame.

## 1280×720 and one P-frame

Same release profile and one thread. The input is the 349,461-byte Annex B file: the 232,774-byte level 3.1 I-frame, then one zero-motion P-frame, copied in as bytes. Two `avc1` samples. 200 calls after 20 warmup.

- input: 349,461 bytes
- output: 350,095 bytes
- release p50: 48,312 µs, p99: 52,859 µs on the run taken immediately after linking. Four later runs on that binary were p50 48,397–49,320 µs and p99 54,825–57,840 µs

`scripts/check-identify-720pair.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 1280×720, level 3.1, and decoded 2 frames.

## Four 1280×720 pictures

Same release profile and one thread. The input is the 354,619-byte Annex B file: the level 3.1 I-frame, then three zero-motion P-frames of the same kind as the pair. The first two P-frames are references. The leading bytes are the 232,774-byte I-frame. Four `avc1` samples. 200 calls after 20 warmup.

- input: 354,619 bytes
- output: 355,249 bytes
- release p50: 48,009 µs, p99: 52,207 µs on the run taken immediately after linking. Four later runs on that binary were p50 48,129–49,769 µs and p99 50,993–59,786 µs

`scripts/check-identify-720four.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 1280×720, level 3.1, and decoded 4 frames.

## Eight 1280×720 pictures

Same release profile and one thread. The input is the 363,671-byte Annex B file: the level 3.1 I-frame, then seven zero-motion P-frames of the same kind as the four-picture file. The first six P-frames are references. The leading bytes are the 232,774-byte I-frame. Eight `avc1` samples. 200 calls after 20 warmup.

- input: 363,671 bytes
- output: 364,293 bytes
- release p50: 48,308 µs, p99: 52,866 µs on the run taken immediately after linking. Four later runs on that binary were p50 48,020–48,214 µs and p99 50,753–61,493 µs

`scripts/check-identify-720eight.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 1280×720, level 3.1, and decoded 8 frames.

## Sixteen 1280×720 pictures

Same release profile and one thread. The input is the 381,775-byte Annex B file: the level 3.1 I-frame, then fifteen zero-motion P-frames of the same kind as the eight-picture file. The first fourteen P-frames are references. The leading bytes are the 232,774-byte I-frame. Sixteen `avc1` samples. 200 calls after 20 warmup.

- input: 381,775 bytes
- output: 382,381 bytes
- release p50: 48,963 µs, p99: 58,534 µs on the run taken immediately after linking. Four later runs on that binary were p50 48,609–50,132 µs and p99 53,079–61,764 µs

`scripts/check-identify-720sixteen.sh` may run ffmpeg only to identify that file. On this machine ffmpeg reported `Video: h264` (Constrained Baseline), 1280×720, level 3.1, and decoded 16 frames.
