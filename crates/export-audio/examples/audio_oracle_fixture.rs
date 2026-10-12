//! Write a WAV, raw s16le, and subset FLAC for the host byte-compare script.
//! This binary does not call ffmpeg.

use export_audio::{encode, Pcm};
use std::env;
use std::fs;
use std::process::ExitCode;

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(wav_path) = args.next() else {
        eprintln!("usage: audio_oracle_fixture <out.wav> <out.pcm> <out.flac>");
        return ExitCode::from(2);
    };
    let Some(pcm_path) = args.next() else {
        eprintln!("usage: audio_oracle_fixture <out.wav> <out.pcm> <out.flac>");
        return ExitCode::from(2);
    };
    let Some(flac_path) = args.next() else {
        eprintln!("usage: audio_oracle_fixture <out.wav> <out.pcm> <out.flac>");
        return ExitCode::from(2);
    };
    let mut samples = Vec::with_capacity(4096);
    for i in 0..4096 {
        let s = ((i % 200) - 100) * 40 + (i * i) % 30;
        samples.push(s.clamp(-32_768, 32_767));
    }
    let pcm = Pcm {
        sample_rate: 44_100,
        channels: 1,
        bits: 16,
        samples,
    };
    let mut raw = Vec::with_capacity(pcm.samples.len() * 2);
    for s in &pcm.samples {
        raw.extend_from_slice(&(*s as i16).to_le_bytes());
    }
    let flac = match encode(&pcm) {
        Ok(b) => b,
        Err(err) => {
            eprintln!("encode: {err}");
            return ExitCode::from(1);
        }
    };
    let wav = match export_audio::export(&flac, export_audio::Job::ToWav, &mut export_audio::Dedup::new())
    {
        Ok(export_audio::Output::Fresh { data, .. }) => data,
        Ok(export_audio::Output::Duplicate { .. }) => {
            eprintln!("duplicate");
            return ExitCode::from(1);
        }
        Err(err) => {
            eprintln!("wav: {err}");
            return ExitCode::from(1);
        }
    };
    if fs::write(&wav_path, wav).is_err() || fs::write(&pcm_path, raw).is_err() || fs::write(&flac_path, flac).is_err()
    {
        eprintln!("write failed");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}
