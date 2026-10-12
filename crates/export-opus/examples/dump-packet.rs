//! Write one Opus packet and the PCM this crate decodes it to.
//!
//! `dump-packet silence packet.opus expected.pcm`
//! `dump-packet sine packet.opus expected.pcm`
//! `dump-packet sine-two first.opus second.opus followed.pcm`
//! `dump-packet sine-three a.opus b.opus c.opus file.pcm alone.pcm`
//! `dump-packet stereo packet.opus pcm.pcm`
//!
//! The host verification script may pass those files to ffmpeg. This
//! example does not call ffmpeg.

use export_opus::{
    decode, encode, encode_second_sine_frame, encode_stereo_frame, encode_third_sine_frame,
    sine_followed_frame, sine_third_frame, stereo_source, Pcm, FRAME_SAMPLES,
};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let kind = args.next().unwrap_or_else(|| "silence".to_string());
    if kind == "sine-two" {
        return dump_sine_two(args);
    }
    if kind == "sine-three" {
        return dump_sine_three(args);
    }
    if kind == "stereo" {
        return dump_stereo(args);
    }
    let packet_path = args.next().unwrap_or_else(|| "packet.opus".to_string());
    let pcm_path = args.next().unwrap_or_else(|| "expected.pcm".to_string());

    let source = match kind.as_str() {
        "silence" => [0i16; FRAME_SAMPLES],
        "sine" => export_opus::sine_source(),
        other => {
            eprintln!("unknown frame {other}");
            return ExitCode::from(1);
        }
    };
    let packet = match encode(&source) {
        Ok(packet) => packet,
        Err(err) => {
            eprintln!("encode failed: {err}");
            return ExitCode::from(1);
        }
    };
    let decoded = match decode(&packet, 1, false) {
        Ok(Pcm::Mono(pcm)) => pcm,
        Ok(Pcm::Stereo(_)) => {
            eprintln!("mono decode returned two channels");
            return ExitCode::from(1);
        }
        Err(err) => {
            eprintln!("decode failed: {err}");
            return ExitCode::from(1);
        }
    };
    if kind == "silence" && decoded != source {
        eprintln!("silence roundtrip PCM mismatch");
        return ExitCode::from(1);
    }

    if let Err(err) = fs::write(&packet_path, &packet) {
        eprintln!("write {packet_path}: {err}");
        return ExitCode::from(1);
    }
    let mut bytes = Vec::with_capacity(FRAME_SAMPLES * 2);
    for sample in decoded {
        bytes.extend_from_slice(&sample.to_le_bytes());
    }
    if let Err(err) = fs::write(&pcm_path, &bytes) {
        eprintln!("write {pcm_path}: {err}");
        return ExitCode::from(1);
    }
    println!(
        "{kind} packet={} bytes pcm_samples={}",
        packet.len(),
        decoded.len()
    );
    ExitCode::SUCCESS
}

fn expect_mono(packet: &[u8]) -> [i16; FRAME_SAMPLES] {
    match decode(packet, 1, false) {
        Ok(Pcm::Mono(pcm)) => pcm,
        Ok(Pcm::Stereo(_)) => panic!("mono packet decoded as two channels"),
        Err(err) => panic!("decode failed: {err}"),
    }
}

fn dump_stereo(mut args: impl Iterator<Item = String>) -> ExitCode {
    let packet_path = args.next().unwrap_or_else(|| "stereo.opus".to_string());
    let pcm_path = args.next().unwrap_or_else(|| "stereo.pcm".to_string());
    let (source_left, source_right) = stereo_source();
    let packet = match encode_stereo_frame(&source_left, &source_right) {
        Ok(packet) => packet,
        Err(err) => {
            eprintln!("stereo encode failed: {err}");
            return ExitCode::from(1);
        }
    };
    let (left, right) = match decode(&packet, 1, false) {
        Ok(Pcm::Stereo(pcm)) => (pcm.left, pcm.right),
        Ok(Pcm::Mono(_)) => {
            eprintln!("stereo packet decoded as one channel");
            return ExitCode::from(1);
        }
        Err(err) => {
            eprintln!("stereo decode failed: {err}");
            return ExitCode::from(1);
        }
    };
    if let Err(err) = fs::write(&packet_path, &packet) {
        eprintln!("write {packet_path}: {err}");
        return ExitCode::from(1);
    }
    let mut bytes = Vec::with_capacity(FRAME_SAMPLES * 4);
    for index in 0..FRAME_SAMPLES {
        bytes.extend_from_slice(&left[index].to_le_bytes());
        bytes.extend_from_slice(&right[index].to_le_bytes());
    }
    if let Err(err) = fs::write(&pcm_path, &bytes) {
        eprintln!("write {pcm_path}: {err}");
        return ExitCode::from(1);
    }
    println!(
        "stereo packet={} bytes channels=2 samples_per_channel={} interleaved={}",
        packet.len(),
        FRAME_SAMPLES,
        FRAME_SAMPLES * 2
    );
    ExitCode::SUCCESS
}

