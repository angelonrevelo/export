//! Percentiles for a twelve-sentence en ↔ tl paragraph.
//!
//! ```text
//! cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-tl-twelve
//! ```
//!
//! Same sample as `export-translate-tl-six`: one iteration of both directions,
//! divided by sentences × 2. The six-sentence bench is unchanged.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_translate::translate;

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

/// Nearest-rank percentile. `pct` is 50 or 99. `sorted` is ascending.
fn percentile(sorted: &[u128], pct: u128) -> u128 {
    let n = sorted.len() as u128;
    let rank = (pct * n).div_ceil(100);
    let idx = rank.saturating_sub(1) as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn main() {
    let en = "The father bought bread yesterday. She wrote a story at school. The boy drank water at home. We can read the book tomorrow. The girl played outside, but the cat is sleeping inside. I think the house is new. He opened the door today. They want to eat rice. The teacher came from the city. We should study the book tomorrow. My friend saw the bird yesterday. The children are playing at school.";
    let sentences = 12u32;

    let tl = translate(en, "en", "tl").expect("paragraph");
    println!("paragraph_en {en}");
    println!("paragraph_tl {tl}");
    let back = translate(&tl, "tl", "en").expect("reverse");
    println!("paragraph_en_back {back}");

    let warmup = 200u32;
    let mut w = 0;
    while w < warmup {
        black_box(translate(en, "en", "tl").unwrap());
        black_box(translate(&tl, "tl", "en").unwrap());
        w += 1;
    }

    let iters = 2000u32;
    let mut samples = Vec::with_capacity(iters as usize);
    let mut n = 0u32;
    while n < iters {
        let started = Instant::now();
        black_box(translate(en, "en", "tl").unwrap());
        black_box(translate(&tl, "tl", "en").unwrap());
        let per = started.elapsed().as_nanos() / (u128::from(sentences) * 2);
        samples.push(per);
        n += 1;
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 50);
    let p99 = percentile(&samples, 99);

    println!("pair en-tl");
    println!("directions en-tl tl-en");
    println!("paragraph_sentences {sentences}");
    println!("iterations {iters}");
    println!("ns_p50_per_sentence {p50}");
    println!("ns_p99_per_sentence {p99}");
    println!("us_p50_per_sentence {:.3}", (p50 as f64) / 1000.0);
    println!("us_p99_per_sentence {:.3}", (p99 as f64) / 1000.0);
}
