//! Release latency for a split of one 16×16 gray page.
//!
//! ```text
//! cargo run --release -p export-pdf --bin pdf-bench
//! ```

use export_pdf::{gray_pdf, gray_samples, split, GRAY_EDGE};
use std::time::Instant;

fn main() {
    let width = GRAY_EDGE;
    let height = GRAY_EDGE;
    let mut samples = vec![0u8; (width * height) as usize];
    for (i, byte) in samples.iter_mut().enumerate() {
        *byte = (i * 13 + 7) as u8;
    }
    let pdf = gray_pdf(width, height, &samples).expect("gray page");
    let pages = split(&pdf).expect("split");
    let (got_w, got_h, got) = gray_samples(&pages[0]).expect("samples");
    if pages.len() != 1 || got_w != width || got_h != height || got != samples {
        eprintln!("pixels differ");
        std::process::exit(1);
    }
    let bytes_in = pdf.len();
    let bytes_out = pages[0].len();
    if cfg!(debug_assertions) {
        println!("rgb split: NOT JUDGED (debug build)");
        println!("bytes in {bytes_in}");
        println!("bytes out {bytes_out}");
        return;
    }
    const N: usize = 201;
    for _ in 0..20 {
        let _ = split(&pdf).expect("warm");
    }
    let mut times = Vec::with_capacity(N);
    for _ in 0..N {
        let start = Instant::now();
        let out = split(&pdf).expect("split");
        let ns = start.elapsed().as_nanos();
        if out.len() != 1 || out[0].len() != bytes_out {
            eprintln!("pixels differ");
            std::process::exit(1);
        }
        times.push(ns);
    }
    times.sort_unstable();
    let p50 = times[(times.len() - 1) / 2];
    let p99 = times[(times.len() - 1) * 99 / 100];
    let check = split(&pdf).expect("split");
    let (_, _, after) = gray_samples(&check[0]).expect("samples");
    if after != samples {
        eprintln!("pixels differ");
        std::process::exit(1);
    }
    println!("bytes in {bytes_in}");
    println!("bytes out {bytes_out}");
    println!("p50 {p50} ns");
    println!("p99 {p99} ns");
}
