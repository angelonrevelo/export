# export-aac

One AAC-LC ADTS frame. Mono. 44.1 kHz. 1024 samples. Long sine window. No CRC.

This is not a song encoder. `encode_frame` does not keep overlap from one
call to the next, and these four frames do not replace a multi-frame FFmpeg
encode of a song. HE-AAC, SBR, and parametric stereo return
`Error::Unsupported`. A truncated frame or a bad ADTS header returns an
error. The library does not link or spawn FFmpeg.

A single long window overlap-adds against a zero history. The half-scale
440 Hz frame therefore does not come back as that sine. Host FFmpeg 6.1.1
decodes the 513-byte frame as AAC-LC, 44100 Hz, mono, 1024 float samples.
This decoder and that decode differ by at most 0.00000011 in absolute
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

## Second frame, release

The second frame is the same mono AAC-LC long window. Its left half is the
first frame's PCM. Its right half is the next 1024 samples of the same
half-scale 440 Hz sine. Overlap-add against the first frame's history emits
that first sine, one frame late. It is not near silence: the peak is
0.5006469, and the sine's peak is 0.5.

Max abs error versus the sine is 0.0018240213. The locked bound is 0.002.
Error versus the second frame's own samples is 0.63019896, which is the
one-frame delay. Host FFmpeg 6.1.1 decodes the two-frame ADTS file as
AAC-LC, 44100 Hz, mono, 2048 float samples. That second frame matches this
decoder within 0.00000012.

Same profile, 200 calls. Not a throughput number for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 6.007 ms (6006500 ns) | 7.073 ms (7073108 ns) | 68 |

## Third frame, release

The third frame is the same mono AAC-LC long window. Its left half is the
second frame's PCM. Its right half is the next 1024 samples of the same
half-scale 440 Hz sine. Overlap-add against the second frame's history is
still one frame late: it emits the second frame's sine, not its own input.
Max abs error versus that previous sine is 0.0008997023. Error versus its
own samples is 0.62984324. Peak is 0.50012225.

Host FFmpeg 6.1.1 decodes the three-frame ADTS file as AAC-LC, 44100 Hz,
mono, 3072 float samples. That third frame matches this decoder within
0.00000015.

Same profile, 200 calls. Not a throughput number for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 6.041 ms (6040873 ns) | 8.329 ms (8329276 ns) | 94 |

## Fourth frame, release

The fourth frame is the same mono AAC-LC long window. Its left half is the
third frame's PCM. Its right half is the next 1024 samples of the same
half-scale 440 Hz sine. Overlap-add against the third frame's history is
still one frame late: it emits the third frame's sine, not its own input.

Max abs error versus that delayed sine is 0.0012165308. Max abs error versus
this frame's own samples is 0.6303679.

Same profile, 200 calls. Not a throughput number for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 6.402 ms (6402279 ns) | 7.589 ms (7588843 ns) | 60 |

Decode is `decode_fourth_frame` on that same 60-byte frame. The call also reads the earlier three frames so the overlap tail exists. It is still one frame late. 200 calls, same profile.

| | p50 | p99 |
| --- | --- | --- |
| decode | 20.838 ms (20837888 ns) | 22.949 ms (22949440 ns) |

## Build

```
cargo build --release --manifest-path crates/export-aac/Cargo.toml
cargo build --release --manifest-path crates/export-aac/Cargo.toml --target wasm32-unknown-unknown --lib
```
