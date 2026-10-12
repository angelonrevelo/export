# export-vp9

This is not a video decoder.

It decodes one generated 8×8 VP9 keyframe from an IVF file. The frame is profile 0, 8-bit, 4:2:0, and lossless. Prediction is DC. One luma 4×4 carries a single coefficient; the other transforms are empty. The loop filter is off.

FFmpeg decodes that same file. On this frame the largest absolute sample difference is 0. That bound is recorded in `docs/research/keyframe.md` with the file size and the release p50 and p99.

A truncated buffer or a bad header returns an error. More than one frame, inter frames, and VP8 are unsupported, as are other sizes, segmentation, a loop filter, and probability updates. The library does not link FFmpeg, spawn it, or start threads. There are no dependencies.

```sh
cargo test
scripts/check-ffmpeg.sh
scripts/gate.sh
```
