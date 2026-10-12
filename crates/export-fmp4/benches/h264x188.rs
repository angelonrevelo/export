//! Release timing for 188 copies of the 16×16 access unit.
//! This process does not spawn threads and does not decode the bitstream.

use export_fmp4::{h264_x188_file, stream_copy, H264_X188};
use std::time::Instant;

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("debug assertions are on; this is not a release measurement");
        std::process::exit(2);
    }

    let input = h264_x188_file();
    let output = stream_copy(&input).expect("h264x188 copy").bytes;
    assert!(output.windows(4).any(|window| window == *b"avc1"));
    assert!(output.windows(4).any(|window| window == *b"moof"));
    assert_eq!(H264_X188, 188);

    const N: usize = 2000;
    for _ in 0..16 {
        let _ = stream_copy(&input);
    }

    let mut times = Vec::with_capacity(N);
    for _ in 0..N {
        let start = Instant::now();
        let copied = stream_copy(&input).expect("h264x188 copy");
        let elapsed = start.elapsed().as_nanos();
        assert_eq!(copied.bytes.len(), output.len());
        times.push(elapsed);
    }
    times.sort_unstable();

    let p50 = percentile(&times, 50.0);
    let p99 = percentile(&times, 99.0);
    println!("threading=off");
    println!(
        "row=h264x188 samples={} input_bytes={} output_bytes={} p50_ns={p50} p99_ns={p99}",
        times.len(),
        input.len(),
        output.len(),
    );
}

fn percentile(sorted: &[u128], pct: f64) -> u128 {
    let n = sorted.len();
    let rank = ((pct / 100.0) * n as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(n - 1)]
}
