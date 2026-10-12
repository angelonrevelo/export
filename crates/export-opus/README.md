# export-opus

One Opus packet: mono, 48 kHz, one 20 ms frame.

This crate encodes these frames and decodes them back to PCM:

- Digital silence (960 zeros). The decoded frame is silence.
- A half-scale 440 Hz sine (960 samples, amplitude `0.5 * 32767`). FFmpeg decodes that packet to a tone, not to silence: RMS 10802.8, peak 16826. The start of the reconstructed frame is near zero (onset peak 4 over the first 8 samples) because of the frame window; the rest of the frame is the tone. Those figures are the ffmpeg 6.1.1 decode checked by `scripts/verify-ffmpeg.py`.
- A second 20 ms frame of that same sine, as the next packet. FFmpeg decodes the two-frame file to 1920 samples. The second frame is a tone, not near silence: RMS 11596.1, peak 17959, onset peak 15968. The same second packet with no frame before it is quieter (RMS 1767.0, peak 7524). That lone decode is what `decode` returns for the second packet.
- A third 20 ms packet of that same sine. In the three-frame file, ffmpeg's third frame is RMS 11606.2, peak 18866, onset peak 16570. The same packet with no frames before it is RMS 1455.1, peak 3528, onset peak 22. Those two decodes differ in all 960 samples. `decode` returns the lone-packet result.
- One stereo 20 ms packet at 48 kHz, 160 bytes. FFmpeg sees two channels (`48000 Hz, stereo`). `decode` still returns `unsupported` for stereo, including this packet. This is not a stereo encoder.

Any other PCM is `unsupported`. A truncated packet or a bad TOC byte returns an error. Stereo, multistream, and FEC return `unsupported`.

It is not a song encoder. It does not replace yt-dlp.

The library does not call ffmpeg. A host script may call ffmpeg only to decode the packet this crate wrote and compare samples.

```sh
cargo test
cargo bench --bench packet
cargo bench --bench sine
cargo bench --bench sine-third
cargo build --release --target wasm32-unknown-unknown --lib
python3 scripts/verify-ffmpeg.py
```

The silence bench is single-threaded (threading off) and prints p50 and p99 for the silence packet, including the truncated-packet error. The silence encode row (p50 350 ns) and the first sine encode rows (p50 52 ns and 49 ns) are unchanged. `cargo bench --bench sine` times only the first half-scale 440 Hz packet. `cargo bench --bench sine-third` times encode of the third 20 ms packet only.

License: MIT OR Apache-2.0.
