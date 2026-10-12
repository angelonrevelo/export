//! FLAC in, 16-bit WAV out. This binary does not call ffmpeg.

use export_audio::{export, Dedup, Job, Output};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(input) = args.next() else {
        eprintln!("usage: export_audio_wav <in.flac> <out.wav>");
        return ExitCode::from(2);
    };
    let Some(output) = args.next() else {
        eprintln!("usage: export_audio_wav <in.flac> <out.wav>");
        return ExitCode::from(2);
    };
    let bytes = match fs::read(&input) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("read: {err}");
            return ExitCode::from(1);
        }
    };
    let wav = match export(&bytes, Job::ToWav, &mut Dedup::new()) {
        Ok(Output::Fresh { data, .. }) => data,
        Ok(Output::Duplicate { .. }) => {
            eprintln!("duplicate");
            return ExitCode::from(1);
        }
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    if fs::write(&output, wav).is_err() {
        eprintln!("write failed");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
