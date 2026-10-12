//! Release p50/p99 and size for the one AAC-LC frame, including a truncated miss.
//!
//! Writes the ADTS frame to stdout. Prints counters on stderr.
//! Pass a path as the first argument to also store the frame there.

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
    let pcm = export_aac::one_frame_pcm();
    let frame = export_aac::encode_frame(&pcm).expect("encode");
    let decoded = export_aac::decode_frame(&frame).expect("decode");
    let mut max_abs = 0.0f32;
    for (a, b) in pcm.iter().zip(decoded.iter()) {
        max_abs = max_abs.max((a - b).abs());
    }

    let mut encode_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let encoded = export_aac::encode_frame(&pcm).expect("encode");
        encode_ns.push(started.elapsed().as_nanos());
        assert_eq!(encoded.len(), frame.len());
    }

    let truncated = &frame[..frame.len() - 1];
    let mut miss_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let missed = export_aac::decode_frame(truncated);
        miss_ns.push(started.elapsed().as_nanos());
        assert!(matches!(missed, Err(export_aac::Error::Truncated)));
    }

    let encode_p50 = percentile(&mut encode_ns, 0.50);
    let encode_p99 = percentile(&mut encode_ns, 0.99);
    let miss_p50 = percentile(&mut miss_ns, 0.50);
    let miss_p99 = percentile(&mut miss_ns, 0.99);

    let mut err = io::stderr().lock();
    let _ = writeln!(err, "samples={SAMPLES}");
    let _ = writeln!(err, "output_bytes={}", frame.len());
    let _ = writeln!(err, "encode_p50_ns={encode_p50}");
    let _ = writeln!(err, "encode_p99_ns={encode_p99}");
    let _ = writeln!(err, "max_abs_error={max_abs}");
    let _ = writeln!(err, "miss=1");
    let _ = writeln!(err, "miss_output_bytes=0");
    let _ = writeln!(err, "miss_p50_ns={miss_p50}");
    let _ = writeln!(err, "miss_p99_ns={miss_p99}");

    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, &frame).unwrap_or_else(|e| panic!("write {path}: {e}"));
    }
    io::stdout().write_all(&frame).expect("stdout");
}
