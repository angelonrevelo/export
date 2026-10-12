use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let input = match args.next() {
        Some(path) => path,
        None => {
            eprintln!("usage: decode <frame.ivf> [out.yuv]");
            return ExitCode::from(2);
        }
    };
    let output = args.next();
    let bytes = match fs::read(&input) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("read {input}: {err}");
            return ExitCode::from(1);
        }
    };
    let frame = match export_vp9::decode_ivf(&bytes) {
        Ok(frame) => frame,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    let mut yuv = Vec::with_capacity(frame.y.len() + frame.u.len() + frame.v.len());
    yuv.extend_from_slice(&frame.y);
    yuv.extend_from_slice(&frame.u);
    yuv.extend_from_slice(&frame.v);
    if let Some(path) = output {
        if let Err(err) = fs::write(&path, &yuv) {
            eprintln!("write {path}: {err}");
            return ExitCode::from(1);
        }
    } else if let Err(err) = io::stdout().write_all(&yuv) {
        eprintln!("write stdout: {err}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
