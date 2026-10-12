use export_audio::{decode, encode, export, hash, Dedup, Job, Output, Pcm};
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
        System.dealloc(ptr, layout)
    }
}

#[global_allocator]
static GLOBAL: CountAlloc = CountAlloc;

struct Timed {
    secs: Vec<f64>,
    allocs: Vec<u64>,
}

fn time_once(mut f: impl FnMut()) -> (f64, u64) {
    ALLOCS.store(0, Ordering::Relaxed);
    let t0 = Instant::now();
    f();
    let dt = t0.elapsed().as_secs_f64();
    (dt, ALLOCS.load(Ordering::Relaxed))
}

fn measure(iters: usize, mut f: impl FnMut()) -> Timed {
    for _ in 0..3 {
        f();
    }
    let mut secs = Vec::with_capacity(iters);
    let mut allocs = Vec::with_capacity(iters);
    for _ in 0..iters {
        let (dt, n) = time_once(&mut f);
        secs.push(dt);
        allocs.push(n);
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

fn fixture() -> Pcm {
    let n = 1_048_576usize;
    let mut samples = Vec::with_capacity(n);
    for i in 0..n {
        let ramp = ((i % 256) as i32) - 128;
        let slow = ((i / 64) % 512) as i32 - 256;
        let wobble = ((i.wrapping_mul(i) >> 12) & 63) as i32;
        samples.push((slow * 40 + ramp * 8 + wobble).clamp(-32_768, 32_767));
    }
    Pcm {
        sample_rate: 44_100,
        channels: 1,
        bits: 16,
        samples,
    }
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("refusing timing verdict: debug_assertions is on");
        std::process::exit(2);
    }
    const ITERS: usize = 31;
    let pcm = fixture();
    let flac = encode(&pcm).expect("encode fixture");
    let back = decode(&flac).expect("decode fixture");
    assert_eq!(back.samples, pcm.samples, "bench fixture is not exact");
    let flac_len = flac.len() as u64;
    let pcm_len = (pcm.samples.len() * 2) as u64;

    let sha = measure(ITERS, || {
        std::hint::black_box(hash(&flac));
    });
    report("sha256", "hit", flac_len, &sha);

    let decode_miss = measure(ITERS, || {
        let mut dedup = Dedup::new();
        std::hint::black_box(export(&flac, Job::ToWav, &mut dedup).expect("miss"));
    });
    report("flac_decode", "miss", flac_len, &decode_miss);

    let mut seeded = Dedup::new();
    export(&flac, Job::ToWav, &mut seeded).expect("seed");
    let dedup_hit = measure(ITERS, || {
        let out = export(&flac, Job::ToWav, &mut seeded).expect("hit");
        assert!(matches!(out, Output::Duplicate { .. }));
    });
    report("dedup", "hit", flac_len, &dedup_hit);

    let encode_miss = measure(ITERS, || {
        std::hint::black_box(encode(&pcm).expect("encode"));
    });
    report("flac_encode", "miss", pcm_len, &encode_miss);

    let mut torn = flac.clone();
    torn.truncate(torn.len().saturating_sub(1).max(1));
    let truncated = measure(ITERS, || {
        std::hint::black_box(decode(&torn).err());
    });
    report("truncated", "miss", torn.len() as u64, &truncated);
}
