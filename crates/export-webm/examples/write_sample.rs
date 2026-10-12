use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: write_sample OUTPUT.webm");
            return ExitCode::from(2);
        }
    };
    let input = export_webm::sample_file();
    let copied = match export_webm::stream_copy(&input) {
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
