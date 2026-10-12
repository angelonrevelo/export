# export-aac

One AAC-LC ADTS frame. Mono. 44.1 kHz. 1024 samples. Long sine window. No CRC.

This is not a song encoder. It does not write a stream, it does not keep
overlap from one call to the next, and it does not replace a multi-frame
FFmpeg encode of a song. HE-AAC, SBR, and parametric stereo return
`Error::Unsupported`. A truncated frame or a bad ADTS header returns an
error. The library does not link or spawn FFmpeg.

A single long window overlap-adds against a zero history. The half-scale
440 Hz frame therefore does not come back as that sine. Host FFmpeg 6.1.1
decodes the 513-byte frame as AAC-LC, 44100 Hz, mono, 1024 float samples.
This decoder and that decode differ by at most 0.00359388 in absolute
sample value. Both sit about 0.50 away from the original sine (full scale
is 1.0). The measured max abs error of this crate's round trip is
0.5003551. The locked bound is 0.501.

## One frame, release

Profile: `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`. Threading is
off. 200 calls of this one frame on this machine. Not a throughput number
for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 5.872 ms (5871704 ns) | 7.590 ms (7590356 ns) | 513 |
| truncated miss | 43 ns | 98 ns | 0 |

The miss is `decode_frame` of that frame with the last byte removed. It
returns `Error::Truncated` and no samples.

## Build

```
cargo build --release --manifest-path crates/export-aac/Cargo.toml
cargo build --release --manifest-path crates/export-aac/Cargo.toml --target wasm32-unknown-unknown --lib
```
