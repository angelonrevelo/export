# export-h264

One baseline H.264 I-frame, and one P-frame after it. 4:2:0, one slice, one QP, Annex B. No dependencies. Threading is off.

This is not a video shrink of a real MP4. It does not open a movie. A lone P-frame, a B-frame, and a second pass return an error. The measured P-frame is the second picture, motion zero, one reference (the I-frame reconstruction). A separate shifted picture uses one non-zero 16×16 vector. Eight pictures are that I-frame plus seven P-frames, each predicted from the previous reconstruction. A separate I-frame is 160×160 at the same profile and QP. A zero-motion P-frame can follow it, one shifted picture can use quarter-pel motion (16, 0), and eight pictures can chain those choices from the previous reconstruction. Sixteen 160×160 pictures use that same chain. One I-frame can also be 320×240. It does not link or spawn ffmpeg.

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

## 160×160

Same release profile, QP 26, one slice, one thread. The picture is 100 macroblocks, so the level byte is 1.1. The 32×32 files stay on level 1.0. 200 calls after 20 warmup.

- output: 6432 bytes
- release p50: 202 µs, p99: 248 µs on the recorded run. Four later runs on that binary were p50 203–204 µs and p99 243–324 µs
- mean absolute error, ffmpeg decode versus the source: 3.3912 (Y 2.0368, U 3.0250, V 9.1750)

The ffmpeg decode matches the encoder reconstruction. `scripts/check-160.sh` may run ffmpeg only to decode that file. Chroma residual is still only the DC coefficient, which is why V is the large part of the error.

## 160×160 and one P-frame

Same release profile and QP. The I-frame bytes are the 6,432-byte picture above. The P-frame is a brighter copy, motion zero, and its only reference is that I-frame's reconstruction. 200 calls after 20 warmup.

- output: 9689 bytes
- release p50: 450 µs, p99: 544 µs on the recorded run. Four later runs on that binary were p50 451–468 µs and p99 530–972 µs
- mean absolute error, ffmpeg decode versus both sources: 3.3958. The I-frame half is still 3.3912. The P-frame half is 3.4004

`scripts/check-pframe160.sh` may run ffmpeg only to decode that two-picture file.

## 160×160 and one motion vector

Same release profile and QP. The I-frame bytes are the 6,432-byte picture above. The P-frame is that fixture moved four luma samples left, predicted with one 16×16 vector, quarter-pel (16, 0), on every macroblock. 200 calls after 20 warmup.

- output: 6694 bytes
- release p50: 376 µs, p99: 460 µs on the recorded run. Four later runs on that binary were p50 374–381 µs and p99 449–716 µs
- mean absolute error, ffmpeg decode versus both sources: 3.3751. The I-frame half is still 3.3912. The P-frame half is 3.3590

The ffmpeg decode matches the encoder reconstruction. `scripts/check-motion160.sh` may run ffmpeg only to decode that file.

## Eight 160×160 pictures

Same release profile and QP. The I-frame bytes are the 6,432-byte picture above, then seven P-frames. Each P-frame is predicted from the previous reconstruction. The first, third, fifth, and seventh hold still. The second, fourth, and sixth move four luma samples and use quarter-pel motion (16, 0). 200 calls after 20 warmup.

- output: 7197 bytes
- release p50: 1383 µs, p99: 1545 µs on the recorded run. Four later runs on that binary were p50 1471–1550 µs and p99 1713–3054 µs
- mean absolute error, ffmpeg decode versus all eight sources: 3.3421. The I-frame half is still 3.3912. The seven P-frames are 3.3846, 3.3598, 3.3598, 3.3247, 3.3247, 3.2958, and 3.2958

The ffmpeg decode matches the encoder reconstruction. `scripts/check-sequence160.sh` may run ffmpeg only to decode that file.

## Sixteen 160×160 pictures

Same profile and QP. One I-frame and fifteen P-frames. The hold-and-shift pattern of the eight-picture file continues. The I-frame bytes are still the 6,432-byte picture. 200 calls after 20 warmup.

- output: 7814 bytes
- release p50: 2632 µs on the recorded run. Three later runs on that binary were 2593–2704 µs

ffmpeg decoded all 16 pictures and matched the encoder reconstruction. Mean absolute error versus the sources was 3.2864. The I-frame half is still 3.3912. `scripts/check-sixteen160.sh` may run ffmpeg only to decode that file.

## 320×240

Same Constrained Baseline profile, QP 26, one slice, one I-frame. 320×240 is 300 macroblocks, so the level byte is 1.1. 200 calls after 20 warmup.

- output: 19337 bytes
- release p50: 652 µs on the recorded run. Three later runs on that binary were 660–690 µs

ffmpeg decoded the picture and matched the encoder reconstruction. `scripts/check-320.sh` may run ffmpeg only to decode that file.
