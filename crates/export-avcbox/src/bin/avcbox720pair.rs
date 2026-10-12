//! Release timing for boxing the 1280×720 I-frame and one P-frame. Does not call ffmpeg.
//!
//! Usage: avcbox720pair <out.mp4>

use std::env;
use std::fs;
use std::io::{self, Write};
use std::process::ExitCode;
use std::time::Instant;

use export_avcbox::{box_iframe, Options, PAIR_1280X720};

fn percentile(sorted: &[u128], p: f64) -> u128 {
    let rank = ((sorted.len() as f64) * p).ceil() as usize;
    let i = rank.clamp(1, sorted.len()) - 1;
    sorted[i]
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let path = match args.next() {
        Some(p) => p,
        None => {
            eprintln!("usage: avcbox720pair <out.mp4>");
            return ExitCode::from(2);
        }
    };

    let opts = Options { threads: 1 };
    let boxed = match box_iframe(PAIR_1280X720, &opts) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::from(1);
        }
    };

    for _ in 0..20 {
        let _ = box_iframe(PAIR_1280X720, &opts);
    }
    let mut samples = Vec::with_capacity(200);
    for _ in 0..200 {
        let t = Instant::now();
        let again = box_iframe(PAIR_1280X720, &opts).expect("720 pair box");
        samples.push(t.elapsed().as_nanos());
        debug_assert_eq!(again.mp4.len(), boxed.mp4.len());
    }
    samples.sort_unstable();
    let p50 = percentile(&samples, 0.50);
    let p99 = percentile(&samples, 0.99);

    if let Err(err) = fs::write(&path, &boxed.mp4) {
        eprintln!("write {path}: {err}");
        return ExitCode::from(1);
    }

    let mut out = io::stdout().lock();
    let _ = writeln!(
        out,
        "input_bytes={}\noutput_bytes={}\nwidth={}\nheight={}\nsamples={}\nthreads=1\nrelease_p50_ns={p50}\nrelease_p99_ns={p99}\nrelease_p50_us={}\nrelease_p99_us={}",
        PAIR_1280X720.len(),
        boxed.mp4.len(),
        boxed.width,
        boxed.height,
        boxed.samples,
        (p50 + 999) / 1000,
        (p99 + 999) / 1000
    );
    ExitCode::SUCCESS
}
