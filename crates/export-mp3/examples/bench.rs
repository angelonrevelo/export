//! Release timings for one-frame decode, including the truncated-frame error.
//!
//! Run with `cargo run --release --example bench`. Percentiles are nearest-rank
//! on the sorted sample: index `floor(p * (n - 1))`.

use export_mp3::{decode_frame, encode_frame, FRAME_SAMPLES};
use std::time::Instant;

const ITERATIONS: usize = 1000;
const WARMUP: usize = 50;

fn main() {
    let silent = encode_frame(&[0.0; FRAME_SAMPLES]).expect("silence");
    let mut tone = [0.0f32; FRAME_SAMPLES];
    for (i, sample) in tone.iter_mut().enumerate() {
        *sample = (i as f32 * 0.05).sin() * 0.5;
    }
    let tone_frame = encode_frame(&tone).expect("tone");
    let truncated = &tone_frame[..tone_frame.len() / 2];

    let silence = time_ok(&silent);
    let tone_ns = time_ok(&tone_frame);
    let truncated_ns = time_err(truncated);

    println!("iterations={ITERATIONS}");
    println!("warmup={WARMUP}");
    println!("percentile=nearest-rank index=floor(p*(n-1))");
    println!("profile=release opt-level=3 lto=thin codegen-units=1");
    report("decode_silence_ns", &silence);
    report("decode_tone_ns", &tone_ns);
    report("decode_truncated_ns", &truncated_ns);
}

fn time_ok(frame: &[u8]) -> Vec<u128> {
    for _ in 0..WARMUP {
        std::hint::black_box(decode_frame(std::hint::black_box(frame)).unwrap());
    }
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        std::hint::black_box(decode_frame(std::hint::black_box(frame)).unwrap());
        samples.push(start.elapsed().as_nanos());
    }
    samples
}

fn time_err(frame: &[u8]) -> Vec<u128> {
    for _ in 0..WARMUP {
        std::hint::black_box(decode_frame(std::hint::black_box(frame)).unwrap_err());
    }
    let mut samples = Vec::with_capacity(ITERATIONS);
    for _ in 0..ITERATIONS {
        let start = Instant::now();
        std::hint::black_box(decode_frame(std::hint::black_box(frame)).unwrap_err());
        samples.push(start.elapsed().as_nanos());
    }
    samples
}

fn report(name: &str, samples: &[u128]) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let p50 = percentile(&sorted, 0.50);
    let p99 = percentile(&sorted, 0.99);
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    println!(
        "{name} n={} min={min} p50={p50} p99={p99} max={max}",
        sorted.len()
    );
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let index = ((sorted.len() - 1) as f64 * p).floor() as usize;
    sorted[index]
}
