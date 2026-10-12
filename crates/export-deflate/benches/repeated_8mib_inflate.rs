use std::time::Instant;

use export_deflate::{compress, inflate, repeated_text};

const BYTES: usize = 8_388_608;
const COMPRESSED_BYTES: usize = 30_449;
const WARMUP: usize = 3;
const SAMPLES: usize = 80;

fn main() {
    let data = repeated_text(BYTES);
    let compressed = compress(&data);
    assert_eq!(compressed.len(), COMPRESSED_BYTES);
    assert_eq!(inflate(&compressed).expect("inflate"), data);

    for _ in 0..WARMUP {
        assert_eq!(inflate(&compressed).expect("warmup"), data);
    }

    let mut inflate_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let decoded = inflate(&compressed).expect("sample");
        inflate_ns.push(start.elapsed().as_nanos());
        assert_eq!(decoded.len(), data.len());
    }

    inflate_ns.sort_unstable();
    println!("fixture repeated_sentence");
    println!("fixture_bytes {BYTES}");
    println!("compressed_bytes {}", compressed.len());
    println!("samples {SAMPLES} warmup {WARMUP}");
    println!(
        "inflate_p50_MBps {:.1}",
        mbps(BYTES, percentile(&inflate_ns, 0.50))
    );
    println!(
        "inflate_p99_MBps {:.1}",
        mbps(BYTES, percentile(&inflate_ns, 0.99))
    );
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index]
}

fn mbps(bytes: usize, nanos: u128) -> f64 {
    (bytes as f64) * 1_000.0 / (nanos as f64)
}
