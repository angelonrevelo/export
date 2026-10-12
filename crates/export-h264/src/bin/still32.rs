//! Time returning the 32×32 I-frame reconstruction as packed I420.
//! Encode is outside the timer. Does not call ffmpeg. Does not read Annex B.
//!
//! Usage: still32 <still.yuv> <source.yuv>

use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Instant;

use export_h264::{encode, fixture_32x32, still, EncodeOptions};

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let rank = ((sorted.len() as f64) * p).ceil() as usize;
    let i = rank.clamp(1, sorted.len()) - 1;
    sorted[i]
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let still_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: still32 <still.yuv> <source.yuv>");
            return ExitCode::from(2);
        }
    };
    let src_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: still32 <still.yuv> <source.yuv>");
            return ExitCode::from(2);
        }
    };

    let frame = fixture_32x32();
    let opts = EncodeOptions::default();
    let encoded = match encode(&frame, &opts) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    let pixels = still(&encoded);
    if pixels != encoded.reconstructed.i420() {
        eprintln!("still pixels are not the encoder reconstruction");
        return ExitCode::from(1);
    }

    for _ in 0..20 {
        black_box(still(black_box(&encoded)));
    }
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        let t = Instant::now();
        let again = still(black_box(&encoded));
        let elapsed = t.elapsed().as_nanos();
        black_box(&again);
        samples.push(elapsed);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);

    if let Err(err) = fs::write(&still_path, &pixels) {
        eprintln!("write {still_path}: {err}");
        return ExitCode::from(1);
    }
    if let Err(err) = fs::write(&src_path, frame.i420()) {
        eprintln!("write {src_path}: {err}");
        return ExitCode::from(1);
    }

    let mut out = io::stdout().lock();
    let _ = writeln!(
        out,
        "raw_bytes={}\nrelease_p50_ns={}\nrelease_p99_ns={}\nrelease_p50_us={}\nrelease_p99_us={}",
        pixels.len(),
        p50,
        p99,
        (p50 + 999) / 1000,
        (p99 + 999) / 1000
    );
    ExitCode::SUCCESS
}
