use export_media::{
    build_movie, concat_mp4, encode_wav, shrink_wav, MovieSpec, TrackSpec, WavSpec,
};
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

fn report(name: &str, kind: &str, nbytes: u64, timed: &Timed) {
    let p50 = pct(&timed.secs, 0.50);
    let p99 = pct(&timed.secs, 0.99);
    let mb = (nbytes as f64 / 1_000_000.0).max(1e-9);
    let allocs_per_mb = median_u64(&timed.allocs) as f64 / mb;
    if p50 >= 1e-6 {
        let mb_s = (nbytes as f64 / 1_000_000.0) / p50;
        println!(
            "{name:16} {kind:4} {mb_s:10.2} MB/s  p50 {p50:.6e} s  p99 {p99:.6e} s  allocs/MB {allocs_per_mb:.2}  bytes {nbytes}"
        );
    } else {
        println!(
            "{name:16} {kind:4} {:>10} MB/s  p50 {p50:.6e} s  p99 {p99:.6e} s  allocs/MB {allocs_per_mb:.2}  bytes {nbytes}",
            "n/a"
        );
    }
}

fn movie(codec: [u8; 4]) -> Vec<u8> {
    let n = 64usize;
    let payload = vec![0x3cu8; 16 * 1024];
    let video: Vec<Vec<u8>> = (0..n).map(|_| payload.clone()).collect();
    let audio: Vec<Vec<u8>> = (0..32).map(|i| (i as i16).to_le_bytes().to_vec()).collect();
    build_movie(&MovieSpec {
        brand: *b"isom",
        timescale: 1000,
        faststart: false,
        interleave: false,
        tracks: vec![
            TrackSpec {
                handler: *b"vide",
                codec: *b"raw ",
                timescale: 30,
                width: 16,
                height: 16,
                channels: 0,
                bits: 0,
                sample_rate: 0,
                deltas: vec![1; n],
                payloads: video,
            },
            TrackSpec {
                handler: *b"soun",
                codec,
                timescale: 32,
                width: 0,
                height: 0,
                channels: 1,
                bits: 16,
                sample_rate: 32,
                deltas: vec![1; 32],
                payloads: audio,
            },
        ],
    })
    .expect("movie")
}

fn wav() -> Vec<u8> {
    let frames = 250_000usize;
    let mut pcm = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let s = (i as i16).wrapping_mul(5);
        pcm.extend_from_slice(&s.to_le_bytes());
        pcm.extend_from_slice(&s.wrapping_neg().to_le_bytes());
    }
    encode_wav(
        WavSpec {
            channels: 2,
            sample_rate: 48_000,
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
    let left = movie(*b"sowt");
    let right = movie(*b"sowt");
    let mismatch = movie(*b"twos");
    let pcm = wav();
    let concat_bytes = (left.len() + right.len()) as u64;
    let miss_bytes = (left.len() + mismatch.len()) as u64;

    let hit = measure(ITERS, || {
        std::hint::black_box(concat_mp4(&left, &right).expect("concat"));
    });
    report("concat", "hit", concat_bytes, &hit);

    let miss = measure(ITERS, || {
        std::hint::black_box(concat_mp4(&left, &mismatch).expect_err("codec mismatch"));
    });
    report("concat", "miss", miss_bytes, &miss);

    let shrunk = measure(ITERS, || {
        std::hint::black_box(shrink_wav(&pcm, 200_000).expect("shrink"));
    });
    report("wav_shrink", "hit", pcm.len() as u64, &shrunk);

    let shrink_miss = measure(ITERS, || {
        std::hint::black_box(shrink_wav(&left, 200_000).expect_err("not wav"));
    });
    report("wav_shrink", "miss", left.len() as u64, &shrink_miss);
}
