//! Write one Opus packet and the PCM this crate decodes it to.
//!
//! `dump-packet silence packet.opus expected.pcm`
//! `dump-packet sine packet.opus expected.pcm`
//! `dump-packet sine-two first.opus second.opus followed.pcm`
//! `dump-packet sine-three a.opus b.opus c.opus file.pcm alone.pcm`
//!
//! The host verification script may pass those files to ffmpeg. This
//! example does not call ffmpeg.

use export_opus::{
    decode, encode, encode_second_sine_frame, encode_stereo_frame, encode_third_sine_frame,
    sine_followed_frame, sine_third_frame, FRAME_SAMPLES,
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
        Ok(pcm) => pcm,
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

fn dump_stereo(mut args: impl Iterator<Item = String>) -> ExitCode {
    let packet_path = args.next().unwrap_or_else(|| "stereo.opus".to_string());
    let packet = encode_stereo_frame();
    if decode(&packet, 1, false) != Err(export_opus::Error::Unsupported) {
        eprintln!("stereo decode was accepted");
        return ExitCode::from(1);
    }
    if let Err(err) = fs::write(&packet_path, &packet) {
        eprintln!("write {packet_path}: {err}");
        return ExitCode::from(1);
    }
    println!("stereo packet={} bytes", packet.len());
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
    for sample in decode(&first, 1, false).expect("first frame") {
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
        for sample in decode(&second, 1, false).expect("second frame") {
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
    for sample in decode(&first, 1, false).expect("first frame") {
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
        for sample in decode(&third, 1, false).expect("third frame") {
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
