//! Encode one sixteen-frame 256×256 GIF on the same path as the eight-frame row.
//! Does not remeasure that row or the four-frame 256×256 row.
//! Video-to-GIF is not called.

use std::time::Instant;

use export_gif::{decode_gif, encode_gif, encode_quantized_gif89a, quantize_frame_phase};

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let rank = ((sorted.len() as f64) * p).ceil() as usize;
    let i = rank.clamp(1, sorted.len()) - 1;
    sorted[i]
}

fn time_n(n: usize, mut f: impl FnMut()) -> (u128, u128) {
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        f();
        samples.push(t.elapsed().as_nanos());
    }
    samples.sort_unstable();
    (percentile(&samples, 0.50), percentile(&samples, 0.99))
}

fn main() {
    let frames: Vec<_> = (0..16)
        .map(|phase| quantize_frame_phase(256, 256, phase).unwrap())
        .collect();
    let delays = [4u16, 8, 15, 30, 4, 8, 15, 30, 4, 8, 15, 30, 4, 8, 15, 30];
    let once = encode_quantized_gif89a(&frames, &delays).unwrap();
    let gif = decode_gif(&once).unwrap();
    assert_eq!(gif.frames.len(), 16);
    let again = encode_gif(&gif).unwrap();
    let decoded_again = decode_gif(&again).unwrap();
    for (left, right) in gif.frames.iter().zip(&decoded_again.frames) {
        assert_eq!(left.indices, right.indices);
        assert_eq!(left.delay_cs, right.delay_cs);
    }
    for _ in 0..2 {
        let _ = encode_quantized_gif89a(&frames, &delays).unwrap();
    }
    let (p50, p99) = time_n(20, || {
        let _ = encode_quantized_gif89a(&frames, &delays).unwrap();
    });
    println!(
        "encode_quantized_gif89a 256x256 frames=16 output_bytes={} samples=20 p50_us={:.1} p99_us={:.1} round_trip=ok",
        once.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
    );
}
