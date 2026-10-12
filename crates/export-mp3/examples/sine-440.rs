//! Encode one mono 44.1 kHz frame from a half-scale 440 Hz sine.
//!
//! Prints the max absolute sample error against that sine. The decoded block
//! is not the sine. This crate is not a waveform encoder and not a song encoder.

use export_mp3::{decode_frame, encode_frame, half_scale_440hz, FRAME_SAMPLES, SAMPLE_RATE_HZ};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let pcm = half_scale_440hz();
    let frame = match encode_frame(&pcm) {
        Ok(frame) => frame,
        Err(err) => {
            eprintln!("encode failed: {err}");
            return ExitCode::from(1);
        }
    };
    let decoded = match decode_frame(&frame) {
        Ok(pcm) => pcm,
        Err(err) => {
            eprintln!("decode failed: {err}");
            return ExitCode::from(1);
        }
    };
    let max_abs = max_abs_error(&pcm, &decoded);
    println!("samples={FRAME_SAMPLES}");
    println!("sample_rate_hz={SAMPLE_RATE_HZ}");
    println!("tone_hz=440");
    println!("scale=0.5");
    println!("frame_bytes={}", frame.len());
    println!("max_abs_error={max_abs:.9}");
    println!("matches_sine=false");
    println!("waveform_encoder=false");
    if let Some(path) = env::args().nth(1) {
        if let Err(err) = fs::write(&path, &frame) {
            eprintln!("write {path} failed: {err}");
            return ExitCode::from(1);
        }
        eprintln!("wrote {} bytes to {path}", frame.len());
    }
    ExitCode::SUCCESS
}

fn max_abs_error(input: &[f32], decoded: &[f32]) -> f32 {
    input
        .iter()
        .zip(decoded.iter())
        .fold(0.0f32, |max, (src, out)| max.max((src - out).abs()))
}
