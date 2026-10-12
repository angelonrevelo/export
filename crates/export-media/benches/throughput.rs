use export_media::{
    build_movie, export, hash, parse, sniff, Dedup, Job, MovieSpec, Output, TrackSpec,
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

fn report(name: &str, kind: &str, nbytes: u64, timed: &Timed, scan: bool) {
    let p50 = pct(&timed.secs, 0.50);
    let p99 = pct(&timed.secs, 0.99);
    let mb = (nbytes as f64 / 1_000_000.0).max(1e-9);
    let allocs_per_mb = median_u64(&timed.allocs) as f64 / mb;
    // Sub-microsecond samples sit on the timer floor. Do not publish MB/s for them.
    if scan && p50 >= 1e-6 {
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

fn fixture() -> Vec<u8> {
    let n = 256usize;
    let payload = vec![0x5au8; 16 * 1024];
    let video: Vec<Vec<u8>> = (0..n).map(|_| payload.clone()).collect();
    let audio: Vec<Vec<u8>> = (0..64).map(|i| (i as i16).to_le_bytes().to_vec()).collect();
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
                width: 32,
                height: 32,
                channels: 0,
                bits: 0,
                sample_rate: 0,
                deltas: vec![1; n],
                payloads: video,
            },
            TrackSpec {
                handler: *b"soun",
                codec: *b"sowt",
                timescale: 64,
                width: 0,
                height: 0,
                channels: 1,
                bits: 16,
                sample_rate: 64,
                deltas: vec![1; 64],
                payloads: audio,
            },
        ],
    })
    .expect("fixture")
}

fn miss_bytes() -> Vec<u8> {
    let mut miss = Vec::with_capacity(4 * 1024 * 1024);
    let body = b"isom\0\0\0\0isom";
    let size = (8 + body.len()) as u32;
    miss.extend_from_slice(&size.to_be_bytes());
    miss.extend_from_slice(b"ftyp");
    miss.extend_from_slice(body);
    miss.extend_from_slice(&0x0fff_ffffu32.to_be_bytes());
    miss.extend_from_slice(b"mdat");
    miss.resize(4 * 1024 * 1024, 0);
    miss
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("refusing timing verdict: debug_assertions is on");
        std::process::exit(2);
    }
    const ITERS: usize = 31;
    let file = fixture();
    let nbytes = file.len() as u64;
    let zeros = vec![0u8; file.len()];
    let miss = miss_bytes();

    let sha = measure(ITERS, || {
        std::hint::black_box(hash(&file));
    });
    report("sha256", "hit", nbytes, &sha, true);

    let sniff_hit = measure(ITERS, || {
        std::hint::black_box(sniff(&file));
    });
    report("sniff", "hit", nbytes, &sniff_hit, false);

    let sniff_miss = measure(ITERS, || {
        std::hint::black_box(sniff(&zeros));
    });
    report("sniff", "miss", zeros.len() as u64, &sniff_miss, false);

    let parse_hit = measure(ITERS, || {
        std::hint::black_box(parse(&file).expect("parse"));
    });
    report("bmff_parse", "hit", nbytes, &parse_hit, true);

    let parse_miss = measure(ITERS, || {
        std::hint::black_box(parse(&miss).err());
    });
    report("bmff_parse", "miss", miss.len() as u64, &parse_miss, false);

    let remux = measure(ITERS, || {
        let mut dedup = Dedup::new();
        std::hint::black_box(
            export(
                &file,
                Job::Remux {
                    faststart: true,
                    audio_only: false,
                    video_only: false,
                },
                &mut dedup,
            )
            .expect("remux"),
        );
    });
    report("remux", "hit", nbytes, &remux, true);

    let mut seeded = Dedup::new();
    export(
        &file,
        Job::Remux {
            faststart: true,
            audio_only: false,
            video_only: false,
        },
        &mut seeded,
    )
    .expect("seed");
    let dedup_hit = measure(ITERS, || {
        let out = export(
            &file,
            Job::Remux {
                faststart: true,
                audio_only: false,
                video_only: false,
            },
            &mut seeded,
        )
        .expect("dedup");
        assert!(matches!(out, Output::Duplicate { .. }));
    });
    report("dedup", "hit", nbytes, &dedup_hit, true);
}
