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

Decode is `decode_fourth_frame` on that same 60-byte frame. The call also
reads the earlier three frames so the overlap tail exists. It is still one
frame late. 200 calls, same profile.

| | p50 | p99 |
| --- | --- | --- |
| decode | 20.838 ms (20837888 ns) | 22.949 ms (22949440 ns) |

The row above includes reading the earlier three frames. With that overlap
tail already built, `decode_frame_with_tail` times only the 60-byte fourth
frame. It is still one frame late. 200 calls, same profile.

| | p50 | p99 |
| --- | --- | --- |
| decode, tail ready | 5.055 ms (5055180 ns) | 8.760 ms (8760302 ns) |

## Silent prime

A silent frame placed before the sine that frame 4 delays emits silence.
Peak 0.0024040816. Max abs error versus that sine is 0.5004582. Versus the
third frame's sine it is 0.50055003. The sine does not land on its own
samples.

## Flush frame, release

The flush is not a new sine. Its left half is the fourth frame's PCM. Its
right half is silence. Overlap-add against the fourth frame's history emits
the half-scale 440 Hz sine that frame 4 delays.

Max abs error versus that delayed sine is 0.0025660098. Max abs error versus
silence is 0.5008244.

Same profile, 200 calls. Not a throughput number for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 5.893 ms (5893053 ns) | 7.643 ms (7642850 ns) | 945 |

Decode is `decode_frame_with_tail` on that same 945-byte frame. The overlap
tail from the fourth frame already exists. The tail copy is outside the
timer. 200 calls, same profile.

| | p50 | p99 |
| --- | --- | --- |
| decode, tail ready | 5.073 ms (5073023 ns) | 5.327 ms (5326863 ns) |

## Stereo frame, release

One AAC-LC channel pair. 44.1 kHz. 1024 samples per channel. Left is the
half-scale 440 Hz sine. Right is silence. Both channels use the same long
sine window as the mono frame. Mid-side stereo is off. This is not SBR or
parametric stereo, and it is not a song encoder.

The left channel is the first-frame delay. Max abs error versus that sine is
0.5003551. The samples are within 0.51 of silence (peak 0.0035939883). Right
peak is 0. Host FFmpeg 6.1.1 decodes the frame as AAC-LC, 44100 Hz, stereo,
1024 samples per channel, with the same left error and a right peak of 0.

Same profile, 200 calls. Not a throughput number for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 5.690 ms (5690457 ns) | 7.035 ms (7035148 ns) | 517 |

## Second stereo frame, release

The second frame continues that pair. Left is the next 1024 samples of the
same half-scale 440 Hz sine. Right stays silence. Same long sine window.
Mid-side stereo stays off. Not SBR or parametric stereo, and not a song
encoder.

Overlap-add against the first stereo frame emits that frame's left sine, one
frame late. Max abs error versus that delayed sine is 0.0018240213. Right
peak is 0. Host FFmpeg 6.1.1 decodes the two-frame ADTS file as AAC-LC,
44100 Hz, stereo, 2048 samples per channel. The second hop matches this
error and a right peak of 0.

Same profile, 200 calls of this frame only. Not a throughput number for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 5.598 ms (5597749 ns) | 5.911 ms (5910809 ns) | 71 |

## Third stereo frame, release

The third frame continues that pair. Left is the next 1024 samples of the
same half-scale 440 Hz sine. Right stays silence. Same long sine window.
Mid-side stereo stays off. Not SBR or parametric stereo, and not a song
encoder.

Overlap-add against the second stereo frame emits that frame's left sine,
one frame late. Max abs error versus that delayed sine is 0.0008997023.
Right peak is 0. Host FFmpeg 6.1.1 decodes the three-frame ADTS file as
AAC-LC, 44100 Hz, stereo, 3072 samples per channel. The third hop matches
this error and a right peak of 0.

Same profile, 200 calls of this frame only. Not a throughput number for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 5.641 ms (5640735 ns) | 6.445 ms (6444696 ns) | 97 |

## Fourth stereo frame, release

The fourth frame continues that pair. Left is the next 1024 samples of the
same half-scale 440 Hz sine. Right stays silence. Same long sine window.
Mid-side stereo stays off. Not SBR or parametric stereo, and not a song
encoder.

Overlap-add against the third stereo frame emits that frame's left sine, one
frame late. Max abs error versus that delayed sine is 0.0012165308. Right
peak is 0. Host FFmpeg 6.1.1 decodes the four-frame ADTS file as AAC-LC,
44100 Hz, stereo, 4096 samples per channel. The fourth hop matches this
error and a right peak of 0.

Same profile, 200 calls of this frame only. Not a throughput number for a song.

| | p50 | p99 | output bytes |
| --- | --- | --- | --- |
| encode | 5.608 ms (5607668 ns) | 6.315 ms (6314886 ns) | 63 |

## Build

```
cargo build --release --manifest-path crates/export-aac/Cargo.toml
cargo build --release --manifest-path crates/export-aac/Cargo.toml --target wasm32-unknown-unknown --lib
```
