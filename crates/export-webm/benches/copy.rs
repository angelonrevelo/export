//! Single-threaded release bench. Prints p50 and p99.
//! The sample set includes the truncated-input error.
//! This process does not spawn threads.

use export_webm::{sample_file, stream_copy, Error};
use std::time::Instant;

fn main() {
    let file = sample_file();
    let mut truncated = file.clone();
    truncated.truncate(truncated.len() / 2);

    const N: usize = 2000;
    let mut copy_ns = Vec::with_capacity(N);
    let mut truncated_ns = Vec::with_capacity(N);

    for _ in 0..32 {
        let _ = stream_copy(&file);
        let _ = stream_copy(&truncated);
    }

    for _ in 0..N {
        let start = Instant::now();
        let copied = stream_copy(&file).expect("sample copies");
        copy_ns.push(start.elapsed().as_nanos());
        assert!(!copied.bytes.is_empty());

        let start = Instant::now();
        let err = stream_copy(&truncated).expect_err("truncated fails");
        truncated_ns.push(start.elapsed().as_nanos());
        assert_eq!(err, Error::Truncated);
    }

    copy_ns.sort_unstable();
    truncated_ns.sort_unstable();
    let mut all = copy_ns.clone();
    all.extend_from_slice(&truncated_ns);
    all.sort_unstable();

    let copy_p50 = percentile(&copy_ns, 50.0);
    let bytes = file.len() as f64;
    let copy_p50_mib_s = if copy_p50 == 0 {
        0.0
    } else {
        (bytes / (copy_p50 as f64 / 1e9)) / (1024.0 * 1024.0)
    };

    println!("samples={N}");
    println!("threading=off");
    println!("p50_ns={}", percentile(&all, 50.0));
    println!("p99_ns={}", percentile(&all, 99.0));
    println!("copy_p50_ns={copy_p50}");
    println!("copy_p99_ns={}", percentile(&copy_ns, 99.0));
    println!("copy_p50_mib_s={copy_p50_mib_s:.3}");
    println!("truncated_errors={N}");
    println!("truncated_p50_ns={}", percentile(&truncated_ns, 50.0));
    println!("truncated_p99_ns={}", percentile(&truncated_ns, 99.0));
}

fn percentile(sorted: &[u128], pct: f64) -> u128 {
    let n = sorted.len();
    let rank = ((pct / 100.0) * n as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(n - 1)]
}
