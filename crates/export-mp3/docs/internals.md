# Internals

Honest limits for this crate. Nothing here is a speedup claim.

## Scope

One MPEG-1 Layer III frame, mono, 44.1 kHz, 128 kbps, no padding (417 bytes). `main_data_begin` is always 0. A frame that points into a bit reservoir returns `Error::Truncated`, because the API is given one buffer and does not keep a previous frame.

Not implemented, and rejected rather than guessed:

- stereo, joint stereo, dual channel (`Error::Unsupported`)
- sample rates other than 44.1 kHz (`Error::Unsupported`)
- short blocks and mixed blocks (`Error::Unsupported`)
- big-value Huffman tables and count-1 table B (`Error::Unsupported`)
- scale-factor bits (`scalefac_compress != 0` is `Error::Unsupported`)
- MPEG-2 / MPEG-2.5, Layer I/II, free bitrate, reserved header fields (`Error::BadHeader`)

## Signal model

Silence is `part2_3_length = 0` for both granules. The hybrid filterbank of a zero spectrum is exact silence, so decode returns `0.0` without running it.

A non-silent block stores table A code `0111` plus a sign bit in each granule. That quadruple is `(±1, 0, 0, 0)`. `global_gain` is `round(210 + 4 * log2(peak))` clamped to `0..=255`, which is the Layer III unit-line exponent. The decoder requantizes with `sign * |is|^(4/3) * 2^((global_gain - 210) / 4)`, then a 36-point IMDCT, alias reduction, and a 32-band polyphase synthesis.

The synthesis window is a 512-tap sine prototype, not a claim that the 512 ISO window coefficients were copied. Sample-level agreement with ffmpeg is therefore not claimed. The host script only checks that ffmpeg accepts the bytes and emits non-silent PCM for a non-silent frame.

This is not a waveform encoder. `half_scale_440hz` is `0.5 * sin(2π * 440 * n / 44100)` for 1152 samples. `decode_frame(encode_frame(...))` against that buffer has max absolute error 2.956202984 (same figure in debug and release). The decoded block is the filterbank output of one count-1 pulse, not the sine.

## ffmpeg

ffmpeg 6.1 `mp3_read_header` seeks to `frame_size` and reads four bytes. If that read fails it returns `Invalid frame size (N): Could not seek to N` and does not decode. A one-frame file still hits that path. Checked again on the 440 Hz sine frame: one copy exits 234 with `Invalid frame size (417): Could not seek to 417`; two copies of that same frame exit 0 with no error line and 9216 bytes of PCM. `scripts/ffmpeg-decode.sh` records the single-file refusal, repeats the same frame, and requires a clean decode of that copy. The library does not link or spawn ffmpeg.

## Memory and threads

Decode allocates one `Vec<f32>` of 1152 samples and uses fixed stack buffers for the spectrum and the synthesis delay line. No threads, no atomics feature for wasm, no rustls, no other crates.

## Benchmarks

`bench/runs/release.txt` is the recorded silence row (decode p50 948 ns, p99 1490 ns) and pulse row (decode p50 514755 ns, about 515 µs, p99 819588 ns), plus the truncated-frame error. Those two decode rows are left as measured. `bench/runs/sine-440.txt` is the 440 Hz error, not a timing. Percentiles are nearest-rank, `floor(p * (n - 1))`. A missing run file means the number was not measured.
