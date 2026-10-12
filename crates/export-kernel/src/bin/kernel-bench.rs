//! Release-only latency. Byte counts are still judged in debug.
//!
//! ```text
//! cargo run --release -p export-kernel --bin kernel-bench
//! ```

use export_kernel::{sha256, write_box, Section};
use std::time::Instant;

fn main() {
    const N: usize = 8 * 1024 * 1024;
    let data = vec![0xA5u8; N];
    if cfg!(debug_assertions) {
        println!("hash latency: NOT JUDGED (debug build)");
    } else {
        let start = Instant::now();
        let mut acc = 0u8;
        for _ in 0..4 {
            acc ^= sha256(&data)[0];
        }
        let secs = start.elapsed().as_secs_f64() / 4.0;
        let mb_s = (N as f64) / secs / (1024.0 * 1024.0);
        println!("sha256 {mb_s:.1} MB/s (8 MiB, mean of 4) acc={acc}");
    }
    let section = Section {
        id: 1,
        bytes: data[..4096].to_vec(),
    };
    let boxed = write_box(&[section]);
    println!("box bytes {} for a 4096-byte section", boxed.len());
}
