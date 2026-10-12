//! Release timings. `cargo bench` uses the bench profile (lto=thin, codegen-units=1).
//!
//! MB/s is uncompressed pixel bytes per second at the p50 latency, not the
//! compressed file size. `file_mb_s` is the PNG or JPEG byte length per second.
//! Allocations are heap allocations inside one steady-state call, divided by
//! megapixels (width*height/1e6). Warmup calls are not included. Percentiles
//! are nearest-rank on the high side: index ceil(p * n) - 1 of the sorted samples.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_image::{
    decode_png, decode_webp, decode_webp_animation, encode_jpeg, encode_png, encode_webp_animation,
    encode_webp_lossless, encode_webp_lossy,
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

fn overlay_matches(canvas: &Raster, tile: &Raster, x: u32, y: u32, got: &Raster) -> bool {
    if got.width != canvas.width
        || got.height != canvas.height
        || got.channels != canvas.channels
        || got.x != 0
        || got.y != 0
    {
        return false;
    }
    let ch = canvas.channels as usize;
    let stride = canvas.width as usize * ch;
    let tile_stride = tile.width as usize * ch;
    for row in 0..canvas.height as usize {
        for col in 0..canvas.width as usize {
            let covered = (col as u32) >= x
                && (col as u32) < x + tile.width
                && (row as u32) >= y
                && (row as u32) < y + tile.height;
            let at = row * stride + col * ch;
            if covered {
                let sr = (row - y as usize) * tile_stride + (col - x as usize) * ch;
                if got.pixels[at..at + ch] != tile.pixels[sr..sr + ch] {
                    return false;
                }
            } else if got.pixels[at..at + ch] != canvas.pixels[at..at + ch] {
                return false;
            }
        }
    }
    true
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
    let miss: &[u8] = black_box(b"RIFF\x04\x00\x00\x00JPEG");
    for _ in 0..4 {
        assert!(black_box(decode_webp(black_box(miss))).is_err());
    }
    let (p50, p99) = time_n(80, || {
        assert!(black_box(decode_webp(black_box(miss))).is_err());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let miss_err = decode_webp(miss).unwrap_err();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    println!(
        "webp_decode_miss bad_signature samples=80 p50_ns={} p99_ns={} error={miss_err} allocs={allocs}",
        p50, p99,
    );

    // Appended encode measurement. Decode rows above are unchanged.
    for _ in 0..2 {
        let _ = encode_webp_lossless(&image).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_lossless(black_box(&image)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let encoded = encode_webp_lossless(&image).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(encoded.len(), lossless.len());
    println!(
        "webp_vp8l_encode {w}x{h} out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        encoded.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        mb_s(pixel_bytes, p99),
        mb_s(encoded.len() as f64, p50),
        allocs as f64 / (pixels / 1_000_000.0),
    );

    for _ in 0..2 {
        let _ = encode_webp_lossy(&image).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_lossy(black_box(&image)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let encoded = encode_webp_lossy(&image).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    assert_eq!(encoded.len(), lossy.len());
    println!(
        "webp_vp8_encode {w}x{h} qindex=8 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        encoded.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(pixel_bytes, p50),
        mb_s(pixel_bytes, p99),
        mb_s(encoded.len() as f64, p50),
        allocs as f64 / (pixels / 1_000_000.0),
    );

    // 8×8 is not a multiple of 16, so the lossy encoder rejects it before the DCT.
    let odd = Raster::new(8, 8, 3, vec![0u8; 8 * 8 * 3]).unwrap();
    for _ in 0..4 {
        assert!(black_box(encode_webp_lossy(black_box(&odd))).is_err());
    }
    let (p50, p99) = time_n(80, || {
        assert!(black_box(encode_webp_lossy(black_box(&odd))).is_err());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let miss_err = encode_webp_lossy(&odd).unwrap_err();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    println!(
        "webp_encode_miss odd_size=8x8 samples=80 p50_ns={} p99_ns={} error={miss_err} allocs={allocs}",
        p50, p99,
    );

    // Appended two-frame animation. The 640×480 encode rows above are unchanged.
    let mut red = vec![0u8; 16 * 16 * 3];
    for px in red.chunks_exact_mut(3) {
        px.copy_from_slice(&[220, 40, 60]);
    }
    let mut green = vec![0u8; 16 * 16 * 3];
    for px in green.chunks_exact_mut(3) {
        px.copy_from_slice(&[20, 180, 90]);
    }
    let frame_a = Raster::new(16, 16, 3, red).unwrap();
    let frame_b = Raster::new(16, 16, 3, green).unwrap();
    let frames = [frame_a, frame_b];
    for _ in 0..2 {
        let _ = encode_webp_animation(&frames).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&frames)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim = encode_webp_animation(&frames).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim_pixels = 16.0 * 16.0 * 2.0;
    println!(
        "webp_anim_encode 16x16 frames=2 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim_pixels * 3.0, p50),
        mb_s(anim_pixels * 3.0, p99),
        mb_s(anim.len() as f64, p50),
        allocs as f64 / (anim_pixels / 1_000_000.0),
    );
    let decoded = decode_webp_animation(&anim).unwrap();
    let still_a = decode_webp(&encode_webp_lossy(&frames[0]).unwrap()).unwrap();
    let still_b = decode_webp(&encode_webp_lossy(&frames[1]).unwrap()).unwrap();
    let both = decoded.len() == 2
        && decoded[0].pixels == still_a.pixels
        && decoded[1].pixels == still_b.pixels;
    println!(
        "webp_anim_decode frames={} both_frames={both} single_raster_errors={}",
        decoded.len(),
        decode_webp(&anim).is_err(),
    );

    // Appended four-frame animation. The two-frame row above is unchanged.
    let colors = [[220u8, 40, 60], [20, 180, 90], [10, 10, 200], [240, 240, 240]];
    let mut quad = Vec::with_capacity(4);
    for rgb in colors {
        let mut pixels = vec![0u8; 16 * 16 * 3];
        for px in pixels.chunks_exact_mut(3) {
            px.copy_from_slice(&rgb);
        }
        quad.push(Raster::new(16, 16, 3, pixels).unwrap());
    }
    for _ in 0..2 {
        let _ = encode_webp_animation(&quad).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&quad)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim4 = encode_webp_animation(&quad).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim4_pixels = 16.0 * 16.0 * 4.0;
    println!(
        "webp_anim4_encode 16x16 frames=4 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim4.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim4_pixels * 3.0, p50),
        mb_s(anim4_pixels * 3.0, p99),
        mb_s(anim4.len() as f64, p50),
        allocs as f64 / (anim4_pixels / 1_000_000.0),
    );
    let decoded4 = decode_webp_animation(&anim4).unwrap();
    let mut all_four = decoded4.len() == 4;
    for (got, src) in decoded4.iter().zip(&quad) {
        let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
        all_four &= got.pixels == still.pixels;
    }
    println!(
        "webp_anim4_decode frames={} all_four={all_four} single_raster_errors={}",
        decoded4.len(),
        decode_webp(&anim4).is_err(),
    );

    // Appended 64×64 two-frame animation. The 16×16 rows above are unchanged.
    let mut wide = Vec::with_capacity(2);
    for rgb in [[220u8, 40, 60], [20, 180, 90]] {
        let mut pixels = vec![0u8; 64 * 64 * 3];
        for px in pixels.chunks_exact_mut(3) {
            px.copy_from_slice(&rgb);
        }
        wide.push(Raster::new(64, 64, 3, pixels).unwrap());
    }
    for _ in 0..2 {
        let _ = encode_webp_animation(&wide).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&wide)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim64 = encode_webp_animation(&wide).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim64_pixels = 64.0 * 64.0 * 2.0;
    println!(
        "webp_anim64_encode 64x64 frames=2 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim64.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim64_pixels * 3.0, p50),
        mb_s(anim64_pixels * 3.0, p99),
        mb_s(anim64.len() as f64, p50),
        allocs as f64 / (anim64_pixels / 1_000_000.0),
    );
    let decoded64 = decode_webp_animation(&anim64).unwrap();
    let still64_a = decode_webp(&encode_webp_lossy(&wide[0]).unwrap()).unwrap();
    let still64_b = decode_webp(&encode_webp_lossy(&wide[1]).unwrap()).unwrap();
    let both64 = decoded64.len() == 2
        && decoded64[0].pixels == still64_a.pixels
        && decoded64[1].pixels == still64_b.pixels;
    println!(
        "webp_anim64_decode frames={} both_frames={both64} single_raster_errors={}",
        decoded64.len(),
        decode_webp(&anim64).is_err(),
    );

    // Appended 256×256 two-frame animation. The rows above are unchanged.
    let mut large = Vec::with_capacity(2);
    for rgb in [[220u8, 40, 60], [20, 180, 90]] {
        let mut pixels = vec![0u8; 256 * 256 * 3];
        for px in pixels.chunks_exact_mut(3) {
            px.copy_from_slice(&rgb);
        }
        large.push(Raster::new(256, 256, 3, pixels).unwrap());
    }
    for _ in 0..2 {
        let _ = encode_webp_animation(&large).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&large)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim256 = encode_webp_animation(&large).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim256_pixels = 256.0 * 256.0 * 2.0;
    println!(
        "webp_anim256_encode 256x256 frames=2 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim256.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim256_pixels * 3.0, p50),
        mb_s(anim256_pixels * 3.0, p99),
        mb_s(anim256.len() as f64, p50),
        allocs as f64 / (anim256_pixels / 1_000_000.0),
    );
    let decoded256 = decode_webp_animation(&anim256).unwrap();
    let still256_a = decode_webp(&encode_webp_lossy(&large[0]).unwrap()).unwrap();
    let still256_b = decode_webp(&encode_webp_lossy(&large[1]).unwrap()).unwrap();
    let both256 = decoded256.len() == 2
        && decoded256[0].pixels == still256_a.pixels
        && decoded256[1].pixels == still256_b.pixels;
    println!(
        "webp_anim256_decode frames={} both_frames={both256} single_raster_errors={}",
        decoded256.len(),
        decode_webp(&anim256).is_err(),
    );

    // Appended 640×480 two-frame animation. The rows above are unchanged.
    let mut flipped = image.pixels.clone();
    for px in flipped.chunks_exact_mut(3) {
        px[0] = px[0].wrapping_add(80);
    }
    let pair = [
        image.clone(),
        Raster::new(w, h, 3, flipped).unwrap(),
    ];
    for _ in 0..2 {
        let _ = encode_webp_animation(&pair).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&pair)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim_hd = encode_webp_animation(&pair).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim_hd_pixels = (w as f64) * (h as f64) * 2.0;
    println!(
        "webp_anim640_encode {w}x{h} frames=2 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim_hd.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim_hd_pixels * 3.0, p50),
        mb_s(anim_hd_pixels * 3.0, p99),
        mb_s(anim_hd.len() as f64, p50),
        allocs as f64 / (anim_hd_pixels / 1_000_000.0),
    );
    let decoded_hd = decode_webp_animation(&anim_hd).unwrap();
    let still_hd_a = decode_webp(&encode_webp_lossy(&pair[0]).unwrap()).unwrap();
    let still_hd_b = decode_webp(&encode_webp_lossy(&pair[1]).unwrap()).unwrap();
    let both_hd = decoded_hd.len() == 2
        && decoded_hd[0].pixels == still_hd_a.pixels
        && decoded_hd[1].pixels == still_hd_b.pixels;
    println!(
        "webp_anim640_decode frames={} both_frames={both_hd} still_match={both_hd} single_raster_errors={}",
        decoded_hd.len(),
        decode_webp(&anim_hd).is_err(),
    );

    // Appended four-frame 640×480. The two-frame row above is unchanged.
    let mut quad_hd = Vec::with_capacity(4);
    for add in [0u8, 80, 160, 40] {
        let mut pixels = image.pixels.clone();
        for px in pixels.chunks_exact_mut(3) {
            px[0] = px[0].wrapping_add(add);
        }
        quad_hd.push(Raster::new(w, h, 3, pixels).unwrap());
    }
    for _ in 0..2 {
        let _ = encode_webp_animation(&quad_hd).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&quad_hd)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim4_hd = encode_webp_animation(&quad_hd).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim4_hd_pixels = (w as f64) * (h as f64) * 4.0;
    println!(
        "webp_anim640x4_encode {w}x{h} frames=4 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim4_hd.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim4_hd_pixels * 3.0, p50),
        mb_s(anim4_hd_pixels * 3.0, p99),
        mb_s(anim4_hd.len() as f64, p50),
        allocs as f64 / (anim4_hd_pixels / 1_000_000.0),
    );
    let decoded4_hd = decode_webp_animation(&anim4_hd).unwrap();
    let mut all_four_hd = decoded4_hd.len() == 4;
    for (got, src) in decoded4_hd.iter().zip(&quad_hd) {
        let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
        all_four_hd &= got.pixels == still.pixels;
    }
    println!(
        "webp_anim640x4_decode frames={} all_four={all_four_hd} still_match={all_four_hd} single_raster_errors={}",
        decoded4_hd.len(),
        decode_webp(&anim4_hd).is_err(),
    );

    // Appended 1280×720 two-frame animation. The rows above are unchanged.
    let wide_w = 1280u32;
    let wide_h = 720u32;
    let hd720 = gradient(wide_w, wide_h);
    let mut shifted = hd720.pixels.clone();
    for px in shifted.chunks_exact_mut(3) {
        px[0] = px[0].wrapping_add(80);
    }
    let pair720 = [
        hd720,
        Raster::new(wide_w, wide_h, 3, shifted).unwrap(),
    ];
    for _ in 0..2 {
        let _ = encode_webp_animation(&pair720).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&pair720)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim720 = encode_webp_animation(&pair720).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim720_pixels = (wide_w as f64) * (wide_h as f64) * 2.0;
    println!(
        "webp_anim1280_encode {wide_w}x{wide_h} frames=2 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim720.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim720_pixels * 3.0, p50),
        mb_s(anim720_pixels * 3.0, p99),
        mb_s(anim720.len() as f64, p50),
        allocs as f64 / (anim720_pixels / 1_000_000.0),
    );
    let decoded720 = decode_webp_animation(&anim720).unwrap();
    let still720_a = decode_webp(&encode_webp_lossy(&pair720[0]).unwrap()).unwrap();
    let still720_b = decode_webp(&encode_webp_lossy(&pair720[1]).unwrap()).unwrap();
    let both720 = decoded720.len() == 2
        && decoded720[0].pixels == still720_a.pixels
        && decoded720[1].pixels == still720_b.pixels;
    println!(
        "webp_anim1280_decode frames={} both_frames={both720} still_match={both720} single_raster_errors={}",
        decoded720.len(),
        decode_webp(&anim720).is_err(),
    );

    // Appended four-frame 1280×720. The two-frame row above is unchanged.
    let mut quad720 = Vec::with_capacity(4);
    for add in [0u8, 80, 160, 40] {
        let mut pixels = pair720[0].pixels.clone();
        for px in pixels.chunks_exact_mut(3) {
            px[0] = px[0].wrapping_add(add);
        }
        quad720.push(Raster::new(wide_w, wide_h, 3, pixels).unwrap());
    }
    for _ in 0..2 {
        let _ = encode_webp_animation(&quad720).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&quad720)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim4_720 = encode_webp_animation(&quad720).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim4_720_pixels = (wide_w as f64) * (wide_h as f64) * 4.0;
    println!(
        "webp_anim1280x4_encode {wide_w}x{wide_h} frames=4 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim4_720.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim4_720_pixels * 3.0, p50),
        mb_s(anim4_720_pixels * 3.0, p99),
        mb_s(anim4_720.len() as f64, p50),
        allocs as f64 / (anim4_720_pixels / 1_000_000.0),
    );
    let decoded4_720 = decode_webp_animation(&anim4_720).unwrap();
    let mut all_four_720 = decoded4_720.len() == 4;
    for (got, src) in decoded4_720.iter().zip(&quad720) {
        let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
        all_four_720 &= got.pixels == still.pixels;
    }
    println!(
        "webp_anim1280x4_decode frames={} all_four={all_four_720} still_match={all_four_720} single_raster_errors={}",
        decoded4_720.len(),
        decode_webp(&anim4_720).is_err(),
    );

    // Appended 64×64 canvas with a 16×16 partial frame at the origin.
    // The full-frame 64×64 row above is unchanged.
    let mut canvas64 = vec![0u8; 64 * 64 * 3];
    for px in canvas64.chunks_exact_mut(3) {
        px.copy_from_slice(&[220, 40, 60]);
    }
    let mut tile16 = vec![0u8; 16 * 16 * 3];
    for px in tile16.chunks_exact_mut(3) {
        px.copy_from_slice(&[20, 180, 90]);
    }
    let partial = [
        Raster::new(64, 64, 3, canvas64).unwrap(),
        Raster::new(16, 16, 3, tile16).unwrap(),
    ];
    for _ in 0..2 {
        let _ = encode_webp_animation(&partial).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&partial)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim_part = encode_webp_animation(&partial).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim_part_pixels = 64.0 * 64.0 + 16.0 * 16.0;
    println!(
        "webp_anim64_partial_encode canvas=64x64 frames=2 partial=16x16 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim_part.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim_part_pixels * 3.0, p50),
        mb_s(anim_part_pixels * 3.0, p99),
        mb_s(anim_part.len() as f64, p50),
        allocs as f64 / (anim_part_pixels / 1_000_000.0),
    );
    let decoded_part = decode_webp_animation(&anim_part).unwrap();
    let still_canvas = decode_webp(&encode_webp_lossy(&partial[0]).unwrap()).unwrap();
    let still_tile = decode_webp(&encode_webp_lossy(&partial[1]).unwrap()).unwrap();
    let both_part = decoded_part.len() == 2
        && decoded_part[0].pixels == still_canvas.pixels
        && overlay_matches(&still_canvas, &still_tile, 0, 0, &decoded_part[1]);
    println!(
        "webp_anim64_partial_decode frames={} both_frames={both_part} still_match={both_part} single_raster_errors={}",
        decoded_part.len(),
        decode_webp(&anim_part).is_err(),
    );

    // Appended 16×16 rectangle at a non-zero origin. The rows above are unchanged.
    let placed = [
        partial[0].clone(),
        partial[1].clone().with_origin(16, 32),
    ];
    for _ in 0..2 {
        let _ = encode_webp_animation(&placed).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(encode_webp_animation(black_box(&placed)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let anim_placed = encode_webp_animation(&placed).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let anim_placed_pixels = 64.0 * 64.0 + 16.0 * 16.0;
    println!(
        "webp_anim64_placed_encode canvas=64x64 frames=2 partial=16x16 origin=16,32 out_bytes={} samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2}",
        anim_placed.len(),
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(anim_placed_pixels * 3.0, p50),
        mb_s(anim_placed_pixels * 3.0, p99),
        mb_s(anim_placed.len() as f64, p50),
        allocs as f64 / (anim_placed_pixels / 1_000_000.0),
    );
    let decoded_placed = decode_webp_animation(&anim_placed).unwrap();
    let still_placed_canvas = decode_webp(&encode_webp_lossy(&placed[0]).unwrap()).unwrap();
    let still_placed = decode_webp(&encode_webp_lossy(&placed[1]).unwrap()).unwrap();
    let placed_ok = decoded_placed.len() == 2
        && decoded_placed[0].pixels == still_placed_canvas.pixels
        && overlay_matches(&still_placed_canvas, &still_placed, 16, 32, &decoded_placed[1]);
    println!(
        "webp_anim64_placed_decode frames={} at_origin={placed_ok} still_match={placed_ok} single_raster_errors={}",
        decoded_placed.len(),
        decode_webp(&anim_placed).is_err(),
    );

    // Decode timing for the composited canvas. The encode rows above are unchanged.
    for _ in 0..2 {
        let _ = decode_webp_animation(&anim_placed).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(decode_webp_animation(black_box(&anim_placed)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let composited = decode_webp_animation(&anim_placed).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let composited_pixels = 64.0 * 64.0 * 2.0;
    let overlay = composited.len() == 2
        && composited[0].pixels == still_placed_canvas.pixels
        && overlay_matches(&still_placed_canvas, &still_placed, 16, 32, &composited[1]);
    println!(
        "webp_anim64_paint_decode canvas=64x64 frames=2 origin=16,32 tile=16x16 samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2} overlay={overlay}",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(composited_pixels * 3.0, p50),
        mb_s(composited_pixels * 3.0, p99),
        mb_s(anim_placed.len() as f64, p50),
        allocs as f64 / (composited_pixels / 1_000_000.0),
    );

    // decode_webp of this animation file only. The paint row above is unchanged.
    for _ in 0..2 {
        let _ = decode_webp(&anim_placed).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(decode_webp(black_box(&anim_placed)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let one = decode_webp(&anim_placed).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let one_pixels = 64.0 * 64.0;
    let overlay = overlay_matches(&still_placed_canvas, &still_placed, 16, 32, &one);
    let still_file = encode_webp_lossy(&placed[0]).unwrap();
    let still_pixels = decode_webp(&still_file).unwrap().pixels;
    let still_ok = still_pixels == still_placed_canvas.pixels
        && decode_webp(&anim_part).is_err()
        && one.pixels == composited[1].pixels;
    println!(
        "webp_anim64_still_decode canvas=64x64 origin=16,32 tile=16x16 samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2} overlay={overlay} still_unchanged={still_ok}",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(one_pixels * 3.0, p50),
        mb_s(one_pixels * 3.0, p99),
        mb_s(anim_placed.len() as f64, p50),
        allocs as f64 / (one_pixels / 1_000_000.0),
    );

    // decode_webp of the origin-zero partial only. The (16, 32) row above is unchanged.
    for _ in 0..2 {
        let _ = decode_webp(&anim_part).unwrap();
    }
    let (p50, p99) = time_n(40, || {
        black_box(decode_webp(black_box(&anim_part)).unwrap());
    });
    ALLOCS.store(0, Ordering::Relaxed);
    let origin = decode_webp(&anim_part).unwrap();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    let origin_pixels = 64.0 * 64.0;
    let canvas_match = origin.width == 64
        && origin.height == 64
        && origin.x == 0
        && origin.y == 0
        && origin.pixels == decoded_part[1].pixels;
    let placed_same = decode_webp(&anim_placed).unwrap().pixels == one.pixels;
    let still_same = decode_webp(&encode_webp_lossy(&partial[0]).unwrap())
        .unwrap()
        .pixels
        == still_canvas.pixels;
    let full_errors = decode_webp(&anim64).is_err();
    println!(
        "webp_anim64_origin0_decode canvas=64x64 origin=0,0 tile=16x16 samples=40 p50_us={:.1} p99_us={:.1} pixel_mb_s_p50={:.2} pixel_mb_s_p99={:.2} file_mb_s_p50={:.2} allocs={allocs} allocs_per_mp={:.2} canvas_match={canvas_match} placed_same={placed_same} still_same={still_same} full_errors={full_errors}",
        p50 as f64 / 1_000.0,
        p99 as f64 / 1_000.0,
        mb_s(origin_pixels * 3.0, p50),
        mb_s(origin_pixels * 3.0, p99),
        mb_s(anim_part.len() as f64, p50),
        allocs as f64 / (origin_pixels / 1_000_000.0),
    );
}
