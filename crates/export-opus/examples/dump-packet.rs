//! Write the one Opus packet and the PCM this crate decodes it to.
//! The host verification script may pass those files to ffmpeg. This
//! example does not call ffmpeg.

use export_opus::{decode, encode, FRAME_SAMPLES};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let packet_path = args.next().unwrap_or_else(|| "packet.opus".to_string());
    let pcm_path = args.next().unwrap_or_else(|| "expected.pcm".to_string());

    let pcm = [0i16; FRAME_SAMPLES];
    let packet = match encode(&pcm) {
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
    if decoded != pcm {
        eprintln!("roundtrip PCM mismatch");
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
        "packet={} bytes pcm_samples={}",
        packet.len(),
        decoded.len()
    );
    ExitCode::SUCCESS
}
