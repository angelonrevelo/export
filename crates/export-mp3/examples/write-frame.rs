//! Write one mono 44.1 kHz Layer III frame to a file.
//!
//! The file is a single frame. For ffmpeg 6.1, use [`export_mp3::append_ffmpeg_demuxer_suffix`]
//! or `examples/write-ffmpeg-openable.rs` so the demuxer can read four bytes at
//! offset 417. That suffix is not a second spectral frame. `scripts/ffmpeg-decode.sh`
//! can also repeat the frame.

use export_mp3::{encode_frame, FRAME_SAMPLES};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: write-frame <path.mp3>");
            return ExitCode::from(2);
        }
    };
    let mut pcm = [0.0f32; FRAME_SAMPLES];
    for (i, sample) in pcm.iter_mut().enumerate() {
        *sample = (i as f32 * 0.05).sin() * 0.2;
    }
    let frame = match encode_frame(&pcm) {
        Ok(frame) => frame,
        Err(err) => {
            eprintln!("encode failed: {err}");
            return ExitCode::from(1);
        }
    };
    if let Err(err) = fs::write(&path, &frame) {
        eprintln!("write {path} failed: {err}");
        return ExitCode::from(1);
    }
    eprintln!("wrote {} bytes to {path}", frame.len());
    ExitCode::SUCCESS
}
