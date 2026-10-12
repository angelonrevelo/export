//! Release timings. `cargo bench` uses the bench profile (lto=thin, codegen-units=1).
//!
//! MB/s is uncompressed pixel bytes per second at the p50 latency, not the
//! compressed file size. `file_mb_s` is the PNG or JPEG byte length per second.
//! Allocations are heap allocations inside one steady-state call, divided by
//! megapixels (width*height/1e6). Warmup calls are not included. Percentiles
//! are nearest-rank on the high side: index ceil(p * n) - 1 of the sorted samples.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_image::{
    decode_png, decode_webp, encode_jpeg, encode_png, encode_webp_lossless, encode_webp_lossy,
    resize, restore_lossless_all, shrink_png, thumbnail_jpeg, Filter, Raster, ShrinkCache,
};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct CountAlloc;

unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout)
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc_zeroed(layout)
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.realloc(ptr, layout, new_size)
    }
}

#[global_allocator]
static GLOBAL: CountAlloc = CountAlloc;

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let rank = ((sorted.len() as f64) * p).ceil() as usize;
    let i = rank.clamp(1, sorted.len()) - 1;
    sorted[i]
}

fn time_n(n: usize, mut f: impl FnMut()) -> (u128, u128) {
    let mut samples = Vec::with_capacity(n);
    for _ in 0..n {
        let t = Instant::now();
        f();
        samples.push(t.elapsed().as_nanos());
    }
    samples.sort_unstable();
    (percentile(&samples, 0.50), percentile(&samples, 0.99))
}

fn mb_s(bytes: f64, nanos: u128) -> f64 {
    if nanos == 0 {
        return 0.0;
    }
    (bytes / 1_000_000.0) / (nanos as f64 / 1_000_000_000.0)
}

fn gradient(w: u32, h: u32) -> Raster {
    let mut pixels = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            pixels.push((x * 255 / w.max(1)) as u8);
            pixels.push((y * 255 / h.max(1)) as u8);
            pixels.push(((x + y) % 256) as u8);
        }
    }
    Raster::new(w, h, 3, pixels).unwrap()
}

