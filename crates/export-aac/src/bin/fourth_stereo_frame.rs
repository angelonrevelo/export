//! Release p50/p99 for the fourth AAC-LC stereo frame.
//!
//! Left continues the half-scale 440 Hz sine. Right stays silence. Same long
//! sine window. Mid-side stays off. Times only this frame's encode. Writes
//! the four ADTS frames, concatenated, to stdout. Prints counters on stderr.
//! Pass a path as the first argument to also store that file. Does not call
//! FFmpeg.

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
    let previous = export_aac::third_frame_pcm();
    let next = export_aac::fourth_frame_pcm();
    let silence = [0.0f32; export_aac::FRAME_SAMPLES];
    let first =
        export_aac::encode_stereo_frame(&export_aac::one_frame_pcm(), &silence).expect("first");
    let second = export_aac::encode_second_stereo_frame().expect("second");
    let third = export_aac::encode_third_stereo_frame().expect("third");
    assert_eq!(first.len(), 517);
    assert_eq!(second.len(), 71);
    assert_eq!(third.len(), 97);
    let fourth = export_aac::encode_fourth_stereo_frame().expect("fourth");
    let decoded =
        export_aac::decode_fourth_stereo_frame(&first, &second, &third, &fourth).expect("decode");
    let mut left_err = 0.0f32;
    let mut right_peak = 0.0f32;
    for ((left, src), right) in decoded
        .left
        .iter()
        .zip(previous.iter())
        .zip(decoded.right.iter())
    {
        left_err = left_err.max((left - src).abs());
        right_peak = right_peak.max(right.abs());
    }
    assert!(left_err <= 0.01, "left error {left_err}");
    assert!(right_peak <= 0.01, "right peak {right_peak}");

    let mut encode_ns = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let encoded =
            export_aac::encode_stereo_frame_following(&previous, &silence, &next, &silence)
                .expect("encode");
        encode_ns.push(started.elapsed().as_nanos());
        assert_eq!(encoded.len(), fourth.len());
    }
    let encode_p50 = percentile(&mut encode_ns, 0.50);
    let encode_p99 = percentile(&mut encode_ns, 0.99);

    let mut err = io::stderr().lock();
    let _ = writeln!(err, "samples={SAMPLES}");
    let _ = writeln!(err, "output_bytes={}", fourth.len());
    let _ = writeln!(err, "encode_p50_ns={encode_p50}");
    let _ = writeln!(err, "encode_p99_ns={encode_p99}");
    let _ = writeln!(err, "left_max_abs_error_vs_delayed_sine={left_err}");
    let _ = writeln!(err, "right_peak={right_peak}");

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
