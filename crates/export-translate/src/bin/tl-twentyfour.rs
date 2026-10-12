//! Percentiles for a twenty-four-sentence en ↔ tl paragraph.
//!
//! ```text
//! cargo run --release --manifest-path crates/export-translate/Cargo.toml --bin export-translate-tl-twentyfour
//! ```
//!
//! Same sample as `export-translate-tl-twelve`: one iteration of both directions,
//! divided by sentences × 2. The twelve-sentence bench is unchanged.

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
    let en = "The doctor drove the car today. She sold bread at the market. The boy found the key at home. We can walk to the office tomorrow. The woman cooked fish, but the man is sleeping. I think the coffee is hot. He closed the window yesterday. They want to drink tea. The student came from school. We should buy food tomorrow. My mother saw the cat today. The children are working at home. The father called the doctor yesterday. She sent the letter today. The girl drank milk at school. We can eat rice tomorrow. The dog ran to the street, but the bird is sleeping. I think the tea is good. He read the note today. They need to help the family. The friend lived in the city. My father found money yesterday. She closed the door today. The children are reading at school.";
    let sentences = 24u32;

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
