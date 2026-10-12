# export-speech

A zero-dependency slice of Export's speech, OCR, and TTS jobs. It does not shell out to Whisper, ffmpeg, Tesseract, or Piper, and it does not load ONNX, `large-v3-turbo`, or Demucs.

Build and test from this directory. The crate is not a workspace member.

```sh
cargo test
cargo clippy --all-targets -- -D warnings
cargo run --release --bin speech-bench
```

Release is `lto = "thin"` and `codegen-units = 1`.

## What it actually does

| Job | What runs | Limit |
|---|---|---|
| Log-mel | Streaming 16 kHz, 512-point FFT, hop 160, 80 mels. [`MelStream`] is 90768 bytes and does not grow with the clip. | Frontend only. Not an acoustic model. |
| Integer runtime | `i32` matmul, valid conv1d, table softmax (parts per million), truncating layer norm. | Checked against hand arithmetic in `runtime.rs`. Not a Whisper graph. |
| CTC | Greedy collapse of synthetic logits, plus `txt` / `srt` / `vtt` / `notes` / `words` in Export's shape. | No beam. `language` is null. |
| Fixture recognizer | PCM → log-mel peak → one 3×3 matmul → greedy CTC. | A fixture, not recognition. Spells `HI` for a 406.25 Hz tone then a 1187.5 Hz tone, and `IH` if those tones are swapped. Silence stays empty. |
| Weight blob | CRC-32 payload, magic `ESW1`, **984 bytes**, `81×3` i32 matrix into the same greedy CTC. | Decodes the English word `NO` from this crate's formant rendering (noise burst plus vowels) at gain 0.75 and a 320-sample lead. That clip is not a pure sine (mel peak span ≥ 8) and the matrix is a perceptron, not a frequency branch. Gain 0.75 and lead 320 were not in the fit. A missing blob or a flipped byte is an error. It does not transcribe microphone speech, and `YES` and a pure tone do not come back as `NO`. Decode scratch is the 90768-byte mel state plus the weight vector. A 30 s silence stays empty and does not grow that scratch. |
| OCR | Hamming distance to one 5×7 prototype each for `A`–`Z` and `0`–`9`, fixed pitch `6n−1`. | Not Tesseract. No page layout, lowercase, rotation, or other sizes. Templates are at least 3 bits apart, so a one-bit edit of `H` still reads `H`. |
| TTS | Formant sines (and a deterministic noise burst on consonants) to mono 16-bit WAV. | Not Piper and not `say`. Intelligibility is not claimed; tests check duration, RMS, and byte-identical repeats. |
| Denoise | Spectral subtraction, FFT 256, noise floor frozen from the first 2048 samples. Scratch is 4616 bytes. | A speech lead-in is treated as noise and dulled. Not `noisereduce`. |
| Bands | 300 Hz Butterworth lowpass and highpass. State is 56 bytes. | Not Demucs. No vocals, drums, bass, or other. A one-pole residual was measured (an 80 Hz sine still left about a quarter of its energy in the residual) and not kept. |

## Benchmark

One release run on this x86_64 VM, rustc 1.83.0, 80 samples after warmup. p99 is the slow tail. `gop` is `2·m·n·k` integer multiply-adds per second, divided by 1e9. Every hot call below allocated 0 times (caller-owned buffers, counting global allocator).

```text
mel_frames_per_sec p50=56349.1 p99=53721.6 p50_ns=7045362 p99_ns=7389948 allocs_per_pass=0 frames_per_pass=397 state_bytes=90768 n_fft=512 secs=4 samples=80
matmul_64 p50_ns=145411.0 p99_ns=164300.0 p50_gop=3.606 p99_gop=3.191 allocs_per_call=0 samples=80
matmul_128 p50_ns=1157750.0 p99_ns=1316320.0 p50_gop=3.623 p99_gop=3.186 allocs_per_call=0 samples=80
denoise_into allocs_per_call=0 scratch_bytes=4616 crossover_allocs_per_call=0 crossover_state_bytes=56
```

The run immediately before that, same binary, was slower in the tail: mel p99 40617 frames/s, matmul-128 p99 1.64 ms. p50 stayed near 55k–56k frames/s and 1.16–1.19 ms. The table is one run, not a floor. `gop` counts integer multiply-adds (`2·m·n·k` per second / 1e9), not IEEE FLOPs.
