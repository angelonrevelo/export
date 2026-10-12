//! Release latency for a reverse merge of a four-page text file.
//!
//! ```text
//! cargo run --release -p export-pdf --bin pdf-bench
//! ```

use export_pdf::{merge, page_count, split, text_pdf};
use std::time::Instant;

fn main() {
    let mut texts = ["Hello", "Second (line)", "Hello", "Second (line)"];
    let pdf = text_pdf(&texts);
    let pages = split(&pdf).expect("split");
    texts.reverse();
    let expected = text_pdf(&texts);
    let parts: Vec<&[u8]> = pages.iter().rev().map(Vec::as_slice).collect();
    let merged = merge(&parts).expect("merge");
    if merged != expected || page_count(&merged).ok() != Some(texts.len()) {
        eprintln!("text differs");
        std::process::exit(1);
    }
    let page_count_n = texts.len();
    let bytes_in: usize = pages.iter().map(Vec::len).sum();
    let bytes_out = merged.len();
    if cfg!(debug_assertions) {
        println!("text merge: NOT JUDGED (debug build)");
        println!("page count {page_count_n}");
        println!("bytes in {bytes_in}");
        println!("bytes out {bytes_out}");
        return;
    }
    const N: usize = 201;
    for _ in 0..20 {
        let _ = merge(&parts).expect("warm");
    }
    let mut times = Vec::with_capacity(N);
    for _ in 0..N {
        let start = Instant::now();
        let out = merge(&parts).expect("merge");
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
    let check = merge(&parts).expect("merge");
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
