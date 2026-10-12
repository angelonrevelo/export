//! Release p50/p99 for the flush frame after the four tone frames.
//!
//! Encode is the 945-byte flush. Decode is that same frame after the overlap
//! tail from the fourth frame already exists. The tail copy is outside the
//! timer. Writes the four tone frames and the flush, concatenated, to stdout.
//! Prints counters on stderr. Pass a path as the first argument to also store
//! that file. Does not call FFmpeg.

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
    let fourth = export_aac::encode_frame_following(&third_pcm, &fourth_pcm).expect("fourth");
    assert_eq!(first.len(), 513);
    assert_eq!(second.len(), 68);
    assert_eq!(third.len(), 94);
    assert_eq!(fourth.len(), 60);
    let flush = export_aac::encode_flush_frame().expect("flush");
    let decoded =
        export_aac::decode_flush_frame(&first, &second, &third, &fourth, &flush).expect("decode");
    let mut vs_sine = 0.0f32;
    let mut vs_silence = 0.0f32;
    for i in 0..fourth_pcm.len() {
        vs_sine = vs_sine.max((fourth_pcm[i] - decoded[i]).abs());
        vs_silence = vs_silence.max(decoded[i].abs());
    }

    let mut encode_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let encoded = export_aac::encode_flush_frame().expect("encode");
        encode_ns.push(started.elapsed().as_nanos());
        assert_eq!(encoded.len(), flush.len());
    }
    let encode_p50 = percentile(&mut encode_ns, 0.50);
    let encode_p99 = percentile(&mut encode_ns, 0.99);

    assert_eq!(flush.len(), 945);
    let mut tail = export_aac::overlap_tail_after_third(&first, &second, &third).expect("tail");
    export_aac::decode_frame_with_tail(&fourth, &mut tail).expect("fourth");
    let mut once = tail;
    let primed = export_aac::decode_frame_with_tail(&flush, &mut once).expect("primed");
    assert_eq!(primed, decoded);
    let mut decode_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let mut history = tail;
        let started = Instant::now();
        let out = export_aac::decode_frame_with_tail(&flush, &mut history).expect("decode");
        decode_ns.push(started.elapsed().as_nanos());
        assert_eq!(out, decoded);
    }
    let decode_p50 = percentile(&mut decode_ns, 0.50);
    let decode_p99 = percentile(&mut decode_ns, 0.99);

    let mut err = io::stderr().lock();
    let _ = writeln!(err, "samples={SAMPLES}");
    let _ = writeln!(err, "output_bytes={}", flush.len());
    // encode_* is this run only. The published encode row stays 5893053 ns
    // p50 and 7642850 ns p99. decode_tail_ready_* is the 945-byte flush after
    // the overlap tail exists, not the 60-byte frame (5055180 / 8760302 ns).
    let _ = writeln!(err, "encode_p50_ns={encode_p50}");
    let _ = writeln!(err, "encode_p99_ns={encode_p99}");
    let _ = writeln!(err, "decode_tail_ready_p50_ns={decode_p50}");
    let _ = writeln!(err, "decode_tail_ready_p99_ns={decode_p99}");
    let _ = writeln!(err, "max_abs_error_vs_delayed_sine={vs_sine}");
    let _ = writeln!(err, "max_abs_error_vs_silence={vs_silence}");

    let mut file =
        Vec::with_capacity(first.len() + second.len() + third.len() + fourth.len() + flush.len());
    file.extend_from_slice(&first);
    file.extend_from_slice(&second);
    file.extend_from_slice(&third);
    file.extend_from_slice(&fourth);
    file.extend_from_slice(&flush);
    if let Some(path) = std::env::args().nth(1) {
        std::fs::write(&path, &file).unwrap_or_else(|e| panic!("write {path}: {e}"));
    }
    io::stdout().write_all(&file).expect("stdout");
}
