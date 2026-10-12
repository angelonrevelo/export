# export-fmp4

Stream copy for fragmented MP4.

Each input is hashed once with SHA-256, sniffed as ISO BMFF, then the `moof` / `mdat` sample bytes are copied into a new fragmented file. AAC (`mp4a`), Opus (`Opus`), and VP9 (`vp09`) are sniffed and return `unsupported`. They are not decoded.

One fragmented file carries a single `avc1` sample. Those bytes are copied. `avcC` and the NAL payload are not parsed. `scripts/identify.sh` runs ffmpeg on that copy; ffmpeg names it `Video: h264 (Constrained Baseline)`, 16×16. Further files repeat that same access unit 47 times and 188 times in one fragment. The host script decodes them and requires ffmpeg to report 47 frames and 188 frames. The library does not call ffmpeg.

The same host script checks that the PCM copy is named `mov,mp4,m4a,3gp,3g2,mj2`.

Threading is off. There is no parallel copy path.

## Limits

- A box must declare its size. A size of zero is invalid, so a short buffer cannot pretend to be finished. A size past the buffer is `truncated`.
- Sample counts are bounded by the file length before the sample table is allocated. A declared sample that runs past the buffer is `truncated`.
- A `trun` data offset has to fit in a signed 32-bit field.
- This is not a video shrink, and it does not flatten a non-fragmented MP4. A file with no `moof` is invalid.
- The PCM file is one `ipcm` track, two fragments, 1 MiB of sample bytes.
- The H.264 file is one 16×16 `avc1` sample. It is a byte copy, not a decoder.
- The 47-sample file and the 188-sample file repeat that same access unit. They are still byte copies.
