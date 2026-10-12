//! Release bench for the half-scale 440 Hz packet only.
//!
//! Threading is off. This binary does not time the silence packet.

use export_opus::{decode, encode, sine_source};
use std::hint::black_box;
use std::time::Instant;

const ITERS: usize = 20_000;
const WARMUP: usize = 1_000;

fn main() {
    println!("threading=off");
    println!("clock=std::time::Instant");
    println!("frame=half-scale-440hz");

    let pcm = sine_source();
    let packet = encode(&pcm).expect("sine packet");

    bench("sine-encode", || {
        black_box(encode(black_box(&pcm)).unwrap());
    });
    bench("sine-decode", || {
        black_box(decode(black_box(packet.as_slice()), 1, false).unwrap());
    });
}

fn bench(name: &str, mut call: impl FnMut()) {
    for _ in 0..WARMUP {
        call();
    }
    let mut samples = Vec::with_capacity(ITERS);
    for _ in 0..ITERS {
        let start = Instant::now();
        call();
        samples.push(start.elapsed().as_nanos() as u64);
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);
    println!("{name} p50={p50}ns p99={p99}ns n={ITERS}");
}

/// Nearest-rank percentile on a sorted slice. `p` is in `0.0..=1.0`.
fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let rank = (p * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
