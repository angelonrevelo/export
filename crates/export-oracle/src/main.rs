//! `export-rust-oracle <src> <dest> <kind>`
//!
//! The outside score sets `EXPORT_RUST_ORACLE` to this binary.

#![forbid(unsafe_code)]

use std::env;
use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{err}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: export-rust-oracle <src> <dest> <kind>".to_string());
    }
    let src = PathBuf::from(&args[0]);
    let dest = PathBuf::from(&args[1]);
    let bytes = std::fs::read(&src).map_err(|err| format!("{}: {err}", src.display()))?;
    let out = export_oracle::convert(&args[2], &bytes)?;
    export_oracle::write_output(&dest, &out)
}