fn dump_sine_two(mut args: impl Iterator<Item = String>) -> ExitCode {
    let first_path = args.next().unwrap_or_else(|| "sine.opus".to_string());
    let second_path = args.next().unwrap_or_else(|| "sine-2.opus".to_string());
    let pcm_path = args.next().unwrap_or_else(|| "followed.pcm".to_string());
    let alone_path = args.next();
    let first = match encode(&export_opus::sine_source()) {
        Ok(packet) => packet,
        Err(err) => {
            eprintln!("encode failed: {err}");
            return ExitCode::from(1);
        }
    };
    let second = encode_second_sine_frame();
    let mut pcm = Vec::with_capacity(FRAME_SAMPLES * 4);
    for sample in expect_mono(&first) {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    for sample in sine_followed_frame() {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    if fs::write(&first_path, &first).is_err() || fs::write(&second_path, &second).is_err() {
        eprintln!("write packets failed");
        return ExitCode::from(1);
    }
    if let Err(err) = fs::write(&pcm_path, &pcm) {
        eprintln!("write {pcm_path}: {err}");
        return ExitCode::from(1);
    }
    if let Some(alone_path) = alone_path {
        let mut alone = Vec::with_capacity(FRAME_SAMPLES * 2);
        for sample in expect_mono(&second) {
            alone.extend_from_slice(&sample.to_le_bytes());
        }
        if let Err(err) = fs::write(&alone_path, &alone) {
            eprintln!("write {alone_path}: {err}");
            return ExitCode::from(1);
        }
    }
    println!(
        "sine-two packets={} {} pcm_samples={}",
        first.len(),
        second.len(),
        FRAME_SAMPLES * 2
    );
    ExitCode::SUCCESS
}

fn dump_sine_three(mut args: impl Iterator<Item = String>) -> ExitCode {
    let first_path = args.next().unwrap_or_else(|| "sine.opus".to_string());
    let second_path = args.next().unwrap_or_else(|| "sine-2.opus".to_string());
    let third_path = args.next().unwrap_or_else(|| "sine-3.opus".to_string());
    let pcm_path = args.next().unwrap_or_else(|| "three.pcm".to_string());
    let alone_path = args.next();
    let first = match encode(&export_opus::sine_source()) {
        Ok(packet) => packet,
        Err(err) => {
            eprintln!("encode failed: {err}");
            return ExitCode::from(1);
        }
    };
    let second = encode_second_sine_frame();
    let third = encode_third_sine_frame();
    let mut pcm = Vec::with_capacity(FRAME_SAMPLES * 6);
    for sample in expect_mono(&first) {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    for sample in sine_followed_frame() {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    for sample in sine_third_frame() {
        pcm.extend_from_slice(&sample.to_le_bytes());
    }
    if fs::write(&first_path, &first).is_err()
        || fs::write(&second_path, &second).is_err()
        || fs::write(&third_path, &third).is_err()
    {
        eprintln!("write packets failed");
        return ExitCode::from(1);
    }
    if let Err(err) = fs::write(&pcm_path, &pcm) {
        eprintln!("write {pcm_path}: {err}");
        return ExitCode::from(1);
    }
    if let Some(alone_path) = alone_path {
        let mut alone = Vec::with_capacity(FRAME_SAMPLES * 2);
        for sample in expect_mono(&third) {
            alone.extend_from_slice(&sample.to_le_bytes());
        }
        if let Err(err) = fs::write(&alone_path, &alone) {
            eprintln!("write {alone_path}: {err}");
            return ExitCode::from(1);
        }
    }
    println!(
        "sine-three packets={} {} {} pcm_samples={}",
        first.len(),
        second.len(),
        third.len(),
        FRAME_SAMPLES * 3
    );
    ExitCode::SUCCESS
}
