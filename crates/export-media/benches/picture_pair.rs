use export_media::{audiogram, concat_mp4, elementary, encode_wav, parse, WavSpec};
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
}

fn measure(iters: usize, mut f: impl FnMut()) -> Timed {
    for _ in 0..3 {
        f();
    }
    let mut secs = Vec::with_capacity(iters);
    for _ in 0..iters {
        ALLOCS.store(0, Ordering::Relaxed);
        let t0 = Instant::now();
        f();
        secs.push(t0.elapsed().as_secs_f64());
    }
    secs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    Timed { secs }
}

fn pct(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
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

fn pictures(file: &[u8]) -> Vec<u8> {
    let movie = parse(file).expect("parse");
    let video = movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"vide")
        .expect("video");
    let samples = elementary(video, file).expect("pictures");
    assert_eq!(samples.len(), 2);
    let mut out = Vec::with_capacity(samples[0].len() + samples[1].len());
    out.extend_from_slice(samples[0]);
    out.extend_from_slice(samples[1]);
    out
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("refusing timing verdict: debug_assertions is on");
        std::process::exit(2);
    }
    const ITERS: usize = 31;
    let one = audiogram(&one_second()).expect("audiogram");
    assert_eq!(one.bytes, 327_420);
    let pair = concat_mp4(&one.data, &one.data).expect("pair");
    assert_eq!(pair.len(), 653_824);
    let bytes = pictures(&pair);
    assert_eq!(bytes.len(), 460_800);
    assert_eq!(&bytes[..230_400], &bytes[230_400..]);

    let timed = measure(ITERS, || {
        std::hint::black_box(pictures(&pair));
    });
    let p50 = pct(&timed.secs, 0.50);
    let p99 = pct(&timed.secs, 0.99);
    println!(
        "picture_pair   samples 2  bytes {out}  p50 {p50:.6e} s  p99 {p99:.6e} s",
        out = bytes.len()
    );
    if p50 > 12.2e-6 {
        eprintln!("p50 {p50:.6e} s is over 12.2 µs");
        std::process::exit(1);
    }
}
