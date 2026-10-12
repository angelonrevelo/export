//! Release bench for the one-packet codec.
//!
//! Threading is off: this process never spawns a thread. Timings are a
//! single-threaded loop. Reported values are the 50th and 99th percentile
//! of per-call latency, including the truncated-packet error path.

use export_opus::{decode, encode, Error, FRAME_SAMPLES};
use std::hint::black_box;
use std::time::Instant;

const ITERS: usize = 20_000;
const WARMUP: usize = 1_000;

fn main() {
    // Threading is off: no pool, no spawn, one call at a time.
    println!("threading=off");
    println!("clock=std::time::Instant");

    let pcm = [0i16; FRAME_SAMPLES];
    let packet = encode(&pcm).expect("silence packet");

    bench("encode", || {
        black_box(encode(black_box(&pcm)).unwrap());
    });
    bench("decode", || {
        black_box(decode(black_box(&packet), 1, false).unwrap());
    });
    bench("truncated", || {
        let err = decode(black_box(&[]), 1, false).unwrap_err();
        assert_eq!(err, Error::Truncated);
        black_box(err);
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
