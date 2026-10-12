# export-opus

One Opus packet: mono, 48 kHz, one 20 ms frame of digital silence (960 zero PCM samples).

This crate encodes that frame and decodes it back to PCM. A truncated packet or a bad TOC byte returns an error. Stereo, multistream, and FEC return `unsupported`. PCM other than silence does too: this crate carries one packet, not an arbitrary waveform.

It is not a song encoder. It does not replace yt-dlp.

The library does not call ffmpeg. A host script may call ffmpeg only to decode the packet this crate wrote and compare samples.

```sh
cargo test
cargo bench --bench packet
cargo build --release --target wasm32-unknown-unknown --lib
```

The bench is single-threaded (threading off) and prints p50 and p99, including the truncated-packet error.

License: MIT OR Apache-2.0.
