//! Release time and peak requested heap for `decrypt_ooxml`.
//!
//! The ciphertext and the expected plaintext are allocated before the clock
//! starts. `peak_extra` is the high-water mark of bytes requested from the
//! allocator during decrypt, minus that baseline. It is not RSS.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::{self, Cursor, Write};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_meta::{decrypt_ooxml, encrypt_ooxml, encrypt_ooxml_agile};

struct Peak;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Peak {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            charge(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        discharge(layout.size());
        System.dealloc(ptr, layout);
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc_zeroed(layout);
        if !ptr.is_null() {
            charge(layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = System.realloc(ptr, layout, new_size);
        if !new_ptr.is_null() {
            charge(new_size);
            discharge(layout.size());
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOC: Peak = Peak;

fn charge(n: usize) {
    let now = CURRENT.fetch_add(n, Ordering::Relaxed) + n;
    let mut peak = PEAK.load(Ordering::Relaxed);
    while now > peak {
        match PEAK.compare_exchange_weak(peak, now, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(seen) => peak = seen,
        }
    }
}

fn discharge(n: usize) {
    CURRENT.fetch_sub(n, Ordering::Relaxed);
}

struct Check<'a> {
    expect: &'a [u8],
    pos: usize,
}

impl Write for Check<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let end = self.pos + buf.len();
        if end > self.expect.len() || buf != &self.expect[self.pos..end] {
            return Err(io::Error::new(io::ErrorKind::Other, "plaintext mismatch"));
        }
        self.pos += buf.len();
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn decrypt_once(file: &[u8], plain: &[u8]) {
    let mut out = Check {
        expect: plain,
        pos: 0,
    };
    decrypt_ooxml(&mut Cursor::new(file), "bench-password", &mut out).unwrap();
    assert_eq!(out.pos, plain.len());
}

fn measure(kind: &str, file: &[u8], plain: &[u8], judged: bool) {
    decrypt_once(file, plain);
    let mut times = Vec::with_capacity(5);
    let mut extras = Vec::with_capacity(5);
    for _ in 0..5 {
        let before = CURRENT.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let started = Instant::now();
        decrypt_once(file, plain);
        let elapsed = started.elapsed().as_nanos();
        let extra = PEAK.load(Ordering::Relaxed).saturating_sub(before);
        times.push(elapsed);
        extras.push(extra);
    }
    times.sort_unstable();
    extras.sort_unstable();
    let median = times[times.len() / 2];
    let peak = extras[extras.len() / 2];
    let n = plain.len();
    if !judged {
        println!("NOT JUDGED {kind} plain={n} peak_extra={peak}");
        return;
    }
    let ms = median as f64 / 1_000_000.0;
    let mb_s = (n as f64) / (median as f64 / 1_000_000_000.0) / 1_000_000.0;
    println!(
        "{kind} plain={n} file={} median_ns={median} median_ms={ms:.3} peak_extra={peak} mb_s={mb_s:.2}",
        file.len()
    );
    assert!(
        peak < 64 * 1024,
        "{kind} allocated {peak} bytes on a {n}-byte package"
    );
}

fn main() {
    let salt = [0x11u8; 16];
    let judged = !cfg!(debug_assertions);
    for &n in &[64 * 1024usize, 1024 * 1024] {
        let mut plain = vec![0x5au8; n];
        plain[..4].copy_from_slice(b"PK\x03\x04");
        let mut file = Vec::new();
        encrypt_ooxml("bench-password", &salt, &plain, &mut file).unwrap();
        measure("standard", &file, &plain, judged);
    }
    let password_salt = [0x22u8; 16];
    let key_salt = [0x33u8; 16];
    for &n in &[64 * 1024usize, 1024 * 1024] {
        let mut plain = vec![0x5au8; n];
        plain[..4].copy_from_slice(b"PK\x03\x04");
        let mut file = Vec::new();
        encrypt_ooxml_agile(
            "bench-password",
            &password_salt,
            &key_salt,
            &plain,
            &mut file,
        )
        .unwrap();
        measure("agile", &file, &plain, judged);
    }
    measure_agile_four_mib(&password_salt, &key_salt, judged);
}

fn measure_agile_four_mib(password_salt: &[u8; 16], key_salt: &[u8; 16], judged: bool) {
    let n = 4 * 1024 * 1024;
    let mut plain = vec![0x5au8; n];
    plain[..4].copy_from_slice(b"PK\x03\x04");
    let mut file = Vec::new();
    encrypt_ooxml_agile("bench-password", password_salt, key_salt, &plain, &mut file).unwrap();
    decrypt_once(&file, &plain);
    let mut times = Vec::with_capacity(100);
    for _ in 0..100 {
        let before = CURRENT.load(Ordering::Relaxed);
        PEAK.store(before, Ordering::Relaxed);
        let started = Instant::now();
        decrypt_once(&file, &plain);
        let elapsed = started.elapsed().as_nanos() as f64;
        let extra = PEAK.load(Ordering::Relaxed).saturating_sub(before);
        assert!(extra < 64 * 1024, "agile 4MiB allocated {extra} bytes");
        times.push(elapsed);
    }
    times.sort_unstable_by(|a, b| a.partial_cmp(b).unwrap());
    let p50 = percentile(&times, 50);
    let p99 = percentile(&times, 99);
    let ms = p50 / 1_000_000.0;
    let mb_p50 = mb_per_s(n, p50);
    let mb_p99 = mb_per_s(n, p99);
    if !judged {
        println!("NOT JUDGED agile plain={n} p50_ms={ms:.3}");
        return;
    }
    println!(
        "agile plain={n} file={} p50_ms={ms:.3} mb_s_p50={mb_p50:.2} mb_s_p99={mb_p99:.2}",
        file.len()
    );
    assert!(
        mb_p50 >= 17.4,
        "agile 4MiB throughput p50 {mb_p50:.2} MB/s is below 17.4"
    );
}

fn mb_per_s(bytes: usize, ns: f64) -> f64 {
    (bytes as f64) / (ns / 1_000_000_000.0) / 1_000_000.0
}

fn percentile(sorted: &[f64], pct: usize) -> f64 {
    let rank = ((pct as f64 / 100.0) * (sorted.len() as f64 - 1.0)).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}
