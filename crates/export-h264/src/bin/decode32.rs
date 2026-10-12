//! Time reading the 272-byte 32×32 I-frame back into its reconstruction.
//! Encode is outside the timer. Does not call ffmpeg.
//!
//! Usage: decode32 <decoded.yuv> <source.yuv> <recon.yuv>

use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Instant;

use export_h264::{decode_iframe, encode, fixture_32x32, EncodeOptions};

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let rank = ((sorted.len() as f64) * p).ceil() as usize;
    let i = rank.clamp(1, sorted.len()) - 1;
    sorted[i]
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let dec_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: decode32 <decoded.yuv> <source.yuv> <recon.yuv>");
            return ExitCode::from(2);
        }
    };
    let src_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: decode32 <decoded.yuv> <source.yuv> <recon.yuv>");
            return ExitCode::from(2);
        }
    };
    let recon_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: decode32 <decoded.yuv> <source.yuv> <recon.yuv>");
            return ExitCode::from(2);
        }
    };

    let frame = fixture_32x32();
    let encoded = match encode(&frame, &EncodeOptions::default()) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    let decoded = match decode_iframe(&encoded.annex_b) {
        Ok(y) => y,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    if decoded != encoded.reconstructed {
        eprintln!("decoded pixels are not the encoder reconstruction");
        return ExitCode::from(1);
    }

    let bytes = encoded.annex_b;
    for _ in 0..20 {
        let _ = black_box(decode_iframe(black_box(&bytes)));
    }
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        let t = Instant::now();
        let again = decode_iframe(black_box(&bytes)).expect("decode");
        let elapsed = t.elapsed().as_nanos();
        black_box(&again);
        samples.push(elapsed);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);

    if let Err(err) = fs::write(&dec_path, decoded.i420()) {
        eprintln!("write {dec_path}: {err}");
        return ExitCode::from(1);
    }
    if let Err(err) = fs::write(&src_path, frame.i420()) {
        eprintln!("write {src_path}: {err}");
        return ExitCode::from(1);
    }
    if let Err(err) = fs::write(&recon_path, encoded.reconstructed.i420()) {
        eprintln!("write {recon_path}: {err}");
        return ExitCode::from(1);
    }

    let mut out = io::stdout().lock();
    let _ = writeln!(
        out,
        "annex_bytes={}\nrelease_p50_ns={}\nrelease_p99_ns={}\nrelease_p50_us={}\nrelease_p99_us={}",
        bytes.len(),
        p50,
        p99,
        (p50 + 999) / 1000,
        (p99 + 999) / 1000
    );
    ExitCode::SUCCESS
}
