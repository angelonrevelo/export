//! Release time for one 24-bit quantize fixture. Does not remeasure LZW
//! decode or encode. Video-to-GIF is not on this path.

use std::time::Instant;

use export_gif::{quantize_fixture, quantize_gif89a, QUANTIZE_MAX_ABS};

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
    let raster = quantize_fixture();
    let pixels = usize::from(raster.width) * usize::from(raster.height);
    let once = quantize_gif89a(&raster).unwrap();
    for _ in 0..3 {
        let _ = quantize_gif89a(&raster).unwrap();
    }
    let (p50, p99) = time_n(30, || {
        let _ = quantize_gif89a(&raster).unwrap();
    });
    println!(
        "quantize_gif89a {}x{} pixels={pixels} samples=30 p50_us={:.1} p99_us={:.1} mae_r={:.3} mae_g={:.3} mae_b={:.3} max_abs={} {} {} bound={} {} {} file_bytes={}",
        raster.width,
        raster.height,
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        once.mae[0],
        once.mae[1],
        once.mae[2],
        once.max_abs[0],
        once.max_abs[1],
        once.max_abs[2],
        QUANTIZE_MAX_ABS[0],
        QUANTIZE_MAX_ABS[1],
        QUANTIZE_MAX_ABS[2],
        once.bytes.len(),
    );
}
