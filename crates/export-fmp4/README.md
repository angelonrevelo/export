# export-fmp4

Stream copy for fragmented MP4.

Each input is hashed once with SHA-256, sniffed as ISO BMFF, then the `moof` / `mdat` sample bytes are copied into a new fragmented file. H.264 (`avc1`–`avc4`), AAC (`mp4a`), Opus (`Opus`), and VP9 (`vp09`) are sniffed and return `unsupported`. They are not decoded.

The library does not call ffmpeg. `scripts/identify.sh` is a host check: ffmpeg must name the copied file as `mov,mp4,m4a,3gp,3g2,mj2`.

Threading is off. There is no parallel copy path.

## Limits

- A box must declare its size. A size of zero is invalid, so a short buffer cannot pretend to be finished. A size past the buffer is `truncated`.
- Sample counts are bounded by the file length before the sample table is allocated. A declared sample that runs past the buffer is `truncated`.
- A `trun` data offset has to fit in a signed 32-bit field.
- This is not a video shrink, and it does not flatten a non-fragmented MP4. A file with no `moof` is invalid.
- The generated file is one PCM (`ipcm`) track, two fragments, 1 MiB of sample bytes.
