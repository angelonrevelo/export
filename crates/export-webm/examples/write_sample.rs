use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: write_sample OUTPUT.webm [2x2|large|vp8-4096|vp8-460800]");
            return ExitCode::from(2);
        }
    };
    let kind = env::args().nth(2).unwrap_or_else(|| "2x2".to_owned());
    let input = match kind.as_str() {
        "2x2" | "small" => export_webm::sample_file(),
        "large" => export_webm::large_sample_file(),
        "vp8-4096" => export_webm::vp8_4096_webm(),
        "vp8-460800" => export_webm::vp8_460800_webm(),
        _ => {
            eprintln!("usage: write_sample OUTPUT.webm [2x2|large|vp8-4096|vp8-460800]");
            return ExitCode::from(2);
        }
    };
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
