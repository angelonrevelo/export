//! Single-threaded release bench.
//! Copy rows are copy-only. The truncated error is its own row.
//! This process does not spawn threads.

use export_webm::{
    large_sample_file, opus_vorbis_webm, opus_webm, sample_file, stream_copy, vorbis_webm,
    vp8_4096_webm, vp8_460800_webm, vp8_opus_webm, vp8_two_webm, vp8_webm, vp9_4096_webm, vp9_webm,
    Error,
};
use std::time::Instant;

fn main() {
    let small = sample_file();
    let large = large_sample_file();
    let vp8 = vp8_webm();
    let vp8_two = vp8_two_webm();
    let vp8_4096 = vp8_4096_webm();
    let vp8_460800 = vp8_460800_webm();
    let vp9 = vp9_webm();
    let vp9_4096 = vp9_4096_webm();
    let opus = opus_webm();
    let vorbis = vorbis_webm();
    let opus_vorbis = opus_vorbis_webm();
    let vp8_opus = vp8_opus_webm();
    let mut truncated = small.clone();
    truncated.truncate(truncated.len() / 2);

    let small_out = stream_copy(&small).expect("2x2 copies").bytes;
    let large_out = stream_copy(&large).expect("large copies").bytes;
    let vp8_out = stream_copy(vp8).expect("vp8 copies").bytes;
    let vp8_two_out = stream_copy(vp8_two).expect("two vp8 blocks copy").bytes;
    let vp8_4096_out = stream_copy(&vp8_4096).expect("4096-byte vp8 copies").bytes;
    let vp8_460800_out = stream_copy(&vp8_460800)
        .expect("460800-byte vp8 copies")
        .bytes;
    let vp9_out = stream_copy(vp9).expect("vp9 copies").bytes;
    let vp9_4096_out = stream_copy(&vp9_4096).expect("4096-byte vp9 copies").bytes;
    let opus_out = stream_copy(opus).expect("opus copies").bytes;
    let vorbis_out = stream_copy(vorbis).expect("vorbis copies").bytes;
    let opus_vorbis_out = stream_copy(&opus_vorbis)
        .expect("opus and vorbis copy")
        .bytes;
    let vp8_opus_out = stream_copy(&vp8_opus).expect("vp8 and opus copy").bytes;
    assert_eq!(
        stream_copy(&truncated).expect_err("truncated fails"),
        Error::Truncated
    );

    const N_SMALL: usize = 2000;
    const N_LARGE: usize = 400;

    for _ in 0..16 {
        let _ = stream_copy(&small);
        let _ = stream_copy(&large);
        let _ = stream_copy(vp8);
        let _ = stream_copy(vp8_two);
        let _ = stream_copy(&vp8_4096);
        let _ = stream_copy(&vp8_460800);
        let _ = stream_copy(vp9);
        let _ = stream_copy(&vp9_4096);
        let _ = stream_copy(opus);
        let _ = stream_copy(vorbis);
        let _ = stream_copy(&opus_vorbis);
        let _ = stream_copy(&vp8_opus);
        let _ = stream_copy(&truncated);
    }

    let small_ns = time_copy(&small, small_out.len(), N_SMALL);
    let large_ns = time_copy(&large, large_out.len(), N_LARGE);
    let vp8_ns = time_copy(vp8, vp8_out.len(), N_SMALL);
    let vp8_two_ns = time_copy(vp8_two, vp8_two_out.len(), N_SMALL);
    let vp8_4096_ns = time_copy(&vp8_4096, vp8_4096_out.len(), N_SMALL);
    let vp8_460800_ns = time_copy(&vp8_460800, vp8_460800_out.len(), N_LARGE);
    let vp9_ns = time_copy(vp9, vp9_out.len(), N_SMALL);
    let vp9_4096_ns = time_copy(&vp9_4096, vp9_4096_out.len(), N_SMALL);
    let opus_ns = time_copy(opus, opus_out.len(), N_SMALL);
    let vorbis_ns = time_copy(vorbis, vorbis_out.len(), N_SMALL);
    let opus_vorbis_ns = time_copy(&opus_vorbis, opus_vorbis_out.len(), N_SMALL);
    let vp8_opus_ns = time_copy(&vp8_opus, vp8_opus_out.len(), N_SMALL);
    let truncated_ns = time_truncated(&truncated, N_SMALL);

    println!("threading=off");
    print_copy_row("2x2", &small, &small_out, &small_ns);
    print_copy_row("large", &large, &large_out, &large_ns);
    print_copy_row("vp8", vp8, &vp8_out, &vp8_ns);
    print_copy_row("vp8_two", vp8_two, &vp8_two_out, &vp8_two_ns);
    print_copy_row("vp8_4096", &vp8_4096, &vp8_4096_out, &vp8_4096_ns);
    print_copy_row("vp8_460800", &vp8_460800, &vp8_460800_out, &vp8_460800_ns);
    print_copy_row("vp9", vp9, &vp9_out, &vp9_ns);
    print_copy_row("vp9_4096", &vp9_4096, &vp9_4096_out, &vp9_4096_ns);
    print_copy_row("opus", opus, &opus_out, &opus_ns);
    print_copy_row("vorbis", vorbis, &vorbis_out, &vorbis_ns);
    print_copy_row(
        "opus_vorbis",
        &opus_vorbis,
        &opus_vorbis_out,
        &opus_vorbis_ns,
    );
    print_copy_row("vp8_opus", &vp8_opus, &vp8_opus_out, &vp8_opus_ns);
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

fn print_copy_row(name: &str, input: &[u8], output: &[u8], times: &[u128]) {
    let p50 = percentile(times, 50.0);
    let p99 = percentile(times, 99.0);
    println!(
        "row={name} samples={} input_bytes={} output_bytes={} p50_ns={p50} p99_ns={p99} mb_s={:.3}",
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
