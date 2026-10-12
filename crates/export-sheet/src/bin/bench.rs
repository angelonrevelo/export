//! Rows per second, release build, 100_000 data rows.
//!
//! Allocations are counted for the conversion call only, with a process-wide
//! allocator. That count includes the output buffer growing, and it does not
//! include building the input. One record must not allocate on its own; if
//! this number tracks the row count, the stream is holding the table.

use std::alloc::{GlobalAlloc, Layout, System};
use std::io::Cursor;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_sheet::{convert, Format, Options, Stats};

static ALLOCS: AtomicUsize = AtomicUsize::new(0);

struct CountAlloc;

unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        System.alloc(layout)
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        System.dealloc(ptr, layout);
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

const DATA_ROWS: u64 = 100_000;

fn push_u64(out: &mut Vec<u8>, mut n: u64) {
    let mut tmp = [0u8; 20];
    let mut i = 20;
    if n == 0 {
        out.push(b'0');
        return;
    }
    while n > 0 {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    out.extend_from_slice(&tmp[i..]);
}

fn build_csv() -> Vec<u8> {
    let mut out = Vec::with_capacity((DATA_ROWS as usize) * 16);
    out.extend_from_slice(b"a,b,c,d\n");
    for i in 0..DATA_ROWS {
        push_u64(&mut out, i);
        out.extend_from_slice(b",1,2,3\n");
    }
    out
}

struct Sample {
    rows_per_sec: f64,
    allocs: usize,
    rows: u64,
    lossy: u64,
}

fn time_convert(input: &[u8], from: Format, to: Format) -> Sample {
    ALLOCS.store(0, Ordering::Relaxed);
    let started = Instant::now();
    let mut output = Vec::new();
    let stats = convert(
        from,
        to,
        Cursor::new(input),
        &mut output,
        Options::default(),
    )
    .expect("bench input is valid");
    let seconds = started.elapsed().as_secs_f64();
    let allocs = ALLOCS.load(Ordering::Relaxed);
    // Touch the output so a future writer cannot delete it as unused.
    assert!(!output.is_empty());
    Sample {
        rows_per_sec: (DATA_ROWS as f64) / seconds,
        allocs,
        rows: stats.rows,
        lossy: stats.lossy,
    }
}

fn best_of_three(input: &[u8], from: Format, to: Format, expect_rows: u64) -> Sample {
    let mut best: Option<Sample> = None;
    for _ in 0..3 {
        let sample = time_convert(input, from, to);
        assert_eq!(sample.rows, expect_rows);
        assert_eq!(sample.lossy, 0);
        let replace = match &best {
            None => true,
            Some(prev) => sample.rows_per_sec > prev.rows_per_sec,
        };
        if replace {
            best = Some(sample);
        }
    }
    best.expect("three runs")
}

fn main() {
    let csv = build_csv();
    let csv_json = best_of_three(&csv, Format::Csv, Format::Json, DATA_ROWS);
    let (json, stats) =
        export_sheet::convert_bytes(Format::Csv, Format::Json, &csv, Options::default())
            .expect("csv to json");
    assert_eq!(
        stats,
        Stats {
            rows: DATA_ROWS,
            lossy: 0
        }
    );
    let json_csv = best_of_three(&json, Format::Json, Format::Csv, DATA_ROWS + 1);
    let (back, _) =
        export_sheet::convert_bytes(Format::Json, Format::Csv, &json, Options::default())
            .expect("json to csv");
    assert_eq!(back, csv, "csv → json → csv changed a cell");
    let csv_tsv = best_of_three(&csv, Format::Csv, Format::Tsv, DATA_ROWS + 1);

    println!("rows {DATA_ROWS}");
    println!("columns 4");
    println!("runs 3");
    println!(
        "csv_to_json rows_per_sec={:.0} allocs={} output_rows={}",
        csv_json.rows_per_sec, csv_json.allocs, csv_json.rows
    );
    println!(
        "json_to_csv rows_per_sec={:.0} allocs={} output_rows={}",
        json_csv.rows_per_sec, json_csv.allocs, json_csv.rows
    );
    println!(
        "csv_to_tsv rows_per_sec={:.0} allocs={} output_rows={}",
        csv_tsv.rows_per_sec, csv_tsv.allocs, csv_tsv.rows
    );
    println!(
        "note rows_per_sec divides by {DATA_ROWS} data rows; allocs counts alloc+realloc during that one conversion, including output buffer growth"
    );
    xls_percentiles();
    xls_two_number_percentiles();
    xls_column_percentiles();
}

fn nearest_rank(sorted: &[u128], pct: usize) -> u128 {
    let rank = (pct * sorted.len() + 99) / 100;
    sorted[rank - 1]
}

fn xls_percentiles() {
    let file = export_sheet::generated_biff_xls();
    let (out, stats) =
        export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default())
            .expect("generated xls is BIFF");
    assert_eq!(stats.rows, 1);
    assert_eq!(out, b"42,cat\n");
    let samples_n = 10_000;
    for _ in 0..200 {
        let _ = export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default());
    }
    let mut samples = Vec::with_capacity(samples_n);
    for _ in 0..samples_n {
        let started = Instant::now();
        let got = export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default())
            .expect("generated xls is BIFF");
        let ns = started.elapsed().as_nanos();
        assert_eq!(got.0, b"42,cat\n");
        samples.push(ns);
    }
    samples.sort_unstable();
    let p50 = nearest_rank(&samples, 50);
    let p99 = nearest_rank(&samples, 99);
    println!("xls_values 42 cat");
    println!("xls_to_csv samples={samples_n} p50_ns={p50} p99_ns={p99}");
}

