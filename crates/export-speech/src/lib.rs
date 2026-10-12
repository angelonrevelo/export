//! Speech, OCR, and TTS slice for Export.
//!
//! The hot crate has no dependencies. Release builds use thin LTO and one
//! codegen unit (`Cargo.toml`). Nothing here shells out to Whisper, ffmpeg,
//! Tesseract, or Piper, and nothing loads an ONNX file or a resident acoustic
//! model.
//!
//! What is real, and the limit next to it:
//!
//! - [`mel::MelStream`] — streaming log-mel, fixed memory, 16 kHz / 80 mels.
//! - [`runtime`] — `i32` matmul, conv1d, softmax, layer norm, checked against
//!   hand arithmetic.
//! - [`decode`] — greedy CTC on synthetic logits, plus `txt` / `srt` / `vtt` /
//!   `notes` / `words` writers in Export's shape.
//! - [`recognize::transcribe_fixture`] — one 3×3 integer layer. It spells `HI`
//!   for two bin-centered tones and `IH` if those tones are swapped. It is a
//!   fixture, not speech recognition.
//! - [`blob`] — CRC-checked `81×3` integer weights, 984 bytes. They decode the
//!   formant word `NO` (not a pure sine, not a frequency branch). They do not
//!   transcribe microphone speech. A missing or flipped blob is an error.
//!   Decode scratch stays flat as the clip grows.
//! - [`ocr`] — 5×7 printed `A`–`Z` `0`–`9` at one pitch. Not Tesseract.
//! - [`tts::synthesize`] — formant sines to WAV. Not Piper.
//! - [`denoise`] — spectral subtraction from a short noise preamble.
//! - [`crossover`] — a 300 Hz Butterworth split. Not Demucs.

#![forbid(unsafe_code)]

pub mod blob;
pub mod crossover;
pub mod decode;
pub mod denoise;
pub mod mel;
pub mod ocr;
pub mod recognize;
pub mod runtime;
pub mod tts;
pub mod wav;

mod fft;

pub use blob::{
    bundled_no_blob, load_acoustic, transcribe_acoustic, transcribe_bundled, BUNDLED_NO_LEN,
};
pub use crossover::Crossover;
pub use decode::{
    ctc_greedy, ctc_greedy_logits, write_notes, write_srt, write_txt, write_vtt, write_words,
    Segment, Transcript, Word,
};
pub use denoise::DenoiseScratch;
pub use mel::MelStream;
pub use ocr::{read_fixed_pitch, render_line};
pub use recognize::{fixture_hi_pcm, transcribe_fixture};
pub use runtime::{conv1d_i32, layer_norm_i32, matmul_i32, softmax_ppm};
pub use tts::{speech_sample_count, synthesize};
