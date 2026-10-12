# export-audio

Zero-dependency FLAC subset decoder and encoder. The product path is pure
Rust. It does not link or spawn ffmpeg. `scripts/oracle.sh` is a host
byte-compare and is the only place ffmpeg is allowed.

Threading is off. The crate does not spawn work and does not read a
parallelism setting.

## Order of work

1. SHA-256 once, streaming, of the input bytes.
2. Dedup on that digest. A hit returns before sniff and before the codec.
3. Sniff magic bytes. Extensions are not an input.
4. Then FLAC decode or encode.

A failed parse is not inserted, so the same broken bytes can be retried.
A reader that would exceed the byte cap returns `Limit` before the hash is
committed and before a codec runs.

## Jobs

| Job | What it does |
|---|---|
| `ToFlac` | WAV PCM or FLAC in, subset FLAC out. |
| `ToWav` | FLAC in, WAV PCM out. |

MP3, AAC, and Opus are sniffed and refused with `unsupported: MP3` (or AAC,
or Opus). There is no encoder for those formats.

## What the subset codec does

The encoder writes a `STREAMINFO` block and fixed-blocksize frames. Each
subframe is constant, verbatim, or fixed order 0..=4, with one Rice
partition or an unencoded residual. An LPC coefficient subframe (precision
8, 12, or 15) replaces that fixed predictor only when the subframe's bit
length is strictly smaller. Order stops at 12 when the sample rate is
48 kHz or below, and at 32 above that.

The decoder also accepts left/side, right/side, and mid/side, wasted bits,
and Rice partition orders. A truncated stream returns `Truncated`. A flipped
frame CRC returns `corrupt: crc16`. A non-zero STREAMINFO MD5 that does not
match the samples returns `corrupt: md5`.

## Honest limits

- Not an MP3, AAC, or Opus codec. Those byte streams error.
- The encoder does not write seek tables, cues, pictures, or wasted-bits.
  It uses one Rice partition per subframe. LPC is not a second codec: it is
  tried only against the fixed predictor already chosen for that block, and
  it is kept only when that subframe is strictly shorter.
- Sample width on the way out is 8, 16, or 24. The stream must hold at least
  16 samples per channel. Block size stays inside the subset cap (4608 at
  48 kHz and below, 16384 above).
- The decoded PCM is held in one buffer, capped at 16,777,216 interleaved
  samples. Input through `export_reader` is capped at 64 MiB.
- A FLAC file from another encoder round-trips the samples, not the bytes.
  Two valid FLAC streams of the same audio are not required to match.
- This build was measured single-threaded, release, on one machine. Do not
  read the table below as a speedup over ffmpeg. That comparison was not made.

## Bench

Release only (`opt-level = 3`, `lto = thin`, `codegen-units = 1`).
`cargo bench --bench throughput` prints MB/s, p50, and p99. A debug build
exits 2 and does not publish a timing verdict. 31 iterations after 3 warmup
passes. MB/s is the named byte count divided by p50.

The fixture is 1,048,576 mono 16-bit samples at 44.1 kHz (2,097,152 PCM
bytes). The encoded FLAC is 940,154 bytes. The same fixture with only the
fixed predictor was 956,218 bytes, so the LPC subframe is a size win on
this file. Decode and hash rows use the FLAC size. The encode row uses the
PCM size. This is one machine, one process, single-threaded. It is not a
comparison with ffmpeg.

| path | result | MB/s | p50 | p99 | bytes |
|---|---|---:|---:|---:|---:|
| sha256 | hit | 320.33 | 2.93 ms | 3.03 ms | 940,154 |
| flac_decode | miss | 38.02 | 24.7 ms | 31.0 ms | 940,154 |
| dedup | hit | 319.22 | 2.95 ms | 5.34 ms | 940,154 |
| flac_encode | miss | 4.43 | 473.9 ms | 543.0 ms | 2,097,152 |
| truncated | miss | 65.85 | 14.3 ms | 14.9 ms | 940,153 |

`flac_decode` / `miss` is a dedup miss: hash, sniff, decode, write WAV.
`dedup` / `hit` hashes and returns before the codec, which is why it sits
next to SHA-256. `flac_encode` / `miss` is PCM to subset FLAC.
`truncated` / `miss` drops the last byte and must return an error; the
time is how fast that failure is found, not a successful decode.

Decode of that file allocates about 3,764 times per MB. The buffers are
built per frame. That cost is inside the 38.02 MB/s figure. Encode time
includes the LPC search on every fixed block. That search is why encode
p50 is 473.9 ms.

### Reader experiment

Rice quotients were timed two ways, interleaved in one process, 31 rounds,
on the fixed-predictor file (956,218 bytes) before LPC subframes were
added. Those rows were not re-measured. The figure is raw decode, not the
product path above.

| reader | MB/s | p50 | p99 |
|---|---:|---:|---:|
| leading-zero in the current byte | 49.92 | 19.16 ms | 25.36 ms |
| one bit at a time | 43.20 | 22.14 ms | 33.27 ms |

p50 moved 13.5%, past a 5% floor, so the leading-zero reader stayed and the
bit-at-a-time reader was removed. A change inside that floor would have
been reverted.
