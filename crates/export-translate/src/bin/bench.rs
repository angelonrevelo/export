//! Release measurement: time per sentence and peak bytes.
//!
//! ```text
//! cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-bench
//! ```
//!
//! The library has no allocator hook. This binary does, so the number is the
//! high-water of bytes outstanding during `translate`, plus VmHWM for the process.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_translate::{lexicon_static_bytes, translate};

struct PeakAlloc;

static CURRENT: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for PeakAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = System.alloc(layout);
        if !ptr.is_null() {
            note_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
        CURRENT.fetch_sub(layout.size(), Ordering::Relaxed);
    }
}

fn note_alloc(size: usize) {
    let now = CURRENT.fetch_add(size, Ordering::Relaxed) + size;
    let mut peak = PEAK.load(Ordering::Relaxed);
    while now > peak {
        match PEAK.compare_exchange_weak(peak, now, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => break,
            Err(seen) => peak = seen,
        }
    }
}

#[global_allocator]
static GLOBAL: PeakAlloc = PeakAlloc;

fn reset_peak() -> usize {
    let base = CURRENT.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    base
}

fn peak_delta(base: usize) -> usize {
    PEAK.load(Ordering::Relaxed).saturating_sub(base)
}

fn vmhwm_bytes() -> Option<u64> {
    let text = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in text.lines() {
        let Some(rest) = line.strip_prefix("VmHWM:") else {
            continue;
        };
        let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
        return Some(kb * 1024);
    }
    None
}

fn main() {
    let en = [
        "The weather is nice today and the children are playing outside.",
        "Please open the file and save the document.",
        "Hello, how are you today?",
        "She wants water.",
        "The small house is new.",
        "The children want water.",
        "The big house is new.",
        "I want water.",
        "We are playing outside.",
        "The house is not new.",
        "The children are playing in the house.",
        "The small houses are new.",
        "The child is small.",
        "Open the file.",
        "How are you today?",
        "Thank you.",
        "Good morning.",
    ];

    let mut es = Vec::with_capacity(en.len());
    for s in en {
        es.push(translate(s, "en", "es").expect(s));
    }

    let warmup = 200u32;
    let mut w = 0;
    while w < warmup {
        let mut i = 0;
        while i < en.len() {
            black_box(translate(en[i], "en", "es").unwrap());
            black_box(translate(&es[i], "es", "en").unwrap());
            i += 1;
        }
        w += 1;
    }

    let base = reset_peak();
    let iters = 2000u32;
    let started = Instant::now();
    let mut n = 0u32;
    while n < iters {
        let mut i = 0;
        while i < en.len() {
            black_box(translate(en[i], "en", "es").unwrap());
            black_box(translate(&es[i], "es", "en").unwrap());
            i += 1;
        }
        n += 1;
    }
    let elapsed = started.elapsed();
    let loop_heap = peak_delta(base);

    let one_base = reset_peak();
    black_box(translate(en[0], "en", "es").unwrap());
    let one_heap = peak_delta(one_base);

    let sentences = (iters as u128) * (en.len() as u128) * 2;
    let ns = elapsed.as_nanos();
    let ns_per = ns / sentences;
    let us_per = (ns_per as f64) / 1000.0;

    println!("pair en-es");
    println!("directions en-es es-en");
    println!("fixture_sentences {}", en.len());
    println!("iterations {iters}");
    println!("translations {sentences}");
    println!("ns_per_sentence {ns_per}");
    println!("us_per_sentence {us_per:.3}");
    println!("peak_heap_bytes {loop_heap}");
    println!("one_sentence_heap_bytes {one_heap}");
    println!("lexicon_static_bytes {}", lexicon_static_bytes());
    match vmhwm_bytes() {
        Some(b) => println!("vmhwm_bytes {b}"),
        None => println!("vmhwm_bytes unavailable"),
    }
}
