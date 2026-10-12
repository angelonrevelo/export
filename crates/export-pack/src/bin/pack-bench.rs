//! Release-only throughput of store-zip write and read.
//! Debug builds print NOT JUDGED and do not report a speed.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs;
use std::hint::black_box;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use export_pack::ZipReader;
use export_pack::ZipWriter;

const PAYLOAD: usize = 32 * 1024 * 1024;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

struct Sink(u64);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        black_box(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Sample {
    write: Duration,
    finish: Duration,
    read: Duration,
    write_allocs: u64,
    write_alloc_bytes: u64,
    read_allocs: u64,
    read_alloc_bytes: u64,
    bytes: u64,
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("NOT JUDGED");
        return;
    }
    let data = payload(PAYLOAD);
    let dir = PathBuf::from(env_dir());
    fs::create_dir_all(&dir).expect("bench dir");
    // Warm the file path once. The published numbers are the second pass.
    run(&data, &dir);
    let sample = run(&data, &dir);
    let mib = sample.bytes as f64 / 1024.0 / 1024.0;
    println!("profile=release");
    println!("payload_bytes={}", sample.bytes);
    println!("entries=1");
    println!("method=store");
    println!("chunk_bytes={}", export_pack::CHUNK_BYTES);
    println!("zip_write_mib_s={:.1}", mib_s(sample.bytes, sample.write));
    println!("zip_write_mb_s={:.1}", mb_s(sample.bytes, sample.write));
    println!("zip_finish_ms={:.1}", ms(sample.finish));
    println!("zip_read_mib_s={:.1}", mib_s(sample.bytes, sample.read));
    println!("zip_read_mb_s={:.1}", mb_s(sample.bytes, sample.read));
    println!(
        "zip_write_allocs_per_mib={:.3}",
        sample.write_allocs as f64 / mib
    );
    println!(
        "zip_write_alloc_bytes_per_mib={:.1}",
        sample.write_alloc_bytes as f64 / mib
    );
    println!(
        "zip_read_allocs_per_mib={:.3}",
        sample.read_allocs as f64 / mib
    );
    println!(
        "zip_read_alloc_bytes_per_mib={:.1}",
        sample.read_alloc_bytes as f64 / mib
    );
    println!("bench_dir={}", dir.display());
    println!("note=write is CRC plus local header plus payload, before fsync");
    println!("note=finish is central directory plus fsync plus rename");
    println!("note=read follows write in-process, so the page cache may hold the file");
}

fn run(data: &[u8], dir: &std::path::Path) -> Sample {
    let path = dir.join("bench.zip");
    let _ = fs::remove_file(&path);
    reset();
    let t0 = Instant::now();
    let mut zip = ZipWriter::create(&path).expect("create");
    zip.write_bytes("payload.bin", data).expect("write");
    let write = t0.elapsed();
    let (write_allocs, write_alloc_bytes) = snapshot();
    let t1 = Instant::now();
    zip.finish().expect("finish");
    let finish = t1.elapsed();

    reset();
    let t2 = Instant::now();
    let mut reader = ZipReader::open(&path).expect("open");
    let mut sink = Sink(0);
    let n = reader.copy_entry(0, &mut sink).expect("read");
    let read = t2.elapsed();
    let (read_allocs, read_alloc_bytes) = snapshot();
    assert_eq!(n, data.len() as u64);
    assert_eq!(sink.0, data.len() as u64);
    let _ = fs::remove_file(&path);
    Sample {
        write,
        finish,
        read,
        write_allocs,
        write_alloc_bytes,
        read_allocs,
        read_alloc_bytes,
        bytes: data.len() as u64,
    }
}

fn payload(n: usize) -> Vec<u8> {
    let mut data = vec![0u8; n];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(3).wrapping_add(1);
    }
    data
}

fn env_dir() -> String {
    std::env::var("EXPORT_PACK_BENCH_DIR").unwrap_or_else(|_| "target/pack-bench".to_owned())
}

fn reset() {
    ALLOCS.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
}

fn snapshot() -> (u64, u64) {
    (
        ALLOCS.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

fn mib_s(bytes: u64, dur: Duration) -> f64 {
    (bytes as f64 / 1024.0 / 1024.0) / dur.as_secs_f64()
}

fn mb_s(bytes: u64, dur: Duration) -> f64 {
    (bytes as f64 / 1_000_000.0) / dur.as_secs_f64()
}

fn ms(dur: Duration) -> f64 {
    dur.as_secs_f64() * 1000.0
}
