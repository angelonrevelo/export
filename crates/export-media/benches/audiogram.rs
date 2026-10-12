use export_media::{audiogram, audiogram_repeat, encode_wav, WavSpec};
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

struct CountAlloc;

static ALLOCS: AtomicU64 = AtomicU64::new(0);

unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountAlloc = CountAlloc;

struct Timed {
    secs: Vec<f64>,
    allocs: Vec<u64>,
}

fn measure(iters: usize, mut f: impl FnMut()) -> Timed {
    for _ in 0..3 {
        f();
    }
    let mut secs = Vec::with_capacity(iters);
    let mut allocs = Vec::with_capacity(iters);
    for _ in 0..iters {
        ALLOCS.store(0, Ordering::Relaxed);
        let t0 = Instant::now();
        f();
        let dt = t0.elapsed().as_secs_f64();
        allocs.push(ALLOCS.load(Ordering::Relaxed));
        secs.push(dt);
    }
    secs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Timed { secs, allocs }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

fn median_u64(values: &[u64]) -> u64 {
    let mut v = values.to_vec();
    v.sort_unstable();
    v[v.len() / 2]
}

fn report(name: &str, kind: &str, nbytes: u64, out_bytes: u64, timed: &Timed) {
    let p50 = pct(&timed.secs, 0.50);
    let p99 = pct(&timed.secs, 0.99);
    let mb = (nbytes as f64 / 1_000_000.0).max(1e-9);
    let allocs_per_mb = median_u64(&timed.allocs) as f64 / mb;
    if p50 >= 1e-6 {
        let mb_s = (nbytes as f64 / 1_000_000.0) / p50;
        println!(
            "{name:16} {kind:5} {mb_s:10.2} MB/s  p50 {p50:.6e} s  p99 {p99:.6e} s  allocs/MB {allocs_per_mb:.2}  out {out_bytes}"
        );
    } else {
        println!(
            "{name:16} {kind:5} {:>10} MB/s  p50 {p50:.6e} s  p99 {p99:.6e} s  allocs/MB {allocs_per_mb:.2}  out {out_bytes}",
            "n/a"
        );
    }
}

fn one_second() -> Vec<u8> {
    let rate = 48_000usize;
    let mut pcm = Vec::with_capacity(rate * 2);
    for i in 0..rate {
        let s = ((i as i16).wrapping_mul(17)).wrapping_add((i as i16) >> 3);
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    encode_wav(
        WavSpec {
            channels: 1,
            sample_rate: rate as u32,
            bits: 16,
        },
        &pcm,
    )
    .expect("wav")
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("refusing timing verdict: debug_assertions is on");
        std::process::exit(2);
    }
    const ITERS: usize = 31;
    let wav = one_second();
    let one = audiogram(&wav).expect("one frame");
    let repeated = audiogram_repeat(&wav, 30, 30).expect("30 fps");
    assert_eq!(one.video_samples, 1);
    assert_eq!(repeated.video_samples, 30);

    let hit = measure(ITERS, || {
        std::hint::black_box(audiogram(&wav).expect("one frame"));
    });
    report("audiogram", "1", wav.len() as u64, one.bytes, &hit);

    let many = measure(ITERS, || {
        std::hint::black_box(audiogram_repeat(&wav, 30, 30).expect("30 fps"));
    });
    report(
        "audiogram",
        "30fps",
        wav.len() as u64,
        repeated.bytes,
        &many,
    );
}
