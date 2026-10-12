//! Release p50/p99 and size for the third AAC-LC frame.
//!
//! The frame overlap-adds against the second frame's history. Writes all
//! three ADTS frames, concatenated, to stdout. Prints counters on stderr.
//! Pass a path as the first argument to also store that three-frame file.
//! Does not call FFmpeg.

use std::io::{self, Write};
use std::time::Instant;

const SAMPLES: usize = 200;

fn percentile(samples: &mut [u128], p: f64) -> u128 {
    samples.sort_unstable();
    let n = samples.len();
    let rank = (p * n as f64).ceil() as usize;
    samples[rank.clamp(1, n) - 1]
}

fn main() {
    let first_pcm = export_aac::one_frame_pcm();
    let second_pcm = export_aac::second_frame_pcm();
    let third_pcm = export_aac::third_frame_pcm();
    let first = export_aac::encode_frame(&first_pcm).expect("first");
    let second = export_aac::encode_frame_following(&first_pcm, &second_pcm).expect("second");
    assert_eq!(first.len(), 513);
    assert_eq!(second.len(), 68);
    let third = export_aac::encode_frame_following(&second_pcm, &third_pcm).expect("third");
    let decoded = export_aac::decode_third_frame(&first, &second, &third).expect("decode");
    let mut late = 0.0f32;
    let mut own = 0.0f32;
    let mut peak = 0.0f32;
    for i in 0..second_pcm.len() {
        late = late.max((second_pcm[i] - decoded[i]).abs());
        own = own.max((third_pcm[i] - decoded[i]).abs());
        peak = peak.max(decoded[i].abs());
    }

    let mut encode_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let encoded = export_aac::encode_frame_following(&second_pcm, &third_pcm).expect("encode");
        encode_ns.push(started.elapsed().as_nanos());
        assert_eq!(encoded.len(), third.len());
    }

    let encode_p50 = percentile(&mut encode_ns, 0.50);
    let encode_p99 = percentile(&mut encode_ns, 0.99);

    let mut err = io::stderr().lock();
    let _ = writeln!(err, "samples={SAMPLES}");
    let _ = writeln!(err, "output_bytes={}", third.len());
    let _ = writeln!(err, "encode_p50_ns={encode_p50}");
    let _ = writeln!(err, "encode_p99_ns={encode_p99}");
    let _ = writeln!(err, "max_abs_error_vs_previous_sine={late}");
    let _ = writeln!(err, "max_abs_error_vs_own_input={own}");
    let _ = writeln!(err, "peak={peak}");

    let mut file = Vec::with_capacity(first.len() + second.len() + third.len());
    file.extend_from_slice(&first);
    file.extend_from_slice(&second);
    file.extend_from_slice(&third);
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, &file).unwrap_or_else(|e| panic!("write {path}: {e}"));
    }
    io::stdout().write_all(&file).expect("stdout");
}
