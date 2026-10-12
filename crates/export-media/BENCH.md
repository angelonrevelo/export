# Bench

Release profile (`opt-level = 3`, `lto = thin`, `codegen-units = 1`).
`cargo bench --bench throughput`. Debug builds exit 2 and are not timed.
31 measured iterations after 3 warmup. Input is 4,197,744 bytes.
MB/s is input bytes / p50. Rows under 1 µs print `n/a` because that is the timer floor.

| path | result | MB/s | p50 | p99 | allocs/MB |
|---|---|---:|---:|---:|---:|
| sha256 | hit | 286.89 | 14.63 ms | 15.57 ms | 0.00 |
| sniff | hit | n/a | 35 ns | 130 ns | 0.00 |
| sniff | miss | n/a | 28 ns | 354 ns | 0.00 |
| bmff_parse | hit | 2,057,718 | 2.04 µs | 5.28 µs | 7.15 |
| bmff_parse | miss | n/a | 52 ns | 594 ns | 0.24 |
| remux | hit | 275.95 | 15.21 ms | 19.66 ms | 13.34 |
| dedup | hit | 282.01 | 14.88 ms | 16.70 ms | 0.00 |

`bmff_parse` does not read `mdat`. Its MB/s is how fast a 4 MB file is accepted, not a payload copy rate. `remux` and `dedup` both hash the file first; the dedup hit does not parse.

## Reverted

Coalesced copy of adjacent samples versus one copy per sample, same 4 MB track (256 × 16 KiB): p50 255 µs vs 252 µs, gain −1.2%. Under a 5% noise floor. Reverted. The shipped copy reserves the known byte count and then copies each sample.

## Concat and WAV shrink

`cargo bench --bench concat_shrink`. Same release profile, 31 iterations, 3 warmup. The rows above were not remeasured.

| path | result | MB/s | p50 | p99 | allocs/MB | bytes |
|---|---|---:|---:|---:|---:|---:|
| concat | hit | 728.66 | 2.883 ms | 3.068 ms | 48.56 | 2,100,576 |
| concat | miss | 9,494.13 | 0.221 ms | 0.264 ms | 33.32 | 2,100,576 |
| wav_shrink | hit | 482.72 | 2.072 ms | 2.545 ms | 5.00 | 1,000,044 |
| wav_shrink | miss | n/a | 45 ns | 99 ns | 0.00 | 1,050,288 |

Concat hit appends two non-fragmented MP4s. The miss is a codec mismatch after both files parse. WAV shrink is one mono downmix and one resample to a 200,000 byte budget. The shrink miss is an MP4, rejected by the magic check.

## Audiogram

`cargo bench --bench audiogram`. Same release profile, 31 iterations, 3 warmup. The rows above were not remeasured.

The input is one second of 48 kHz mono 16-bit PCM WAV (96,044 bytes). `1` is the shipped file: the waveform drawn once. `30fps` stores that same frame 30 times so the video lasts one second. MB/s and allocs/MB divide by the WAV size. `bytes` is the output file.

| path | result | MB/s | p50 | p99 | allocs/MB | bytes |
|---|---|---:|---:|---:|---:|---:|
| audiogram | 1 | 661.07 | 0.145 ms | 0.217 ms | 354.00 | 327,420 |
| audiogram | 30fps | 11.43 | 8.400 ms | 8.749 ms | 739.24 | 7,009,252 |

The file this crate ships is the one-frame MP4, 327,420 bytes. The 30 fps file is the comparison. It is not what `audiogram` returns.

### Uncompressed sample entry

The video sample entry is `raw ` (VisualSampleEntry, depth 24). The frame is 640×120. Each pixel is gray RGB, three equal bytes: background 32, waveform 220. The shipped movie has one video sample. Its timescale is the WAV rate and its delta is the audio frame count, so the still lasts as long as the sound. The audio track is `sowt`, one sample, the PCM bytes. There is no H.264 or AV1 sample.

ffmpeg reads this entry. `ffprobe` reports `rawvideo` (`raw `), `rgb24`, `nb_frames=1`, and `pcm_s16le` (`sowt`). A decode to rgb24 was 230,400 bytes, and every pixel stayed gray (only 32 and 220).

Chrome (headless) fired `loadeddata` with duration 1 and a video size of 0×0. It did not decode the picture. Safari and QuickTime were not run on this host. The file has no `avc1`, `hvc1`, or `av01` sample, which is what those players require for MP4 video. VLC was not installed here.

## Two copies

`cargo bench --bench concat_copies`. Same release profile, 31 iterations, 3 warmup. The audiogram row above was not remeasured. The coded size is still 640×120.

`concat_mp4` stream-copies two copies of the one-frame audiogram this crate already writes. The picture bytes are the same `raw ` sample twice. Nothing is re-encoded.

| path | samples | bytes | p50 | p99 |
|---|---:|---:|---:|---:|
| concat_copies | 2 | 653,824 | 0.642 ms | 0.808 ms |

p50 is under 2 ms. `scripts/concat_copies.sh` is a host ffprobe only. It reported `codec_name=rawvideo`, 640×120, `nb_frames=2`. The library does not call ffmpeg.

## Picture bytes

`cargo bench --bench picture_bytes`. Same release profile, 31 iterations, 3 warmup. The one-frame row and the two-sample row above were not remeasured. The sample-count ladder stays stopped at two.

`elementary` copies the one video sample out of the one-frame audiogram this crate already writes. It does not decode the pixels and it does not write a new file. The bytes are the `raw ` sample: 640×120×3.

| path | bytes | p50 | p99 |
|---|---:|---:|---:|
| picture_bytes | 230,400 | 5.559 µs | 5.999 µs |

p50 is under 0.30 ms.

## Two picture samples

`cargo bench --bench picture_pair`. Same release profile, 31 iterations, 3 warmup. The rows above were not remeasured. The sample-count ladder stays stopped at two.

`elementary` copies both video samples out of the two-sample file this crate already writes (653,824 bytes). Each sample is the 230,400-byte `raw ` frame. It does not decode the pixels and it does not write a new file.

| path | samples | bytes | p50 | p99 |
|---|---:|---:|---:|---:|
| picture_pair | 2 | 460,800 | 11.768 µs | 14.843 µs |

p50 is under 12.2 µs.