fn xls_two_number_percentiles() {
    let file = export_sheet::generated_biff_xls_two_numbers();
    let (out, stats) =
        export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default())
            .expect("two-number xls is BIFF");
    assert_eq!(stats.rows, 1);
    assert_eq!(out, b"42,cat,7\n");
    let samples_n = 10_000;
    for _ in 0..200 {
        let _ = export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default());
    }
    let mut samples = Vec::with_capacity(samples_n);
    for _ in 0..samples_n {
        let started = Instant::now();
        let got = export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default())
            .expect("two-number xls is BIFF");
        let ns = started.elapsed().as_nanos();
        assert_eq!(got.0, b"42,cat,7\n");
        samples.push(ns);
    }
    samples.sort_unstable();
    let p50 = nearest_rank(&samples, 50);
    let p99 = nearest_rank(&samples, 99);
    println!("xls_two_numbers values=42 cat 7");
    println!("xls_two_numbers samples={samples_n} p50_ns={p50} p99_ns={p99}");
}

fn xls_column_percentiles() {
    let file = export_sheet::generated_biff_xls_numeric_column();
    let (out, stats) =
        export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default())
            .expect("numeric column xls is BIFF");
    assert_eq!(stats.rows, 1000);
    assert!(out.starts_with(b"0\n") && out.ends_with(b"999\n"));
    let samples_n = 10_000;
    for _ in 0..200 {
        let _ = export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default());
    }
    let mut samples = Vec::with_capacity(samples_n);
    for _ in 0..samples_n {
        let started = Instant::now();
        let got = export_sheet::convert_bytes(Format::Xls, Format::Csv, &file, Options::default())
            .expect("numeric column xls is BIFF");
        let ns = started.elapsed().as_nanos();
        assert_eq!(got.0, out);
        samples.push(ns);
    }
    samples.sort_unstable();
    let p50 = nearest_rank(&samples, 50);
    let p99 = nearest_rank(&samples, 99);
    println!("xls_numeric_column rows=1000 file_bytes={}", file.len());
    println!("xls_numeric_column samples={samples_n} p50_ns={p50} p99_ns={p99}");
}
