# export-opus

One Opus packet: mono, 48 kHz, one 20 ms frame.

This crate encodes two of those frames and decodes them back to PCM:

- Digital silence (960 zeros). The decoded frame is silence.
- A half-scale 440 Hz sine (960 samples, amplitude `0.5 * 32767`). FFmpeg decodes that packet to a tone, not to silence: RMS 10802.8, peak 16826. The start of the reconstructed frame is near zero (onset peak 4 over the first 8 samples) because of the frame window; the rest of the frame is the tone. Those figures are the ffmpeg 6.1.1 decode checked by `scripts/verify-ffmpeg.py`.

Any other PCM is `unsupported`. A truncated packet or a bad TOC byte returns an error. Stereo, multistream, and FEC return `unsupported`.

It is not a song encoder. It does not replace yt-dlp.

The library does not call ffmpeg. A host script may call ffmpeg only to decode the packet this crate wrote and compare samples.

```sh
cargo test
cargo bench --bench packet
cargo build --release --target wasm32-unknown-unknown --lib
python3 scripts/verify-ffmpeg.py
```

The bench is single-threaded (threading off) and prints p50 and p99 for the silence packet, including the truncated-packet error. That silence encode row is unchanged by the sine frame.

License: MIT OR Apache-2.0.
