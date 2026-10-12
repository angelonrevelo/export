# export-mp3

Encode and decode **one** MPEG-1 Layer III frame: mono, 44.1 kHz, 128 kbps, 1152 samples.

This is not a song encoder. There is no bit reservoir, no ID3 tag, no second frame, and no multi-frame file writer.

## What the frame contains

- Silence (every sample `0.0`) is an empty Huffman region. Decoding it returns 1152 exact zeros.
- Any other finite block stores the first 16 quantized long-block MDCT lines of each granule (Huffman table 5, magnitudes 0..=3). Decoding runs requantization, alias reduction, the long-block IMDCT, and the polyphase synthesis filter, and returns 1152 PCM samples.

A half-scale 440 Hz sine (1152 samples at 44.1 kHz) encodes as those 16 lines. Decoding it in this crate does not return the sine. The sample-aligned max absolute error is 0.680283487 in both debug and release. ffmpeg still opens that frame only after the host repeats it. On that two-copy file the decode is 2304 samples, absolute peak 0.537770271, RMS 0.185432036, and the best absolute correlation with the sine is 0.750842570. That correlation is above 0.5. The sample-aligned error is 0.680283487, so the decoded block is not the sine and this is not a waveform encoder. A negative block decodes to the negation of the positive block.

Stereo, joint stereo, and dual channel return `Error::Unsupported`. A buffer shorter than the frame declared by the header returns `Error::Truncated`. An illegal sync, layer, bitrate, sample-rate, or emphasis field returns `Error::BadHeader`.

## Build

Zero dependencies. Edition 2021, `rust-version` 1.74, license MIT OR Apache-2.0. Release is `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`. The library does not spawn threads and does not use rustls.

```bash
cargo test
cargo build --release --target wasm32-unknown-unknown
./scripts/gate.sh
```

`scripts/ffmpeg-decode.sh` is a host check. The library never calls ffmpeg. ffmpeg 6.1 still will not open a one-frame file, including the 440 Hz sine frame: it seeks one frame ahead, fails with `Invalid frame size (417): Could not seek to 417` (exit 234), and decodes only after that same frame is repeated.

## Bench

`cargo run --release --example bench` prints nearest-rank p50 and p99. In `bench/runs/release.txt` the recorded silence row stays decode p50 948 ns, and the recorded truncated row stays decode p50 33 ns. The 515 µs row is the older count-1 pulse path. A release decode of the existing 16-line 440 Hz frame is p50 559231 ns and p99 581941 ns. That frame is not a waveform. The sample error is in `bench/runs/sine-440.txt`. These are measurements, not a speedup claim.
