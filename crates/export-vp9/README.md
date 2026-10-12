# export-vp9

This is not a video decoder.

It decodes one generated VP9 keyframe from an IVF file, either 8×8 or 16×16. The frame is profile 0, 8-bit, 4:2:0, and lossless. Prediction is DC. Each 8×8 block's first luma 4×4 carries a single coefficient; the other transforms are empty. The 16×16 frame is still one keyframe: it splits into four of those blocks. The loop filter is off.

FFmpeg decodes each of those files. The largest absolute sample difference is 0 on both. The 8×8 file is 66 bytes with a 22-byte payload, release p50 389 ns and p99 443 ns. The 16×16 file is 74 bytes with a 30-byte payload, release p50 1065 ns and p99 1165 ns. Those figures are in `docs/research/`.

A truncated buffer or a bad header returns an error. More than one frame, inter frames, and VP8 are unsupported, as are other sizes, segmentation, a loop filter, and probability updates. The library does not link FFmpeg, spawn it, or start threads. There are no dependencies.

```sh
cargo test
scripts/check-ffmpeg.sh
scripts/gate.sh
```
