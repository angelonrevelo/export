//! Release timing for the 160×160 I-frame plus one zero-motion P-frame.
//! Does not call ffmpeg.
//!
//! Usage: ipframe160 <annexb> <source.yuv> <recon.yuv>

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Instant;

use export_h264::{encode_ip, fixture_160x160, fixture_160x160_predicted, EncodeOptions};

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let rank = ((sorted.len() as f64) * p).ceil() as usize;
    let i = rank.clamp(1, sorted.len()) - 1;
    sorted[i]
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let annex = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: ipframe160 <annexb> <source.yuv> <recon.yuv>");
            return ExitCode::from(2);
        }
    };
    let yuv_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: ipframe160 <annexb> <source.yuv> <recon.yuv>");
            return ExitCode::from(2);
        }
    };
    let recon_path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: ipframe160 <annexb> <source.yuv> <recon.yuv>");
            return ExitCode::from(2);
        }
    };

    let intra = fixture_160x160();
    let predicted = fixture_160x160_predicted();
    let opts = EncodeOptions::default();
    let encoded = match encode_ip(&intra, &predicted, &opts) {
        Ok(e) => e,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };

    for _ in 0..20 {
        let _ = encode_ip(&intra, &predicted, &opts);
    }
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        let t = Instant::now();
        let again = encode_ip(&intra, &predicted, &opts).expect("160 pair encode");
        samples.push(t.elapsed().as_nanos());
        debug_assert_eq!(again.annex_b.len(), encoded.annex_b.len());
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);

    if let Err(err) = fs::write(&annex, &encoded.annex_b) {
        eprintln!("write {annex}: {err}");
        return ExitCode::from(1);
    }
    let mut yuv = intra.i420();
    yuv.extend(predicted.i420());
    if let Err(err) = fs::write(&yuv_path, yuv) {
        eprintln!("write {yuv_path}: {err}");
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
        "pictures=2\nmotion=0\noutput_bytes={}\nrelease_p50_us={}\nrelease_p99_us={}",
        encoded.annex_b.len(),
        p50.div_ceil(1000),
        p99.div_ceil(1000)
    );
    ExitCode::SUCCESS
}
