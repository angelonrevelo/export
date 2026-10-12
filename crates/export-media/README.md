# export-media

Zero-dependency ISO BMFF remux and WAV/PCM. The product path is pure Rust.
It does not link or spawn ffmpeg. `scripts/oracle.sh` is a host byte-compare
and is the only place ffmpeg is allowed.

## Order of work

1. SHA-256 once, streaming, of the input bytes.
2. Dedup on that digest. A hit returns before sniff and before any parse.
3. Sniff magic bytes. Extensions are not an input.
4. Then remux, elementary copy, or WAV/PCM.

A failed parse is not inserted, so the same broken bytes can be retried.
A reader that would exceed the byte cap returns `Limit` before the hash is
committed and before a codec runs.

## Jobs

| Job | What it does |
|---|---|
| `Remux { faststart, audio_only, video_only }` | Stream-copy a non-fragmented mp4, mov, or m4a. Sample bytes are copied, not decoded. `faststart` places `moov` before `mdat`. |
| `Elementary { audio }` | Concatenate the first audio or video sample payloads. |
| `Wav { sample_rate, channels }` | WAV PCM in, or `sowt` / `twos` PCM inside MP4. Optional integer linear resample and mono downmix. |
| `concat_mp4` | Stream-copy two non-fragmented MP4s whose sample entries match. Two copies of the one-frame audiogram become two `raw ` samples. Codec mismatch and `moof` are errors. |
| `shrink_wav` | One downmix and at most one resample to a byte budget. Returns the file and its real byte count. |
| `audiogram` | One gray waveform frame (`raw `, 24-bit RGB) plus the PCM audio track. The video sample lasts as long as the WAV. |

`export_reader` pulls from `Read` into at most `max` bytes and pushes the
result as an owned buffer the caller writes.

## Honest limits

- Fragmented MP4 (`moof`) is detected and refused. It is not flattened.
- AAC, Opus, MP3, FLAC, GIF, and WebM/Matroska are sniffed and not decoded.
- Durations that need `mvhd` version 1, and `mdat` boxes past 4 GiB, are refused.
- There is no video pixel decode, GIF palette, or multi-pass video shrink. WAV shrink is one resample after an optional downmix.
- `audiogram` writes one uncompressed `raw ` sample. It does not emit 30 frames and it does not encode H.264 or AV1. ffmpeg reads that sample; Chrome reports a 0×0 picture. See `BENCH.md`.
- The resample is integer linear interpolation, not a windowed sinc.

## Bench

Release only. `cargo bench --bench throughput`, `cargo bench --bench concat_shrink`,
`cargo bench --bench audiogram`, and `cargo bench --bench concat_copies`
print MB/s, p50, p99, and allocations
per MB for hits and misses. A debug build exits 2 and does not publish a
timing verdict.

A coalesced "copy adjacent samples in one memcpy" probe lost. On a 4 MiB
track of 256 × 16 KiB samples, p50 was 255 µs versus 252 µs for one memcpy
per sample (gain −1.2%, under a 5% noise floor). That path was reverted.
The shipped copy reserves the byte count, then copies each sample.
