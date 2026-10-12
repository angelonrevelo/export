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
partition or an unencoded residual. One LPC candidate replaces that fixed
predictor only when the subframe's bit length is strictly smaller: order 11
at coefficient precision 12, or the subset order cap when that cap is lower
than 11. Order 11 is inside the subset cap of 12 at 48 kHz and below.

The decoder also accepts left/side, right/side, and mid/side, wasted bits,
and Rice partition orders. A truncated stream returns `Truncated`. A flipped
frame CRC returns `corrupt: crc16`. A non-zero STREAMINFO MD5 that does not
match the samples returns `corrupt: md5`.

## Honest limits

- Not an MP3, AAC, or Opus codec. Those byte streams error.
- `encode` does not write a seek table, cues, pictures, or wasted-bits.
  `encode_with_seek_table` writes one seekpoint per block. The one-second
  16-bit stereo measurement is the file that uses it. The encoder uses one
  Rice partition per subframe. LPC is not a second codec. The
  only candidate is order 11 at precision 12, tried against the fixed
  predictor already chosen for that block, and kept only when that subframe
  is strictly shorter. A search over orders 1..=12 and precisions 8, 12,
  and 15 made this fixture 940,154 bytes but took 473.9 ms to encode, so
  that wider search is not what the encoder runs.
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
bytes). The encoded FLAC is 948,202 bytes. The same fixture with only the
fixed predictor was 956,218 bytes. Decode and hash rows use the FLAC size.
The encode row uses the PCM size. This is one machine, one process,
single-threaded. It is not a comparison with ffmpeg.

| path | result | MB/s | p50 | p99 | bytes |
|---|---|---:|---:|---:|---:|
| sha256 | hit | 307.58 | 3.08 ms | 3.26 ms | 948,202 |
| flac_decode | miss | 37.23 | 25.5 ms | 26.4 ms | 948,202 |
| dedup | hit | 296.43 | 3.20 ms | 4.64 ms | 948,202 |
| flac_encode | miss | 26.54 | 79.0 ms | 87.4 ms | 2,097,152 |
| truncated | miss | 58.50 | 16.2 ms | 21.3 ms | 948,201 |

`flac_decode` / `miss` is a dedup miss: hash, sniff, decode, write WAV.
`dedup` / `hit` hashes and returns before the codec, which is why it sits
next to SHA-256. `flac_encode` / `miss` is PCM to subset FLAC.
`truncated` / `miss` drops the last byte and must return an error; the
time is how fast that failure is found, not a successful decode.

Decode of that file allocates about 3,665 times per MB. The buffers are
built per frame. That cost is inside the 37.23 MB/s figure. Encode p50 is
79.0 ms with the single LPC candidate above. The wider search that produced
940,154 bytes is not in this table.

### Stereo block

Separate from the mono row above. One block of 4,096 samples per channel,
16-bit, 44.1 kHz, interleaved. The left channel uses the mono fixture
formula. The right channel swaps the ramp and the slow term and flips the
wobble, so the two channels are not copies. Both channels round-trip. The
encoder writes the channels independently. The encoded file is 7,601 bytes.
Encode p50 is 0.513 ms and p99 is 0.612 ms (31.92 MB/s on the 16,384 PCM
bytes), release, 31 iterations after 3 warmup passes. Decode of that file
back to the samples is p50 0.150 ms and p99 0.186 ms (50.72 MB/s on the
7,601 FLAC bytes), same iteration count.

### 24-bit mono block

Separate from the 16-bit rows above. One mono block of 4,096 samples at
24-bit, 44.1 kHz. Each sample is the mono fixture formula shifted up 8 bits,
plus the low byte `i & 255`, so the value does not fit in 16 bits. The
samples round-trip. The encoded file is 7,293 bytes. Encode p50 is 0.323 ms
and p99 is 0.391 ms (37.99 MB/s on the 12,288 PCM bytes), release, 31
iterations after 3 warmup passes. Decode of that file back to the samples
is p50 0.103 ms and p99 0.116 ms (70.93 MB/s on the 7,293 FLAC bytes), same
iteration count.

### 24-bit stereo block

Separate from the 16-bit stereo row and the 24-bit mono row. One block of
4,096 samples per channel, 24-bit, 44.1 kHz, interleaved. The left channel
is the 24-bit mono block. The right channel uses the swapped ramp and slow
terms, with the low byte flipped, so the channels are not copies. Both
channels round-trip. The encoded file is 16,025 bytes. Encode p50 is
0.592 ms and p99 is 0.693 ms (41.50 MB/s on the 24,576 PCM bytes). Decode
of that file is p50 0.208 ms and p99 0.254 ms (76.95 MB/s on the 16,025
FLAC bytes). Release, 31 iterations after 3 warmup passes.

### One second of 16-bit stereo

Separate from the 4,096-sample stereo row, the 24-bit stereo row, and the
mono 1,048,576-sample row. 44,100 samples per channel, 16-bit, 44.1 kHz.
The same two channel formulas as the 4,096-sample stereo block, so the
channels are not copies. The same LPC candidate, order 11 at precision 12,
and only when that subframe is strictly shorter. Both channels round-trip.
The encoded file is 87,272 bytes. Encode p50 is 5.97 ms and p99 is 6.33 ms
(29.56 MB/s on the 176,400 PCM bytes). Decode p50 is 1.58 ms and p99 is
1.72 ms (55.24 MB/s on the 87,272 FLAC bytes). Release, 31 iterations after
3 warmup passes. A linear scale of the 4,096-sample stereo encode is about
5.5 ms. This encode is 5.97 ms, so the length stayed. The same audio with
one seekpoint per block is 87,474 bytes. Encode p50 is 5.96 ms, beside the
87,272 bytes and 5.97 ms above. 5.96 ms is under the 6.57 ms cap, so the
seek table stayed.

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
