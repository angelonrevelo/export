# export-mp3

Encode and decode **one** MPEG-1 Layer III frame: mono, 44.1 kHz, 128 kbps, 1152 samples.

This is not a song encoder. There is no bit reservoir, no ID3 tag, no second frame, and no multi-frame file writer.

## What the frame contains

- Silence (every sample `0.0`) is an empty Huffman region. Decoding it returns 1152 exact zeros.
- Any other finite block stores the first 32 quantized long-block MDCT lines of each granule, indices 0 through 31 (Huffman table 5, magnitudes 0..=3). Decoding runs requantization, alias reduction, the long-block IMDCT, and the polyphase synthesis filter, and returns 1152 PCM samples.

A half-scale 440 Hz sine (1152 samples at 44.1 kHz) encodes as those 32 lines. Decoding it in this crate does not return the sine. The sample-aligned max absolute error is 0.679248989 in both debug and release. The recorded 16-line error stays 0.680283487, and that frame's two-copy correlation stays 0.750842570. On the two-copy file ffmpeg opens for the 32-line frame, the decode has absolute peak 0.539853394, RMS 0.186021237, and best absolute correlation 0.749146499. The sample-aligned error is not the sine, so this is not a waveform encoder. A negative block decodes to the negation of the positive block.

Stereo, joint stereo, and dual channel return `Error::Unsupported`. A buffer shorter than the frame declared by the header returns `Error::Truncated`. An illegal sync, layer, bitrate, sample-rate, or emphasis field returns `Error::BadHeader`.

## Build

Zero dependencies. Edition 2021, `rust-version` 1.74, license MIT OR Apache-2.0. Release is `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`. The library does not spawn threads and does not use rustls.

```bash
cargo test
cargo build --release --target wasm32-unknown-unknown
./scripts/gate.sh
```

`scripts/ffmpeg-decode.sh` is a host check. The library never calls ffmpeg. A bare 417-byte file still fails ffmpeg 6.1 with exit 234 (`Invalid frame size (417): Could not seek to 417`). [`append_ffmpeg_demuxer_suffix`] writes 421 bytes (frame plus four repeated sync bytes). That is not a second spectral frame. ffmpeg opens it with exit 0. See `bench/runs/ffmpeg-openable-32line.txt`. Two full copies of the frame remain another host option.

## Bench

`cargo run --release --example bench` prints nearest-rank p50 and p99. In `bench/runs/release.txt` the recorded silence row stays decode p50 948 ns, and the recorded truncated row stays decode p50 33 ns. The 515 µs row is the older count-1 pulse path. The recorded 16-line 440 Hz decode stays p50 559231 ns and p99 581941 ns. A release decode of lines 0 through 31 is p50 462272 ns and p99 595640 ns. That is not several times the 16-line row, and it is not a waveform. The 16-line sample error stays in `bench/runs/sine-440.txt`. The 32-line error is in `bench/runs/sine-440-32line.txt`. These are measurements, not a speedup claim.
