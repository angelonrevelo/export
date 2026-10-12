use std::time::Instant;

use export_deflate::{compress, inflate, repeated_text};

const BYTES: usize = 8_388_608;
const WARMUP: usize = 3;
const SAMPLES: usize = 80;

fn main() {
    let data = repeated_text(BYTES);
    let compressed = compress(&data);
    assert_eq!(inflate(&compressed).expect("inflate"), data);
    assert!(compressed.len() < data.len());

    for _ in 0..WARMUP {
        let encoded = compress(&data);
        assert_eq!(encoded.len(), compressed.len());
    }

    let mut compress_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let encoded = compress(&data);
        compress_ns.push(start.elapsed().as_nanos());
        assert_eq!(encoded.len(), compressed.len());
    }

    compress_ns.sort_unstable();
    println!("fixture repeated_sentence");
    println!("fixture_bytes {BYTES}");
    println!("compressed_bytes {}", compressed.len());
    println!("samples {SAMPLES} warmup {WARMUP}");
    println!(
        "compress_p50_MBps {:.1}",
        mbps(BYTES, percentile(&compress_ns, 0.50))
    );
    println!(
        "compress_p99_MBps {:.1}",
        mbps(BYTES, percentile(&compress_ns, 0.99))
    );
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index]
}

fn mbps(bytes: usize, nanos: u128) -> f64 {
    (bytes as f64) * 1_000.0 / (nanos as f64)
}
