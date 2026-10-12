//! Release measurement on a paragraph that is not one of the stored fixtures.
//!
//! ```text
//! cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-paragraph
//! ```
//!
//! The fixture harness in `src/bin/bench.rs` is unchanged. This binary times the
//! same `translate` path on ordinary transcript text.

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
    let en = "Yesterday I talked to my friend about the meeting. She said we need to finish the report before Friday, but the train was late and we missed it. I think we can send the notes tomorrow.";
    let sentences = 3u32;

    let es = translate(en, "en", "es").expect("paragraph");
    println!("paragraph_en {en}");
    println!("paragraph_es {es}");

    let warmup = 200u32;
    let mut w = 0;
    while w < warmup {
        black_box(translate(en, "en", "es").unwrap());
        black_box(translate(&es, "es", "en").unwrap());
        w += 1;
    }

    let base = reset_peak();
    let iters = 2000u32;
    let started = Instant::now();
    let mut n = 0u32;
    while n < iters {
        black_box(translate(en, "en", "es").unwrap());
        black_box(translate(&es, "es", "en").unwrap());
        n += 1;
    }
    let elapsed = started.elapsed();
    let loop_heap = peak_delta(base);

    let one_base = reset_peak();
    black_box(translate(en, "en", "es").unwrap());
    let one_heap = peak_delta(one_base);

    let translations = (iters as u128) * (sentences as u128) * 2;
    let ns = elapsed.as_nanos();
    let ns_per = ns / translations;
    let us_per = (ns_per as f64) / 1000.0;

    println!("pair en-es");
    println!("directions en-es es-en");
    println!("paragraph_sentences {sentences}");
    println!("iterations {iters}");
    println!("translations {translations}");
    println!("ns_per_sentence {ns_per}");
    println!("us_per_sentence {us_per:.3}");
    println!("peak_heap_bytes {loop_heap}");
    println!("one_paragraph_heap_bytes {one_heap}");
    println!("lexicon_static_bytes {}", lexicon_static_bytes());
    match vmhwm_bytes() {
        Some(b) => println!("vmhwm_bytes {b}"),
        None => println!("vmhwm_bytes unavailable"),
    }
}
