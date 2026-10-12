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
| CTC | Greedy collapse of synthetic logits, plus `txt` / `srt` / `vtt` / `notes` / `words` in Export's shape. | No beam. `words` times are character spans from the fixture, not linguistic words. `language` is null. |
| Fixture recognizer | PCM → log-mel peak → one 3×3 matmul → greedy CTC. | Spells `HI` for a 406.25 Hz tone then a 1187.5 Hz tone, and `IH` if those tones are swapped. Silence stays empty. It does not transcribe speech. |
| OCR | Hamming distance to one 5×7 prototype each for `A`–`Z` and `0`–`9`, fixed pitch `6n−1`. | Not Tesseract. No page layout, lowercase, rotation, or other sizes. Templates are at least 3 bits apart, so a one-bit edit of `H` still reads `H`. |
| TTS | Formant sines (and a deterministic noise burst on consonants) to mono 16-bit WAV. | Not Piper and not `say`. Intelligibility is not claimed; tests check duration, RMS, and byte-identical repeats. |
| Denoise | Spectral subtraction, FFT 256, noise floor frozen from the first 2048 samples. Scratch is 4616 bytes. | A speech lead-in is treated as noise and dulled. Not `noisereduce`. |
| Bands | 300 Hz Butterworth lowpass and highpass. State is 56 bytes. | Not Demucs. No vocals, drums, bass, or other. A one-pole residual was measured (an 80 Hz sine still left about a quarter of its energy in the residual) and not kept. |

## Benchmark

One release run on this x86_64 VM, rustc 1.83.0, 80 samples after warmup. p99 is the slow tail. `gop` is `2·m·n·k` integer multiply-adds per second, divided by 1e9. Every hot call below allocated 0 times (caller-owned buffers, counting global allocator).

```text
mel_frames_per_sec p50=51725.6 p99=44650.5 p50_ns=7675123 p99_ns=8891274 allocs_per_pass=0 frames_per_pass=397 state_bytes=90768 n_fft=512 secs=4 samples=80
matmul_64 p50_ns=157157.0 p99_ns=171392.0 p50_gop=3.336 p99_gop=3.059 allocs_per_call=0 samples=80
matmul_128 p50_ns=1241250.0 p99_ns=1408256.0 p50_gop=3.379 p99_gop=2.978 allocs_per_call=0 samples=80
denoise_into allocs_per_call=0 scratch_bytes=4616 crossover_allocs_per_call=0 crossover_state_bytes=56
```

Other release runs on this same VM, same binary shape, sat at mel p50 about 52k–56k frames/s and mel p99 about 45k–52k. Matmul-128 p50 stayed near 1.18–1.24 ms (about 3.4–3.5 Gop/s). The table is one run, not a floor.
