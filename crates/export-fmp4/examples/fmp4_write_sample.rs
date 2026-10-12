use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: fmp4_write_sample OUTPUT.mp4 [pcm|h264|h264x47|h264x188]");
            return ExitCode::from(2);
        }
    };
    let kind = env::args().nth(2).unwrap_or_else(|| "pcm".to_owned());
    let input = match kind.as_str() {
        "pcm" | "audio" => export_fmp4::sample_file(),
        "h264" => export_fmp4::h264_file(),
        "h264x47" => export_fmp4::h264_repeat_file(),
        "h264x188" => export_fmp4::h264_x188_file(),
        _ => {
            eprintln!("usage: fmp4_write_sample OUTPUT.mp4 [pcm|h264|h264x47|h264x188]");
            return ExitCode::from(2);
        }
    };
    let copied = match export_fmp4::stream_copy(&input) {
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
