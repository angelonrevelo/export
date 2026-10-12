//! Time reading the shifted 32×32 pair, quarter-pel (−16, 0).
//! Encode is outside the timer. Does not call ffmpeg.
//!
//! Usage: decodeshiftright32 <decoded.yuv> <recon.yuv>

use std::env;
use std::fs;
use std::hint::black_box;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Instant;

use export_h264::{
    decode_motion, encode_ip_motion, fixture_32x32, fixture_32x32_shifted, EncodeOptions,
    SPLIT_RIGHT_MV,
};

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
            eprintln!("usage: decodeshiftright32 <decoded.yuv> <recon.yuv>");
            return ExitCode::from(2);
        }
    };
    let recon_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: decodeshiftright32 <decoded.yuv> <recon.yuv>");
            return ExitCode::from(2);
        }
    };

    let intra = fixture_32x32();
    let shifted = fixture_32x32_shifted();
    let encoded = match encode_ip_motion(&intra, &shifted, &EncodeOptions::default(), SPLIT_RIGHT_MV)
    {
        Ok(e) => e,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    if encoded.annex_b.len() < 5 || encoded.annex_b[0..4] != [0, 0, 0, 1] {
        eprintln!("pair is not Annex B");
        return ExitCode::from(1);
    }
    let (dec_i, dec_p) = match decode_motion(&encoded.annex_b) {
        Ok(pictures) => pictures,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };
    if dec_i != encoded.intra || dec_p != encoded.predicted {
        eprintln!("decoded pixels are not the encoder reconstruction");
        return ExitCode::from(1);
    }

    let bytes = encoded.annex_b;
    for _ in 0..20 {
        let _ = black_box(decode_motion(black_box(&bytes)));
    }
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        let t = Instant::now();
        let again = decode_motion(black_box(&bytes)).expect("decode motion");
        let elapsed = t.elapsed().as_nanos();
        black_box(&again);
        samples.push(elapsed);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);

    let mut decoded = dec_i.i420();
    decoded.extend(dec_p.i420());
    if let Err(err) = fs::write(&dec_path, decoded) {
        eprintln!("write {dec_path}: {err}");
        return ExitCode::from(1);
    }
    let mut recon = encoded.intra.i420();
    recon.extend(encoded.predicted.i420());
    if let Err(err) = fs::write(&recon_path, recon) {
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
