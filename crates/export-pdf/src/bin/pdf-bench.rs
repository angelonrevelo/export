//! Release latency for a split of the existing two-page text file.
//!
//! ```text
//! cargo run --release -p export-pdf --bin pdf-bench
//! ```

use export_pdf::{page_count, split, text_pdf};
use std::time::Instant;

fn main() {
    let texts = ["Hello", "Second (line)"];
    let pdf = text_pdf(&texts);
    let expected: Vec<Vec<u8>> = texts.iter().map(|text| text_pdf(&[text])).collect();
    let pages = split(&pdf).expect("split");
    if pages != expected || page_count(&pdf).ok() != Some(texts.len()) {
        eprintln!("text differs");
        std::process::exit(1);
    }
    let page_count_n = texts.len();
    let bytes_in = pdf.len();
    let bytes_out: usize = pages.iter().map(Vec::len).sum();
    if cfg!(debug_assertions) {
        println!("text split: NOT JUDGED (debug build)");
        println!("page count {page_count_n}");
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
        if out != expected {
            eprintln!("text differs");
            std::process::exit(1);
        }
        times.push(ns);
    }
    times.sort_unstable();
    let p50 = times[(times.len() - 1) / 2];
    let p99 = times[(times.len() - 1) * 99 / 100];
    let check = split(&pdf).expect("split");
    if check != expected {
        eprintln!("text differs");
        std::process::exit(1);
    }
    println!("page count {page_count_n}");
    println!("bytes in {bytes_in}");
    println!("bytes out {bytes_out}");
    println!("p50 {p50} ns");
    println!("p99 {p99} ns");
}
