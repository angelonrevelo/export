//! Release p50/p99 for the fourth AAC-LC frame.
//!
//! Encode is the fourth frame. Decode is that same frame after the overlap
//! tail from the earlier three frames already exists. The published
//! `decode_fourth_frame` row includes those earlier frames and is not
//! reprinted here. Writes all four ADTS frames, concatenated, to stdout.
//! Prints counters on stderr. Pass a path as the first argument to also
//! store that four-frame file. Does not call FFmpeg.

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
    let fourth_pcm = export_aac::fourth_frame_pcm();
    let first = export_aac::encode_frame(&first_pcm).expect("first");
    let second = export_aac::encode_frame_following(&first_pcm, &second_pcm).expect("second");
    let third = export_aac::encode_frame_following(&second_pcm, &third_pcm).expect("third");
    assert_eq!(first.len(), 513);
    assert_eq!(second.len(), 68);
    assert_eq!(third.len(), 94);
    let fourth = export_aac::encode_frame_following(&third_pcm, &fourth_pcm).expect("fourth");
    let decoded =
        export_aac::decode_fourth_frame(&first, &second, &third, &fourth).expect("decode");
    let mut late = 0.0f32;
    let mut own = 0.0f32;
    for i in 0..third_pcm.len() {
        late = late.max((third_pcm[i] - decoded[i]).abs());
        own = own.max((fourth_pcm[i] - decoded[i]).abs());
    }

    let mut encode_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let encoded = export_aac::encode_frame_following(&third_pcm, &fourth_pcm).expect("encode");
        encode_ns.push(started.elapsed().as_nanos());
        assert_eq!(encoded.len(), fourth.len());
    }

    let encode_p50 = percentile(&mut encode_ns, 0.50);
    let encode_p99 = percentile(&mut encode_ns, 0.99);

    assert_eq!(fourth.len(), 60);
    let tail = export_aac::overlap_tail_after_third(&first, &second, &third).expect("tail");
    let mut once = tail;
    let primed = export_aac::decode_frame_with_tail(&fourth, &mut once).expect("primed");
    assert_eq!(primed, decoded);
    let mut decode_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let mut history = tail;
        let started = Instant::now();
        let out = export_aac::decode_frame_with_tail(&fourth, &mut history).expect("decode");
        decode_ns.push(started.elapsed().as_nanos());
        assert_eq!(out, decoded);
    }
    let decode_p50 = percentile(&mut decode_ns, 0.50);
    let decode_p99 = percentile(&mut decode_ns, 0.99);

    let mut err = io::stderr().lock();
    let _ = writeln!(err, "samples={SAMPLES}");
    let _ = writeln!(err, "output_bytes={}", fourth.len());
    // encode_* is this run only. The published encode row stays 6402279 ns
    // p50 and 7588843 ns p99. decode_tail_ready_* is the fourth frame after
    // the overlap tail exists, not decode_fourth_frame (20837888 / 22949440 ns).
    let _ = writeln!(err, "encode_p50_ns={encode_p50}");
    let _ = writeln!(err, "encode_p99_ns={encode_p99}");
    let _ = writeln!(err, "decode_tail_ready_p50_ns={decode_p50}");
    let _ = writeln!(err, "decode_tail_ready_p99_ns={decode_p99}");
    let _ = writeln!(err, "max_abs_error_vs_delayed_sine={late}");
    let _ = writeln!(err, "max_abs_error_vs_own_input={own}");

    let mut file = Vec::with_capacity(first.len() + second.len() + third.len() + fourth.len());
    file.extend_from_slice(&first);
    file.extend_from_slice(&second);
    file.extend_from_slice(&third);
    file.extend_from_slice(&fourth);
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, &file).unwrap_or_else(|e| panic!("write {path}: {e}"));
    }
    io::stdout().write_all(&file).expect("stdout");
}
