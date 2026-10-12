# export-h264

One baseline H.264 I-frame. 4:2:0, one slice, one QP, Annex B. No dependencies. Threading is off.

This is not a video shrink of a real MP4. It does not open a movie, does not write P-frames or B-frames, and does not run a second pass. Those requests return an error. It does not link or spawn ffmpeg.

Chroma residual is the DC coefficient of each 4×4 block. Luma residual is a full 4×4 transform. Deblocking is off so a decoder's pixels are the inverse transform.

## One frame

Release build (`opt-level = 3`, `lto = "thin"`, `codegen-units = 1`), generated 32×32 fixture, QP 26, one thread. The timed loop is 200 calls after 20 warmup.

- output: 272 bytes
- release p50: 11 µs (five runs, p99 11–18 µs). One process started immediately after linking measured p50 20 µs.
- mean absolute error, ffmpeg decode versus the source, every Y, U, and V sample: 2.4974 (Y 1.9961, U 3, V 4)

That decode matches the encoder reconstruction. `scripts/check-decode.sh` is a host check. It may run ffmpeg only to decode the Annex B file the `iframe` binary already wrote.
