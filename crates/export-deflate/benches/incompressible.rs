use std::time::Instant;

use export_deflate::{compress, inflate};

const BYTES: usize = 1_048_576;
const WARMUP: usize = 3;
const SAMPLES: usize = 80;

fn main() {
    let data = incompressible(BYTES);
    let compressed = compress(&data);
    assert_eq!(inflate(&compressed).expect("inflate"), data);
    assert_eq!(compressed[0] & 0b110, 0, "expected a stored block");

    for _ in 0..WARMUP {
        let encoded = compress(&data);
        assert_eq!(inflate(&encoded).expect("warmup"), data);
    }

    let mut compress_ns = Vec::with_capacity(SAMPLES);
    let mut inflate_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let start = Instant::now();
        let encoded = compress(&data);
        compress_ns.push(start.elapsed().as_nanos());
        let start = Instant::now();
        let decoded = inflate(&encoded).expect("sample");
        inflate_ns.push(start.elapsed().as_nanos());
        assert_eq!(decoded.len(), data.len());
    }

    compress_ns.sort_unstable();
    inflate_ns.sort_unstable();
    println!("fixture incompressible");
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
    println!(
        "inflate_p50_MBps {:.1}",
        mbps(BYTES, percentile(&inflate_ns, 0.50))
    );
    println!(
        "inflate_p99_MBps {:.1}",
        mbps(BYTES, percentile(&inflate_ns, 0.99))
    );
}

fn incompressible(len: usize) -> Vec<u8> {
    let mut state = 0xA5A5_1234u32;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        out.push((state >> 24) as u8);
    }
    out
}

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let index = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[index]
}

fn mbps(bytes: usize, nanos: u128) -> f64 {
    (bytes as f64) * 1_000.0 / (nanos as f64)
}
