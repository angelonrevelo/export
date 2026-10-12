# export-mp3

Encode and decode **one** MPEG-1 Layer III frame: mono, 44.1 kHz, 128 kbps, 1152 samples.

This is not a song encoder. There is no bit reservoir, no ID3 tag, no second frame, and no multi-frame file writer.

## What the frame contains

- Silence (every sample `0.0`) is an empty Huffman region. Decoding it returns 1152 exact zeros.
- Any other finite block is one count-1 pulse per granule (`±1` on the first spectral line, `global_gain` taken from the peak). Decoding runs requantization, the long-block IMDCT, and the polyphase synthesis filter, and returns 1152 PCM samples.

The pulse is not a reconstruction of the input waveform. A negative block decodes to the negation of the positive block at the same peak. That is the whole signal model.

Stereo, joint stereo, and dual channel return `Error::Unsupported`. A buffer shorter than the frame declared by the header returns `Error::Truncated`. An illegal sync, layer, bitrate, sample-rate, or emphasis field returns `Error::BadHeader`.

## Build

Zero dependencies. Edition 2021, `rust-version` 1.74, license MIT OR Apache-2.0. Release is `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`. The library does not spawn threads and does not use rustls.

```bash
cargo test
cargo build --release --target wasm32-unknown-unknown
./scripts/gate.sh
```

`scripts/ffmpeg-decode.sh` is a host check. The library never calls ffmpeg. ffmpeg 6.1 will not open a file that ends on this frame (it seeks one frame ahead and must read the next header). The script repeats the frame so the demuxer can lock, then decodes it.

## Bench

`cargo run --release --example bench` prints nearest-rank p50 and p99 for silence decode, pulse decode, and the truncated-frame error. Numbers from a real run live in `bench/runs/`. They are timings, not a speedup claim.
