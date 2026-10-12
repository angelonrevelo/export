//! Release p50/p99 for one AAC-LC stereo frame.
//!
//! Left is the half-scale 440 Hz sine. Right is silence. Same long sine
//! window as the mono frame. Writes the ADTS frame to stdout. Prints
//! counters on stderr. Pass a path as the first argument to also store the
//! file. Does not call FFmpeg.

use std::io::{self, Write};
use std::time::Instant;

const SAMPLES: usize = 200;

fn percentile(samples: &mut [u128], p: f64) -> u128 {
    samples.sort_unstable();
    let n = samples.len();
    let rank = (p * n as f64).ceil() as usize;
    samples[rank.clamp(1, n) - 1]
}

fn main() {
    let left_pcm = export_aac::one_frame_pcm();
    let right_pcm = [0.0f32; export_aac::FRAME_SAMPLES];
    let frame = export_aac::encode_stereo_frame(&left_pcm, &right_pcm).expect("encode");
    let decoded = export_aac::decode_stereo_frame(&frame).expect("decode");
    let mut left_err = 0.0f32;
    let mut left_peak = 0.0f32;
    let mut right_peak = 0.0f32;
    for ((src, left), right) in left_pcm
        .iter()
        .zip(decoded.left.iter())
        .zip(decoded.right.iter())
    {
        left_err = left_err.max((src - left).abs());
        left_peak = left_peak.max(left.abs());
        right_peak = right_peak.max(right.abs());
    }
    assert!(left_peak <= 0.51 || left_err <= 0.01);
    assert!(right_peak <= 0.01);

    let mut encode_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let encoded = export_aac::encode_stereo_frame(&left_pcm, &right_pcm).expect("encode");
        encode_ns.push(started.elapsed().as_nanos());
        assert_eq!(encoded.len(), frame.len());
    }
    let encode_p50 = percentile(&mut encode_ns, 0.50);
    let encode_p99 = percentile(&mut encode_ns, 0.99);

    let mut err = io::stderr().lock();
    let _ = writeln!(err, "samples={SAMPLES}");
    let _ = writeln!(err, "output_bytes={}", frame.len());
    let _ = writeln!(err, "encode_p50_ns={encode_p50}");
    let _ = writeln!(err, "encode_p99_ns={encode_p99}");
    let _ = writeln!(err, "left_max_abs_error={left_err}");
    let _ = writeln!(err, "left_peak={left_peak}");
    let _ = writeln!(err, "right_peak={right_peak}");

    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, &frame).unwrap_or_else(|e| panic!("write {path}: {e}"));
    }
    io::stdout().write_all(&frame).expect("stdout");
}
