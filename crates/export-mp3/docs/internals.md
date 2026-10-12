# Internals

Honest limits for this crate. Nothing here is a speedup claim.

## Scope

One MPEG-1 Layer III frame, mono, 44.1 kHz, 128 kbps, no padding (417 bytes). `main_data_begin` is always 0. A frame that points into a bit reservoir returns `Error::Truncated`, because the API is given one buffer and does not keep a previous frame.

Not implemented, and rejected rather than guessed:

- stereo, joint stereo, dual channel (`Error::Unsupported`)
- sample rates other than 44.1 kHz (`Error::Unsupported`)
- short blocks and mixed blocks (`Error::Unsupported`)
- big-value Huffman tables other than table 5, and count-1 table B (`Error::Unsupported`)
- scale-factor bits (`scalefac_compress != 0` is `Error::Unsupported`)
- MPEG-2 / MPEG-2.5, Layer I/II, free bitrate, reserved header fields (`Error::BadHeader`)

## Signal model

Silence is `part2_3_length = 0` and `big_values = 0` for both granules. The hybrid filterbank of a zero spectrum is exact silence, so decode returns `0.0` without running it.

A non-silent block stores the first 16 long-block MDCT lines of subband 0 in each granule. `big_values` is 8, region 0 ends at line 16 (`region0_count = 3`), and the pairs are Huffman table 5 (magnitudes 0..=3, no linbits). Lines 16 and 17, and every other subband, stay zero. `global_gain` is chosen so the largest rounded magnitude is at most 3 and as large as that limit allows. The decoder requantizes with `sign * |is|^(4/3) * 2^((global_gain - 210) / 4)`, then alias reduction, a 36-point IMDCT, and a 32-band polyphase synthesis. A `big_values == 0` payload is still count-1 table A. The encoder does not write that payload.

The analysis window is the 512-point polyphase window. The synthesis window is a 512-tap sine prototype. Sample-level agreement with ffmpeg is therefore not claimed. The host script only checks that ffmpeg accepts the bytes.

`half_scale_440hz` is `0.5 * sin(2π * 440 * n / 44100)` for 1152 samples. `decode_frame(encode_frame(...))` against that buffer has max absolute error 0.680283487 (same figure in debug and release). The decoded block is those 16 quantized lines through this crate's filterbank, not the sine.

On the two-copy file ffmpeg already opens, the host decode is 2304 float samples, absolute peak 0.537770271, RMS 0.185432036, best absolute correlation with the sine 0.750842570. That correlation is above 0.5. The sample-aligned error in this crate is 0.680283487, so the frame is not a waveform encoding of the sine. The library does not call ffmpeg.

## ffmpeg

ffmpeg 6.1 `mp3_read_header` seeks to `frame_size` and reads four bytes. If that read fails it returns `Invalid frame size (N): Could not seek to N` and does not decode. A one-frame file still hits that path. Checked again on the 440 Hz sine frame: one copy exits 234 with `Invalid frame size (417): Could not seek to 417`; two copies of that same frame exit 0 with no error line and 9216 bytes of PCM. `scripts/ffmpeg-decode.sh` records the single-file refusal, repeats the same frame, and requires a clean decode of that copy. The library does not link or spawn ffmpeg.

## Memory and threads

Decode allocates one `Vec<f32>` of 1152 samples and uses fixed stack buffers for the spectrum and the synthesis delay line. No threads, no atomics feature for wasm, no rustls, no other crates.

## Benchmarks

`bench/runs/release.txt` keeps the recorded silence row (decode p50 948 ns, p99 1490 ns) and the recorded truncated row (decode p50 33 ns, p99 37 ns). The 515 µs row (`decode_tone_ns` p50 514755 ns, p99 819588 ns) is the older count-1 pulse path. A later release decode of the existing 16-line half-scale 440 Hz frame is `decode_440hz_16line_ns` p50 559231 ns, p99 581941 ns. That is not a waveform. `bench/runs/sine-440.txt` is the 440 Hz sample error, not this timing. Percentiles are nearest-rank, `floor(p * (n - 1))`, 1000 iterations, 50 warmup, release `opt-level = 3`, `lto = "thin"`, `codegen-units = 1`. A missing run file means the number was not measured.
