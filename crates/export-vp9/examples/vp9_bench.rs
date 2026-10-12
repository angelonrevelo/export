use std::env;
use std::fs;
use std::hint::black_box;
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let path = match env::args().nth(1) {
        Some(path) => path,
        None => {
            eprintln!("usage: bench <frame.ivf>");
            return ExitCode::from(2);
        }
    };
    let bytes = match fs::read(&path) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("read {path}: {err}");
            return ExitCode::from(1);
        }
    };
    let frame_bytes = if bytes.len() >= 44 {
        u32::from_le_bytes([bytes[32], bytes[33], bytes[34], bytes[35]]) as usize
    } else {
        0
    };
    for _ in 0..200 {
        let frame = export_vp9::decode_ivf(black_box(&bytes)).expect("decode");
        black_box(&frame.y);
    }
    let samples = 2000u32;
    let mut nanos = Vec::with_capacity(samples as usize);
    for _ in 0..samples {
        let start = Instant::now();
        let frame = export_vp9::decode_ivf(black_box(&bytes)).expect("decode");
        black_box(&frame.y);
        nanos.push(start.elapsed().as_nanos());
    }
    nanos.sort_unstable();
    let p50 = nanos[rank(samples, 50)];
    let p99 = nanos[rank(samples, 99)];
    println!("file_bytes={}", bytes.len());
    println!("frame_bytes={frame_bytes}");
    println!("samples={samples}");
    println!("p50_ns={p50}");
    println!("p99_ns={p99}");
    ExitCode::SUCCESS
}

fn rank(n: u32, percentile: u32) -> usize {
    let index = (u64::from(percentile) * u64::from(n)).div_ceil(100);
    (index as usize).saturating_sub(1).min(n as usize - 1)
}