fn main() {
    let w = 640u32;
    let h = 480u32;
    let pixels = w as f64 * h as f64;
    let image = gradient(w, h);
    let png = encode_png(&image).unwrap();
    let file_bytes = png.len() as f64;
    let pixel_bytes = pixels * 3.0;

    for _ in 0..6 {
        let _ = decode_png(&png).unwrap();
    }
    let (p50, p99) = time_n(80, || {
        let _ = decode_png(&png).unwrap();
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let _ = decode_png(&png).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let allocs_per_mp = allocs as f64 / (pixels / 1_000_000.0);
    println!(
        "png_decode pixels={w}x{h} file_bytes={} samples=80 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={allocs_per_mp:.2}",
        png.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        mb_s(pixel_bytes, p99),
        mb_s(file_bytes, p50),
    );

    let dw = 220u32;
    let dh = 165u32;
    for _ in 0..4 {
        let _ = resize(&image, dw, dh, Filter::Area).unwrap();
    }
    let (p50, p99) = time_n(60, || {
        let _ = resize(&image, dw, dh, Filter::Area).unwrap();
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let _ = resize(&image, dw, dh, Filter::Area).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let out_pixels = dw as f64 * dh as f64;
    println!(
        "resize_area {w}x{h}->{dw}x{dh} samples=60 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        allocs as f64 / (out_pixels / 1_000_000.0),
    );

    let (p50, p99) = time_n(40, || {
        let _ = resize(&image, dw, dh, Filter::Bilinear).unwrap();
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let _ = resize(&image, dw, dh, Filter::Bilinear).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    println!(
        "resize_bilinear {w}x{h}->{dw}x{dh} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        allocs as f64 / (out_pixels / 1_000_000.0),
    );

    for _ in 0..3 {
        let _ = encode_jpeg(&image, 60).unwrap();
    }
    let (p50, p99) = time_n(20, || {
        let _ = encode_jpeg(&image, 60).unwrap();
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let jpeg = encode_jpeg(&image, 60).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    println!(
        "jpeg_encode_q60 {w}x{h} out_bytes={} samples=20 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        jpeg.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        mb_s(jpeg.len() as f64, p50),
        allocs as f64 / (pixels / 1_000_000.0),
    );
    println!(
        "jpeg_encode_q60_note single quality 60 is an encoder microbenchmark, not the shrink setting"
    );

    let small = gradient(96, 72);
    let small_png = encode_png(&small).unwrap();
    let target = 8_000u64;
    let mut cache = ShrinkCache::new();
    let once = shrink_png(&small_png, target, &mut cache).unwrap();
    println!(
        "shrink_once {}x{} target={target} quality={} scales={} probes={} out_bytes={} copied={}",
        small.width,
        small.height,
        once.quality.unwrap_or(0),
        once.scales,
        once.probes.len(),
        once.bytes.len(),
        once.copied,
    );
    for _ in 0..2 {
        let mut c = ShrinkCache::new();
        let _ = shrink_png(&small_png, target, &mut c).unwrap();
    }
    let (p50, p99) = time_n(12, || {
        let mut c = ShrinkCache::new();
        let _ = shrink_png(&small_png, target, &mut c).unwrap();
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let mut c = ShrinkCache::new();
    let _ = shrink_png(&small_png, target, &mut c).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let small_pixels = 96.0 * 72.0;
    println!(
        "shrink_to_bytes 96x72 target={target} samples=12 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(small_pixels * 3.0, p50),
        allocs as f64 / (small_pixels / 1_000_000.0),
    );

    let bad = b"not a png";
    let report = restore_lossless_all(&[&png, &small_png, bad]);
    println!(
        "lossless_restore seen={} restored={} failed={}",
        report.seen, report.restored, report.failed
    );

    // Appended measurement. The rows above are unchanged.
    for _ in 0..2 {
        let _ = thumbnail_jpeg(&png).unwrap();
    }
    let (p50, p99) = time_n(20, || {
        let _ = thumbnail_jpeg(&png).unwrap();
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let thumb = thumbnail_jpeg(&png).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    println!(
        "thumbnail_q72 {w}x{h}->{}x{} quality=72 read_limit={} out_bytes={} samples=20 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        thumb.width,
        thumb.height,
        export_image::THUMB_READ_LIMIT,
        thumb.jpeg.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        allocs as f64 / (pixels / 1_000_000.0),
    );

    // Appended WebP measurement. The rows above are unchanged.
    let lossless = encode_webp_lossless(&image).unwrap();
    for _ in 0..2 {
        let _ = decode_webp(&lossless).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        let _ = decode_webp(&lossless).unwrap();
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let decoded = decode_webp(&lossless).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(decoded.pixels, image.pixels);
    println!(
        "webp_vp8l_decode {w}x{h} file_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        lossless.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        mb_s(pixel_bytes, p99),
        mb_s(lossless.len() as f64, p50),
        allocs as f64 / (pixels / 1_000_000.0),
    );

    let lossy = encode_webp_lossy(&image).unwrap();
    for _ in 0..2 {
        let _ = decode_webp(&lossy).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        let _ = decode_webp(&lossy).unwrap();
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let _ = decode_webp(&lossy).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    println!(
        "webp_vp8_decode {w}x{h} qindex=8 file_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        lossy.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        mb_s(pixel_bytes, p99),
        mb_s(lossy.len() as f64, p50),
        allocs as f64 / (pixels / 1_000_000.0),
    );

    // Bad signature. This is the miss: the call returns Invalid and does not decode.
    let miss: &[u8] = b"RIFF\x04\x00\x00\x00JPEG";
    for _ in 0..4 {
        assert!(decode_webp(miss).is_err());
    }
    let (p50, p99) = time_n(80, || {
        assert!(decode_webp(miss).is_err());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let miss_err = decode_webp(miss).unwrap_err();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    println!(
        "webp_decode_miss bad_signature samples=80 p50_us={:.1} p99_us={:.1} error={miss_err} allocs={allocs}",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
    );
}
