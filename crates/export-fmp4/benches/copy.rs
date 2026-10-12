//! Single-threaded release bench.
//! The copy row is copy-only. The truncated error is its own row.
//! This process does not spawn threads.

use export_fmp4::{sample_file, stream_copy, Error};
use std::time::Instant;

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("debug assertions are on; this is not a release measurement");
        std::process::exit(2);
    }

    let input = sample_file();
    let output = stream_copy(&input).expect("copy").bytes;
    let mut truncated = input.clone();
    truncated.truncate(truncated.len() / 2);
    assert_eq!(
        stream_copy(&truncated).expect_err("truncated fails"),
        Error::Truncated
    );

    const N_COPY: usize = 200;
    const N_TRUNC: usize = 400;

    for _ in 0..4 {
        let _ = stream_copy(&input);
        let _ = stream_copy(&truncated);
    }

    let copy_ns = time_copy(&input, output.len(), N_COPY);
    let truncated_ns = time_truncated(&truncated, N_TRUNC);

    println!("threading=off");
    print_copy_row(&input, &output, &copy_ns);
    print_truncated_row(&truncated, &truncated_ns);
}

fn time_copy(input: &[u8], expected_len: usize, samples: usize) -> Vec<u128> {
    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        let copied = stream_copy(input).expect("copy");
        let elapsed = start.elapsed().as_nanos();
        assert_eq!(copied.bytes.len(), expected_len);
        times.push(elapsed);
    }
    times.sort_unstable();
    times
}

fn time_truncated(input: &[u8], samples: usize) -> Vec<u128> {
    let mut times = Vec::with_capacity(samples);
    for _ in 0..samples {
        let start = Instant::now();
        let err = stream_copy(input).expect_err("truncated");
        times.push(start.elapsed().as_nanos());
        assert_eq!(err, Error::Truncated);
    }
    times.sort_unstable();
    times
}

fn print_copy_row(input: &[u8], output: &[u8], times: &[u128]) {
    let p50 = percentile(times, 50.0);
    let p99 = percentile(times, 99.0);
    println!(
        "row=copy samples={} input_bytes={} output_bytes={} p50_ns={p50} p99_ns={p99} mb_s={:.3}",
        times.len(),
        input.len(),
        output.len(),
        mb_s(input.len(), p50),
    );
}

fn print_truncated_row(input: &[u8], times: &[u128]) {
    let p50 = percentile(times, 50.0);
    let p99 = percentile(times, 99.0);
    println!(
        "row=truncated samples={} input_bytes={} p50_ns={p50} p99_ns={p99}",
        times.len(),
        input.len(),
    );
}

/// Decimal megabytes per second from input bytes and the p50 copy time.
fn mb_s(input_bytes: usize, p50_ns: u128) -> f64 {
    if p50_ns == 0 {
        return 0.0;
    }
    (input_bytes as f64) / (p50_ns as f64 / 1e9) / 1_000_000.0
}

fn percentile(sorted: &[u128], pct: f64) -> u128 {
    let n = sorted.len();
    let rank = ((pct / 100.0) * n as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(n - 1)]
}
