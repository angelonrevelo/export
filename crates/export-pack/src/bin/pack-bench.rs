//! Release-only throughput of store-zip write and read.
//! Debug builds print NOT JUDGED and do not report a speed.

use std::alloc::{GlobalAlloc, Layout, System};
use std::fs;
use std::hint::black_box;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use export_pack::ZipReader;
use export_pack::ZipWriter;

const PAYLOAD: usize = 32 * 1024 * 1024;

static ALLOCS: AtomicU64 = AtomicU64::new(0);
static ALLOC_BYTES: AtomicU64 = AtomicU64::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        ALLOC_BYTES.fetch_add(layout.size() as u64, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

struct Sink(u64);

impl Write for Sink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        black_box(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct Sample {
    write: Duration,
    finish: Duration,
    read: Duration,
    write_allocs: u64,
    write_alloc_bytes: u64,
    read_allocs: u64,
    read_alloc_bytes: u64,
    bytes: u64,
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("NOT JUDGED");
        return;
    }
    if let Some(value) = std::env::var_os("EXPORT_PACK_BENCH") {
        if value == "font-roundtrip" {
            println!("profile=release");
            bench_font_roundtrip();
            return;
        }
        if value == "zip-64mib" {
            println!("profile=release");
            let dir = PathBuf::from(env_dir());
            fs::create_dir_all(&dir).expect("bench dir");
            bench_zip_64mib(&dir);
            return;
        }
        if value == "zip-128mib" {
            println!("profile=release");
            let dir = PathBuf::from(env_dir());
            fs::create_dir_all(&dir).expect("bench dir");
            bench_zip_128mib(&dir);
            return;
        }
        if value == "zip-256mib" {
            println!("profile=release");
            let dir = PathBuf::from(env_dir());
            fs::create_dir_all(&dir).expect("bench dir");
            bench_zip_256mib(&dir);
            return;
        }
        if value == "zip-512mib" {
            println!("profile=release");
            let dir = PathBuf::from(env_dir());
            fs::create_dir_all(&dir).expect("bench dir");
            bench_zip_512mib(&dir);
            return;
        }
    }
    let data = payload(PAYLOAD);
    let dir = PathBuf::from(env_dir());
    fs::create_dir_all(&dir).expect("bench dir");
    // Warm the file path once. The published numbers are the second pass.
    run(&data, &dir);
    let sample = run(&data, &dir);
    let mib = sample.bytes as f64 / 1024.0 / 1024.0;
    println!("profile=release");
    println!("payload_bytes={}", sample.bytes);
    println!("entries=1");
    println!("method=store");
    println!("chunk_bytes={}", export_pack::CHUNK_BYTES);
    println!("zip_write_mib_s={:.1}", mib_s(sample.bytes, sample.write));
    println!("zip_write_mb_s={:.1}", mb_s(sample.bytes, sample.write));
    println!("zip_finish_ms={:.1}", ms(sample.finish));
    println!("zip_read_mib_s={:.1}", mib_s(sample.bytes, sample.read));
    println!("zip_read_mb_s={:.1}", mb_s(sample.bytes, sample.read));
    println!(
        "zip_write_allocs_per_mib={:.3}",
        sample.write_allocs as f64 / mib
    );
    println!(
        "zip_write_alloc_bytes_per_mib={:.1}",
        sample.write_alloc_bytes as f64 / mib
    );
    println!(
        "zip_read_allocs_per_mib={:.3}",
        sample.read_allocs as f64 / mib
    );
    println!(
        "zip_read_alloc_bytes_per_mib={:.1}",
        sample.read_alloc_bytes as f64 / mib
    );
    println!("bench_dir={}", dir.display());
    println!("note=write is CRC plus local header plus payload, before fsync");
    println!("note=finish is central directory plus fsync plus rename");
    println!("note=read follows write in-process, so the page cache may hold the file");
    bench_zip_64mib(&dir);
    bench_zip_128mib(&dir);
    bench_zip_256mib(&dir);
    bench_zip_512mib(&dir);
    bench_inflate();
    bench_pdf(&dir);
    bench_pdf16(&dir);
    bench_pdf32(&dir);
    bench_font();
    bench_font_roundtrip();
}

fn bench_zip_64mib(dir: &std::path::Path) {
    const BYTES: usize = 64 * 1024 * 1024;
    const PASSES_64: usize = 100;
    let data = payload(BYTES);
    verify_store_roundtrip(&data, dir, "bench64-check.zip");
    let mut writes = Vec::with_capacity(PASSES_64);
    let mut reads = Vec::with_capacity(PASSES_64);
    for _ in 0..PASSES_64 {
        let sample = run(&data, dir);
        assert_eq!(sample.bytes, BYTES as u64);
        writes.push(ms(sample.write));
        reads.push(ms(sample.read));
    }
    println!("zip_64mib_bytes={}", data.len());
    println!("zip_64mib_entries=1");
    println!("zip_64mib_method=store");
    println!("zip_64mib_passes={PASSES_64}");
    println!("zip_64mib_write_ms_p50={:.1}", percentile(&writes, 50.0));
    println!("zip_64mib_write_ms_p99={:.1}", percentile(&writes, 99.0));
    println!("zip_64mib_read_ms_p50={:.1}", percentile(&reads, 50.0));
    println!("zip_64mib_read_ms_p99={:.1}", percentile(&reads, 99.0));
    println!("note=64 MiB store entry, the next size after the 32 MiB entry");
    println!("note=no zip64 and no deflate; the existing store writer and reader");
    println!("note=write is CRC plus local header plus payload, before fsync");
    println!("note=read follows write in-process, so the page cache may hold the file");
    println!("note=warmup compared every payload byte; each timed read checks length and CRC");
    println!(
        "note=percentile is nearest-rank on milliseconds sorted ascending, so p99 is the slow tail"
    );
}

fn bench_zip_128mib(dir: &std::path::Path) {
    const BYTES: usize = 128 * 1024 * 1024;
    const PASSES_128: usize = 100;
    let data = payload(BYTES);
    verify_store_roundtrip(&data, dir, "bench128-check.zip");
    let mut writes = Vec::with_capacity(PASSES_128);
    let mut reads = Vec::with_capacity(PASSES_128);
    for _ in 0..PASSES_128 {
        let sample = run(&data, dir);
        assert_eq!(sample.bytes, BYTES as u64);
        writes.push(ms(sample.write));
        reads.push(ms(sample.read));
    }
    println!("zip_128mib_bytes={}", data.len());
    println!("zip_128mib_entries=1");
    println!("zip_128mib_method=store");
    println!("zip_128mib_passes={PASSES_128}");
    println!("zip_128mib_write_ms_p50={:.1}", percentile(&writes, 50.0));
    println!("zip_128mib_write_ms_p99={:.1}", percentile(&writes, 99.0));
    println!("zip_128mib_read_ms_p50={:.1}", percentile(&reads, 50.0));
    println!("zip_128mib_read_ms_p99={:.1}", percentile(&reads, 99.0));
    println!("note=128 MiB store entry, the next size after the 64 MiB entry");
    println!("note=no zip64 and no deflate; the existing store writer and reader");
    println!("note=write is CRC plus local header plus payload, before fsync");
    println!("note=read follows write in-process, so the page cache may hold the file");
    println!("note=warmup compared every payload byte; each timed read checks length and CRC");
    println!(
        "note=percentile is nearest-rank on milliseconds sorted ascending, so p99 is the slow tail"
    );
}

fn bench_zip_256mib(dir: &std::path::Path) {
    const BYTES: usize = 256 * 1024 * 1024;
    const PASSES_256: usize = 100;
    let data = payload(BYTES);
    verify_store_roundtrip(&data, dir, "bench256-check.zip");
    let mut writes = Vec::with_capacity(PASSES_256);
    let mut reads = Vec::with_capacity(PASSES_256);
    for _ in 0..PASSES_256 {
        let sample = run(&data, dir);
        assert_eq!(sample.bytes, BYTES as u64);
        writes.push(ms(sample.write));
        reads.push(ms(sample.read));
    }
    println!("zip_256mib_bytes={}", data.len());
    println!("zip_256mib_entries=1");
    println!("zip_256mib_method=store");
    println!("zip_256mib_passes={PASSES_256}");
    println!("zip_256mib_write_ms_p50={:.1}", percentile(&writes, 50.0));
    println!("zip_256mib_write_ms_p99={:.1}", percentile(&writes, 99.0));
    println!("zip_256mib_read_ms_p50={:.1}", percentile(&reads, 50.0));
    println!("zip_256mib_read_ms_p99={:.1}", percentile(&reads, 99.0));
    println!("note=256 MiB store entry, the next size after the 128 MiB entry");
    println!("note=no zip64 and no deflate; the existing store writer and reader");
    println!("note=write is CRC plus local header plus payload, before fsync");
    println!("note=read follows write in-process, so the page cache may hold the file");
    println!("note=warmup compared every payload byte; each timed read checks length and CRC");
    println!(
        "note=percentile is nearest-rank on milliseconds sorted ascending, so p99 is the slow tail"
    );
}

fn bench_zip_512mib(dir: &std::path::Path) {
    const BYTES: usize = 512 * 1024 * 1024;
    const PASSES_512: usize = 100;
    let data = payload(BYTES);
    verify_store_roundtrip(&data, dir, "bench512-check.zip");
    let mut writes = Vec::with_capacity(PASSES_512);
    let mut reads = Vec::with_capacity(PASSES_512);
    for _ in 0..PASSES_512 {
        let sample = run(&data, dir);
        assert_eq!(sample.bytes, BYTES as u64);
        writes.push(ms(sample.write));
        reads.push(ms(sample.read));
    }
    println!("zip_512mib_bytes={}", data.len());
    println!("zip_512mib_entries=1");
    println!("zip_512mib_method=store");
    println!("zip_512mib_passes={PASSES_512}");
    println!("zip_512mib_write_ms_p50={:.1}", percentile(&writes, 50.0));
    println!("zip_512mib_write_ms_p99={:.1}", percentile(&writes, 99.0));
    println!("zip_512mib_read_ms_p50={:.1}", percentile(&reads, 50.0));
    println!("zip_512mib_read_ms_p99={:.1}", percentile(&reads, 99.0));
    println!("note=512 MiB store entry, the next size after the 256 MiB entry");
    println!("note=no zip64 and no deflate; the existing store writer and reader");
    println!("note=write is CRC plus local header plus payload, before fsync");
    println!("note=read follows write in-process, so the page cache may hold the file");
    println!("note=warmup compared every payload byte; each timed read checks length and CRC");
    println!(
        "note=percentile is nearest-rank on milliseconds sorted ascending, so p99 is the slow tail"
    );
}

fn verify_store_roundtrip(data: &[u8], dir: &std::path::Path, name: &str) {
    let path = dir.join(name);
    let _ = fs::remove_file(&path);
    let mut zip = ZipWriter::create(&path).expect("create");
    zip.write_bytes("payload.bin", data).expect("write");
    zip.finish().expect("finish");
    let mut reader = ZipReader::open(&path).expect("open");
    let mut out = Vec::new();
    let n = reader.copy_entry(0, &mut out).expect("read");
    assert_eq!(n, data.len() as u64);
    assert_eq!(out, data);
    let _ = fs::remove_file(&path);
}

fn bench_inflate() {
    let raw = payload(PAYLOAD);
    let wrapped = export_pack::zlib_store(&raw);
    let _ = export_pack::inflate_zlib(&wrapped).expect("inflate warmup");
    let mut rates = Vec::with_capacity(PASSES);
    let mut allocs = Vec::with_capacity(PASSES);
    let mut alloc_bytes = Vec::with_capacity(PASSES);
    for _ in 0..PASSES {
        reset();
        let t0 = Instant::now();
        let out = export_pack::inflate_zlib(black_box(&wrapped)).expect("inflate");
        let dur = t0.elapsed();
        let (a, b) = snapshot();
        assert_eq!(out.len(), raw.len());
        black_box(out);
        rates.push(mb_s(raw.len() as u64, dur));
        allocs.push(a as f64 / (raw.len() as f64 / 1024.0 / 1024.0));
        alloc_bytes.push(b as f64 / (raw.len() as f64 / 1024.0 / 1024.0));
    }
    println!("inflate_bytes={}", raw.len());
    println!("inflate_kind=stored-deflate-blocks");
    println!("inflate_passes={PASSES}");
    print_spread("inflate_mb_s", &rates);
    print_spread("inflate_allocs_per_mib", &allocs);
    print_spread("inflate_alloc_bytes_per_mib", &alloc_bytes);
    println!("note=inflate is zlib_store output, CMF/FLG 0x78 0x01, not a Huffman stream");
    println!("note=timed region is inflate_zlib only, including the Adler-32 check");
}

fn bench_pdf(dir: &std::path::Path) {
    const PAGES: usize = 8;
    const WIDTH: u32 = 2048;
    const HEIGHT: u32 = 1024;
    let rgb = vec![0x7Fu8; (WIDTH as usize) * (HEIGHT as usize) * 3];
    let png = export_pack::encode_png_rgb(WIDTH, HEIGHT, &rgb).expect("png");
    let cbz = dir.join("bench.cbz");
    let pdf = dir.join("bench.pdf");
    let _ = fs::remove_file(&cbz);
    let mut zip = export_pack::ZipWriter::create(&cbz).expect("cbz");
    for i in 0..PAGES {
        let name = format!("page-{i}.png");
        zip.write_bytes(&name, &png).expect("page");
    }
    zip.finish().expect("cbz finish");
    let pixel_bytes = (PAGES as u64) * (WIDTH as u64) * (HEIGHT as u64) * 3;
    let _ = export_pack::cbz_to_pdf(&cbz, &pdf).expect("pdf warmup");
    let _ = fs::remove_file(&pdf);
    let mut rates = Vec::with_capacity(PASSES);
    let mut allocs = Vec::with_capacity(PASSES);
    let mut alloc_bytes = Vec::with_capacity(PASSES);
    for _ in 0..PASSES {
        let _ = fs::remove_file(&pdf);
        reset();
        let t0 = Instant::now();
        let info = export_pack::cbz_to_pdf(&cbz, &pdf).expect("pdf");
        let dur = t0.elapsed();
        let (a, b) = snapshot();
        assert_eq!(info.pages, PAGES);
        assert_eq!(info.pixel_bytes, pixel_bytes);
        let mib = pixel_bytes as f64 / 1024.0 / 1024.0;
        rates.push(mb_s(pixel_bytes, dur));
        allocs.push(a as f64 / mib);
        alloc_bytes.push(b as f64 / mib);
    }
    println!("pdf_pages={PAGES}");
    println!("pdf_png=stored-zlib");
    println!("pdf_pixel_bytes={pixel_bytes}");
    println!("pdf_passes={PASSES}");
    print_spread("pdf_mb_s", &rates);
    print_spread("pdf_allocs_per_mib", &allocs);
    print_spread("pdf_alloc_bytes_per_mib", &alloc_bytes);
    println!("note=pdf rate counts decoded RGB bytes, not the CBZ size");
    println!(
        "note=cbz_to_pdf includes zip read, stored-block inflate, pdf write, fsync, and rename"
    );
    println!("note=the CBZ stays in the page cache after the warmup pass");
    let _ = fs::remove_file(&cbz);
    let _ = fs::remove_file(&pdf);
}

fn bench_pdf16(dir: &std::path::Path) {
    const PAGES: usize = 16;
    const WIDTH: u32 = 2048;
    const HEIGHT: u32 = 1024;
    const PASSES_16: usize = 100;
    let rgb = vec![0x7Fu8; (WIDTH as usize) * (HEIGHT as usize) * 3];
    let png = export_pack::encode_png_rgb(WIDTH, HEIGHT, &rgb).expect("png");
    let cbz = dir.join("bench16.cbz");
    let pdf = dir.join("bench16.pdf");
    let _ = fs::remove_file(&cbz);
    let mut zip = export_pack::ZipWriter::create(&cbz).expect("cbz");
    for i in 0..PAGES {
        let name = format!("page-{i}.png");
        zip.write_bytes(&name, &png).expect("page");
    }
    zip.finish().expect("cbz finish");
    let pixel_bytes = (PAGES as u64) * (WIDTH as u64) * (HEIGHT as u64) * 3;
    let _ = export_pack::cbz_to_pdf(&cbz, &pdf).expect("pdf warmup");
    let _ = fs::remove_file(&pdf);
    let mut rates = Vec::with_capacity(PASSES_16);
    for _ in 0..PASSES_16 {
        let _ = fs::remove_file(&pdf);
        let t0 = Instant::now();
        let info = export_pack::cbz_to_pdf(&cbz, &pdf).expect("pdf");
        let dur = t0.elapsed();
        assert_eq!(info.pages, PAGES);
        assert_eq!(info.pixel_bytes, pixel_bytes);
        rates.push(mb_s(pixel_bytes, dur));
    }
    println!("pdf16_pages={PAGES}");
    println!("pdf16_width={WIDTH}");
    println!("pdf16_height={HEIGHT}");
    println!("pdf16_png=stored-zlib");
    println!("pdf16_pixel_bytes={pixel_bytes}");
    println!("pdf16_passes={PASSES_16}");
    println!("pdf16_mb_s_p50={:.1}", percentile(&rates, 50.0));
    println!("pdf16_mb_s_p99={:.1}", percentile(&rates, 99.0));
    println!("note=pdf16 rate counts decoded RGB bytes, same image as the eight-page run");
    println!(
        "note=pdf16 percentile is nearest-rank on MB/s sorted ascending, so p99 is the fast tail"
    );
    println!(
        "note=cbz_to_pdf includes zip read, stored-block inflate, pdf write, fsync, and rename"
    );
    println!("note=the 16-page CBZ stays in the page cache after the warmup pass");
    let _ = fs::remove_file(&cbz);
    let _ = fs::remove_file(&pdf);
}

fn bench_pdf32(dir: &std::path::Path) {
    const PAGES: usize = 32;
    const WIDTH: u32 = 2048;
    const HEIGHT: u32 = 1024;
    const PASSES_32: usize = 100;
    let rgb = vec![0x7Fu8; (WIDTH as usize) * (HEIGHT as usize) * 3];
    let png = export_pack::encode_png_rgb(WIDTH, HEIGHT, &rgb).expect("png");
    let cbz = dir.join("bench32.cbz");
    let pdf = dir.join("bench32.pdf");
    let _ = fs::remove_file(&cbz);
    let mut zip = export_pack::ZipWriter::create(&cbz).expect("cbz");
    for i in 0..PAGES {
        let name = format!("page-{i}.png");
        zip.write_bytes(&name, &png).expect("page");
    }
    zip.finish().expect("cbz finish");
    let pixel_bytes = (PAGES as u64) * (WIDTH as u64) * (HEIGHT as u64) * 3;
    let _ = export_pack::cbz_to_pdf(&cbz, &pdf).expect("pdf warmup");
    let _ = fs::remove_file(&pdf);
    let mut rates = Vec::with_capacity(PASSES_32);
    for _ in 0..PASSES_32 {
        let _ = fs::remove_file(&pdf);
        let t0 = Instant::now();
        let info = export_pack::cbz_to_pdf(&cbz, &pdf).expect("pdf");
        let dur = t0.elapsed();
        assert_eq!(info.pages, PAGES);
        assert_eq!(info.pixel_bytes, pixel_bytes);
        rates.push(mb_s(pixel_bytes, dur));
    }
    println!("pdf32_pages={PAGES}");
    println!("pdf32_width={WIDTH}");
    println!("pdf32_height={HEIGHT}");
    println!("pdf32_png=stored-zlib");
    println!("pdf32_pixel_bytes={pixel_bytes}");
    println!("pdf32_passes={PASSES_32}");
    println!("pdf32_mb_s_p50={:.1}", percentile(&rates, 50.0));
    println!("pdf32_mb_s_p99={:.1}", percentile(&rates, 99.0));
    println!("note=pdf32 rate counts decoded RGB bytes, same image as the eight-page run");
    println!(
        "note=pdf32 percentile is nearest-rank on MB/s sorted ascending, so p99 is the fast tail"
    );
    println!(
        "note=cbz_to_pdf includes zip read, stored-block inflate, pdf write, fsync, and rename"
    );
    println!("note=the 32-page CBZ stays in the page cache after the warmup pass");
    let _ = fs::remove_file(&cbz);
    let _ = fs::remove_file(&pdf);
}

fn percentile(values: &[f64], pct: f64) -> f64 {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let rank = ((pct / 100.0) * sorted.len() as f64).ceil() as usize;
    let idx = rank.clamp(1, sorted.len()) - 1;
    sorted[idx]
}

fn bench_font() {
    const REPEATS: u64 = 5_000;
    let ttf = export_pack::tiny_ttf();
    let _ = export_pack::convert_font(&ttf, export_pack::FontContainer::Otf).expect("font warmup");
    let bytes = (ttf.len() as u64).saturating_mul(REPEATS);
    let mut rates = Vec::with_capacity(PASSES);
    let mut allocs = Vec::with_capacity(PASSES);
    let mut alloc_bytes = Vec::with_capacity(PASSES);
    for _ in 0..PASSES {
        reset();
        let t0 = Instant::now();
        for _ in 0..REPEATS {
            black_box(
                export_pack::convert_font(black_box(&ttf), export_pack::FontContainer::Otf)
                    .expect("font"),
            );
        }
        let dur = t0.elapsed();
        let (a, b) = snapshot();
        let mib = bytes as f64 / 1024.0 / 1024.0;
        rates.push(mb_s(bytes, dur));
        allocs.push(a as f64 / mib);
        alloc_bytes.push(b as f64 / mib);
    }
    println!("font_bytes={}", ttf.len());
    println!("font_repeats={REPEATS}");
    println!("font_passes={PASSES}");
    print_spread("font_mb_s", &rates);
    print_spread("font_allocs_per_mib", &allocs);
    print_spread("font_alloc_bytes_per_mib", &alloc_bytes);
    println!("note=font rate is a repeated tiny-font container copy, not a bulk font benchmark");
}

fn bench_font_roundtrip() {
    const PASSES_RT: usize = 100;
    let ttf = export_pack::tiny_ttf();
    assert_eq!(ttf.len(), 492, "existing tiny font");
    let otf = export_pack::convert_font(&ttf, export_pack::FontContainer::Otf).expect("otf");
    let back = export_pack::convert_font(&otf, export_pack::FontContainer::Ttf).expect("ttf");
    assert_eq!(back, ttf, "round-trip bytes");
    let mut micros = Vec::with_capacity(PASSES_RT);
    for _ in 0..PASSES_RT {
        let t0 = Instant::now();
        let otf = export_pack::convert_font(black_box(&ttf), export_pack::FontContainer::Otf)
            .expect("otf");
        let back = export_pack::convert_font(black_box(&otf), export_pack::FontContainer::Ttf)
            .expect("ttf");
        let dur = t0.elapsed();
        assert_eq!(back, ttf);
        micros.push(dur.as_secs_f64() * 1_000_000.0);
    }
    println!("font_rt_bytes_in={}", ttf.len());
    println!("font_rt_bytes_out={}", back.len());
    println!("font_rt_passes={PASSES_RT}");
    println!("font_rt_us_p50={:.3}", percentile(&micros, 50.0));
    println!("font_rt_us_p99={:.3}", percentile(&micros, 99.0));
    println!("note=one sample is TTF to OTF to TTF and the bytes match");
    println!(
        "note=percentile is nearest-rank on microseconds sorted ascending, so p99 is the slow tail"
    );
}

const PASSES: usize = 5;

fn print_spread(name: &str, values: &[f64]) {
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let median = sorted[sorted.len() / 2];
    let min = sorted[0];
    let max = sorted[sorted.len() - 1];
    println!("{name}_median={median:.1}");
    println!("{name}_min={min:.1}");
    println!("{name}_max={max:.1}");
}

fn run(data: &[u8], dir: &std::path::Path) -> Sample {
    let path = dir.join("bench.zip");
    let _ = fs::remove_file(&path);
    reset();
    let t0 = Instant::now();
    let mut zip = ZipWriter::create(&path).expect("create");
    zip.write_bytes("payload.bin", data).expect("write");
    let write = t0.elapsed();
    let (write_allocs, write_alloc_bytes) = snapshot();
    let t1 = Instant::now();
    zip.finish().expect("finish");
    let finish = t1.elapsed();

    reset();
    let t2 = Instant::now();
    let mut reader = ZipReader::open(&path).expect("open");
    let mut sink = Sink(0);
    let n = reader.copy_entry(0, &mut sink).expect("read");
    let read = t2.elapsed();
    let (read_allocs, read_alloc_bytes) = snapshot();
    assert_eq!(n, data.len() as u64);
    assert_eq!(sink.0, data.len() as u64);
    let _ = fs::remove_file(&path);
    Sample {
        write,
        finish,
        read,
        write_allocs,
        write_alloc_bytes,
        read_allocs,
        read_alloc_bytes,
        bytes: data.len() as u64,
    }
}

fn payload(n: usize) -> Vec<u8> {
    let mut data = vec![0u8; n];
    for (i, byte) in data.iter_mut().enumerate() {
        *byte = (i as u8).wrapping_mul(3).wrapping_add(1);
    }
    data
}

fn env_dir() -> String {
    std::env::var("EXPORT_PACK_BENCH_DIR").unwrap_or_else(|_| "target/pack-bench".to_owned())
}

fn reset() {
    ALLOCS.store(0, Ordering::Relaxed);
    ALLOC_BYTES.store(0, Ordering::Relaxed);
}

fn snapshot() -> (u64, u64) {
    (
        ALLOCS.load(Ordering::Relaxed),
        ALLOC_BYTES.load(Ordering::Relaxed),
    )
}

fn mib_s(bytes: u64, dur: Duration) -> f64 {
    (bytes as f64 / 1024.0 / 1024.0) / dur.as_secs_f64()
}

fn mb_s(bytes: u64, dur: Duration) -> f64 {
    (bytes as f64 / 1_000_000.0) / dur.as_secs_f64()
}

fn ms(dur: Duration) -> f64 {
    dur.as_secs_f64() * 1000.0
}
