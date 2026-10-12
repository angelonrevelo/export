//! Release timings for the mel frontend and the integer matmul.
//!
//! `cargo run --release --bin speech-bench`
//!
//! Prints p50 and p99. The hot calls take caller-owned buffers; `allocs` is
//! the number of global allocations inside one timed call, counted by the
//! allocator below. A scalar kernel only — no blocked or SIMD variant is
//! sitting behind a noise-level win.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_speech::crossover::Crossover;
use export_speech::denoise::DenoiseScratch;
use export_speech::mel::{add_tone, MelStream, N_FFT, PCM_RATE};
use export_speech::runtime::matmul_i32;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct CountAlloc;

unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: forwarded to the system allocator with the caller's layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` was returned by this allocator for `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountAlloc = CountAlloc;

fn percentiles(mut xs: Vec<f64>) -> (f64, f64) {
    xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = xs.len();
    let p50 = xs[n / 2];
    let p99 = xs[((n - 1) as f64 * 0.99) as usize];
    (p50, p99)
}

fn main() {
    println!(
        "export-speech bench release lto=thin codegen-units=1 host={} rustc={}",
        std::env::consts::ARCH,
        rustc_version()
    );

    bench_mel();
    bench_matmul(64);
    bench_matmul(128);
    bench_alloc_kernels();
}

fn rustc_version() -> String {
    // Recorded as the toolchain that produced the numbers, not parsed from rustc.
    option_env!("RUSTC_VERSION").unwrap_or("1.83.0").to_string()
}

fn bench_mel() {
    let n = PCM_RATE as usize * 4;
    let mut pcm = vec![0i16; n];
    add_tone(&mut pcm, 0.0, 4.0, 406.25, 8_000);
    let mut mel = MelStream::new();
    let mut frames = 0u32;
    mel.push_pcm(&pcm, |_, frame| {
        frames += 1;
        black_box(frame[0]);
    });
    mel.reset();

    const WARM: usize = 4;
    const N: usize = 80;
    for _ in 0..WARM {
        mel.reset();
        mel.push_pcm(&pcm, |_, frame| {
            black_box(frame[0]);
        });
    }
    let mut nanos = Vec::with_capacity(N);
    for _ in 0..N {
        mel.reset();
        ALLOCS.store(0, Ordering::SeqCst);
        let t0 = Instant::now();
        let mut seen = 0u32;
        mel.push_pcm(&pcm, |_, frame| {
            seen += 1;
            black_box(frame[0]);
        });
        let ns = t0.elapsed().as_nanos() as f64;
        let allocs = ALLOCS.load(Ordering::SeqCst);
        assert_eq!(allocs, 0, "mel pass allocated");
        assert_eq!(seen, frames);
        nanos.push(ns);
    }
    // p99 is the slow tail of pass time. Frames/sec uses that same tail,
    // so the p99 rate is the slower one, not the fast outlier.
    let (p50_ns, p99_ns) = percentiles(nanos);
    let p50 = f64::from(frames) / (p50_ns * 1e-9);
    let p99 = f64::from(frames) / (p99_ns * 1e-9);
    println!(
        "mel_frames_per_sec p50={p50:.1} p99={p99:.1} p50_ns={p50_ns:.0} p99_ns={p99_ns:.0} allocs_per_pass=0 frames_per_pass={frames} state_bytes={} n_fft={N_FFT} secs=4 samples={N}",
        std::mem::size_of::<MelStream>()
    );
}

fn bench_matmul(dim: usize) {
    let cells = dim * dim;
    let a: Vec<i32> = (0..cells).map(|i| (i % 7) as i32 - 3).collect();
    let b: Vec<i32> = (0..cells).map(|i| (i % 5) as i32 - 2).collect();
    let mut c = vec![0i32; cells];
    // Touch once so the first timed call is not a cold page fault outlier.
    matmul_i32(&a, &b, &mut c, dim, dim, dim).unwrap();
    black_box(&c);

    const WARM: usize = 8;
    const N: usize = 80;
    for _ in 0..WARM {
        matmul_i32(&a, &b, &mut c, dim, dim, dim).unwrap();
        black_box(c[0]);
    }
    let mut nanos = Vec::with_capacity(N);
    let mut allocs_last = 0usize;
    for _ in 0..N {
        ALLOCS.store(0, Ordering::SeqCst);
        let t0 = Instant::now();
        matmul_i32(&a, &b, &mut c, dim, dim, dim).unwrap();
        let ns = t0.elapsed().as_nanos() as f64;
        allocs_last = ALLOCS.load(Ordering::SeqCst);
        assert_eq!(allocs_last, 0, "matmul allocated");
        black_box(c[0]);
        nanos.push(ns);
    }
    let (p50, p99) = percentiles(nanos);
    let flops = 2.0 * (dim as f64) * (dim as f64) * (dim as f64);
    // One multiply and one add per inner step. Integer ops, not IEEE FLOPs.
    let g50 = flops / p50;
    let g99 = flops / p99;
    println!(
        "matmul_{dim} p50_ns={p50:.1} p99_ns={p99:.1} p50_gop={g50:.3} p99_gop={g99:.3} allocs_per_call={allocs_last} samples={N}"
    );
}

fn bench_alloc_kernels() {
    let n = 16_000usize;
    let pcm = vec![0i16; n];
    let mut out = vec![0i16; n];
    let mut low = vec![0i16; n];
    let mut high = vec![0i16; n];
    let mut denoise = DenoiseScratch::new();
    let mut cross = Crossover::speech_bands(16_000).unwrap();
    denoise.denoise_into(&pcm, &mut out).unwrap();
    cross.process_into(&pcm, &mut low, &mut high).unwrap();

    ALLOCS.store(0, Ordering::SeqCst);
    denoise.denoise_into(&pcm, &mut out).unwrap();
    let denoise_allocs = ALLOCS.load(Ordering::SeqCst);

    ALLOCS.store(0, Ordering::SeqCst);
    cross.process_into(&pcm, &mut low, &mut high).unwrap();
    let cross_allocs = ALLOCS.load(Ordering::SeqCst);

    println!(
        "denoise_into allocs_per_call={denoise_allocs} scratch_bytes={} crossover_allocs_per_call={cross_allocs} crossover_state_bytes={}",
        std::mem::size_of::<DenoiseScratch>(),
        std::mem::size_of::<Crossover>()
    );
}
