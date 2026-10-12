use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: write_sample OUTPUT.mp4");
            return ExitCode::from(2);
        }
    };
    let copied = match export_fmp4::stream_copy(&export_fmp4::sample_file()) {
        Ok(copied) => copied,
        Err(err) => {
            eprintln!("stream copy failed: {err}");
            return ExitCode::from(1);
        }
    };
    if let Err(err) = fs::write(&path, &copied.bytes) {
        eprintln!("write {path}: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
