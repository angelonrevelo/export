//! Release-only latency. Byte counts are still judged in debug.
//!
//! ```text
//! cargo run --release -p export-kernel --bin kernel-bench
//! ```

use export_kernel::{inflate, sha256, write_box, Section};
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
    let compressed = [
        0x4b, 0x4c, 0x4a, 0x4e, 0x1c, 0x45, 0xa3, 0x88, 0xda, 0x08, 0x00,
    ];
    if cfg!(debug_assertions) {
        println!("inflate latency: NOT JUDGED (debug build)");
    } else {
        let start = Instant::now();
        let mut n = 0usize;
        for _ in 0..20 {
            n += inflate(&compressed).expect("inflate").len();
        }
        let secs = start.elapsed().as_secs_f64() / 20.0;
        let mb_s = (n as f64 / 20.0) / secs / (1024.0 * 1024.0);
        println!("inflate {mb_s:.1} MB/s (600 byte payload, mean of 20)");
    }
    let section = Section {
        id: 1,
        bytes: data[..4096].to_vec(),
    };
    let boxed = write_box(&[section]);
    println!("box bytes {} for a 4096-byte section", boxed.len());
}
