//! Write the half-scale 440 Hz frame with the ffmpeg demuxer suffix.
//!
//! The library does not call ffmpeg. Hosts append four sync bytes so ffmpeg
//! 6.1 can read a header at offset 417. That is not a second spectral frame.

use export_mp3::{
    append_ffmpeg_demuxer_suffix, decode_frame, encode_frame, half_scale_440hz, FRAME_BYTES,
};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: write-ffmpeg-openable <path.mp3>");
            return ExitCode::from(2);
        }
    };
    let pcm = half_scale_440hz();
    let frame = match encode_frame(&pcm) {
        Ok(frame) => frame,
        Err(err) => {
            eprintln!("encode failed: {err}");
            return ExitCode::from(1);
        }
    };
    let openable = match append_ffmpeg_demuxer_suffix(&frame) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("suffix failed: {err}");
            return ExitCode::from(1);
        }
    };
    let decoded = match decode_frame(&openable[..FRAME_BYTES]) {
        Ok(samples) => samples,
        Err(err) => {
            eprintln!("decode failed: {err}");
            return ExitCode::from(1);
        }
    };
    let max_abs = pcm
        .iter()
        .zip(decoded.iter())
        .fold(0.0f32, |max, (src, out)| max.max((src - out).abs()));
    if let Err(err) = fs::write(&path, &openable) {
        eprintln!("write {path} failed: {err}");
        return ExitCode::from(1);
    }
    println!("file_bytes={}", openable.len());
    println!("frame_bytes={FRAME_BYTES}");
    println!("max_abs_error={max_abs:.9}");
    eprintln!("wrote {} bytes to {path}", openable.len());
    ExitCode::SUCCESS
}
