use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let input_path = match args.next() {
        Some(path) => path,
        None => {
            eprintln!("usage: copy_file INPUT.webm OUTPUT.webm");
            return ExitCode::from(2);
        }
    };
    let output_path = match args.next() {
        Some(path) => path,
        None => {
            eprintln!("usage: copy_file INPUT.webm OUTPUT.webm");
            return ExitCode::from(2);
        }
    };
    let input = match fs::read(&input_path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("read {input_path}: {err}");
            return ExitCode::from(1);
        }
    };
    let copied = match export_webm::stream_copy(&input) {
        Ok(copied) => copied,
        Err(err) => {
            eprintln!("stream copy failed: {err}");
            return ExitCode::from(1);
        }
    };
    if let Err(err) = fs::write(&output_path, &copied.bytes) {
        eprintln!("write {output_path}: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
