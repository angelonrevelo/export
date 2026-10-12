//! Release latency for the existing reverse merge of the two-page text file.
//!
//! ```text
//! cargo run --release -p export-pdf --bin pdf-bench
//! ```

use export_pdf::{merge, page_count, split, text_pdf};
use std::time::Instant;

fn main() {
    let pdf = text_pdf(&["Hello", "Second (line)"]);
    let pages = split(&pdf).expect("split");
    let expected = text_pdf(&["Second (line)", "Hello"]);
    let merged = merge(&[&pages[1], &pages[0]]).expect("merge");
    if merged != expected || page_count(&merged).ok() != Some(2) {
        eprintln!("text differs");
        std::process::exit(1);
    }
    let page_count_n = 2;
    let bytes_in = pages[0].len() + pages[1].len();
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
        let _ = merge(&[&pages[1], &pages[0]]).expect("warm");
    }
    let mut times = Vec::with_capacity(N);
    for _ in 0..N {
        let start = Instant::now();
        let out = merge(&[&pages[1], &pages[0]]).expect("merge");
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
    let check = merge(&[&pages[1], &pages[0]]).expect("merge");
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
