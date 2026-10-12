# export-vp9

This is not a video decoder.

It decodes one generated VP9 keyframe from an IVF file: 8×8, 16×16, 32×32, or 64×64. The frame is profile 0, 8-bit, 4:2:0, and lossless. Prediction is DC. A coded 8×8 block's first luma 4×4 carries a single coefficient; the other transforms are empty. The 16×16 frame is still one keyframe: it splits into four of those blocks. The 32×32 frame is one keyframe too: four blocks are coded and twelve are skipped. The 64×64 frame is one keyframe: four blocks are coded and the rest are skipped. The loop filter is off.

FFmpeg decodes each of those files. The largest absolute sample difference is 0 on each. The 8×8 file is 66 bytes with a 22-byte payload, release p50 389 ns and p99 443 ns. The 16×16 file is 74 bytes with a 30-byte payload, release p50 1065 ns and p99 1165 ns. The 32×32 file is 81 bytes with a 37-byte payload, release p50 1870 ns and p99 3583 ns. The 64×64 file is 100 bytes with a 56-byte payload, release p50 5051 ns and p99 7457 ns. Those figures are in `docs/research/`.

A truncated buffer or a bad header returns an error. More than one frame, inter frames, and VP8 are unsupported, as are other sizes, segmentation, a loop filter, and probability updates. The library does not link FFmpeg, spawn it, or start threads. There are no dependencies.

```sh
cargo test
scripts/check-ffmpeg.sh
scripts/gate.sh
```
