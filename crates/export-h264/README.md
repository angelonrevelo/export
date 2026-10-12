# export-h264

One baseline H.264 I-frame, and one P-frame after it. 4:2:0, one slice, one QP, Annex B. No dependencies. Threading is off.

This is not a video shrink of a real MP4. It does not open a movie. A lone P-frame, a B-frame, and a second pass return an error. The measured P-frame is the second picture, motion zero, one reference (the I-frame reconstruction). A separate shifted picture uses one non-zero 16×16 vector. Eight pictures are that I-frame plus seven P-frames, each predicted from the previous reconstruction. It does not link or spawn ffmpeg.

Chroma residual is the DC coefficient of each 4×4 block. Luma residual is a full 4×4 transform. Deblocking is off so a decoder's pixels are the inverse transform.

## One frame

Release build (`opt-level = 3`, `lto = "thin"`, `codegen-units = 1`), generated 32×32 fixture, QP 26, one thread. The timed loop is 200 calls after 20 warmup.

- output: 272 bytes
- release p50: 11 µs (five runs, p99 11–18 µs). One process started immediately after linking measured p50 20 µs.
- mean absolute error, ffmpeg decode versus the source, every Y, U, and V sample: 2.4974 (Y 1.9961, U 3, V 4)

That decode matches the encoder reconstruction. `scripts/check-decode.sh` is a host check. It may run ffmpeg only to decode the Annex B file the `iframe` binary already wrote.

## Two frames

Same release profile and QP, the 32×32 fixture then a brighter copy, motion zero, one thread. 200 calls after 20 warmup. The I-frame bytes are the 272-byte picture above.

- output: 415 bytes
- release p50: 18 µs, p99: 21 µs on the recorded run. Five later runs on that binary were p50 18–21 µs and p99 23–41 µs
- mean absolute error, ffmpeg decode versus both sources: 2.5098. The I-frame half is still 2.4974. The P-frame half is 2.5221

`scripts/check-pframe.sh` may run ffmpeg only to decode that two-picture file.

## One motion vector

Same release profile and QP. The I-frame is the 272-byte picture above. The P-frame is the fixture moved four luma samples left, predicted with one 16×16 vector, quarter-pel (16, 0). That is four luma samples and two chroma samples, shared by every macroblock. 200 calls after 20 warmup.

- output: 304 bytes
- release p50: 17 µs, p99: 21 µs on the recorded run. Four later runs on that binary were p50 17 µs and p99 17–21 µs
- mean absolute error, ffmpeg decode versus both sources: 2.4368. The I-frame half is still 2.4974. The P-frame half is 2.3763

The ffmpeg decode matches the encoder reconstruction. `scripts/check-motion.sh` may run ffmpeg only to decode that file.

## Eight pictures

Same release profile and QP. The I-frame is the 272-byte picture above, then seven P-frames. Each P-frame is predicted from the previous reconstruction. The first, third, fifth, and seventh hold still. The second, fourth, and sixth move four luma samples and use quarter-pel motion (16, 0). 200 calls after 20 warmup.

- output: 405 bytes
- release p50: 52 µs, p99: 72 µs on the recorded run. Four later runs on that binary were p50 52–104 µs and p99 71–146 µs
- mean absolute error, ffmpeg decode versus all eight sources: 2.3146. The I-frame half is still 2.4974. The seven P-frames are 2.4909, 2.3763, 2.3763, 2.2409, 2.2409, 2.1471, and 2.1471

The ffmpeg decode matches the encoder reconstruction. `scripts/check-sequence.sh` may run ffmpeg only to decode that file.
