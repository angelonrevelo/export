//! Write a WAV, the PCM it was built from, and a PCM-in-MP4 remux.
//! The host script decodes those with ffmpeg and compares bytes.
//! This binary does not call ffmpeg.

use export_media::{
    build_movie, encode_wav, export, Dedup, Job, MovieSpec, Output, TrackSpec, WavSpec,
};
use std::env;
use std::fs;

fn tone(n: usize, rate: u32) -> Vec<i16> {
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let t = (i as f32) * 440.0 * std::f32::consts::TAU / (rate as f32);
        out.push((t.sin() * 8000.0) as i16);
    }
    out
}

fn main() {
    let mut args = env::args().skip(1);
    let wav_path = args.next().expect("wav path");
    let pcm_path = args.next().expect("pcm path");
    let mp4_path = args.next().expect("mp4 path");
    let rate = 16_000u32;
    let samples = tone(rate as usize, rate);
    let mut pcm = Vec::with_capacity(samples.len() * 2);
    for s in &samples {
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    let wav = encode_wav(
        WavSpec {
            channels: 1,
            sample_rate: rate,
            bits: 16,
        },
        &pcm,
    )
    .unwrap();
    let payloads: Vec<Vec<u8>> = pcm.chunks(2).map(|c| c.to_vec()).collect();
    let n = payloads.len();
    let built = build_movie(&MovieSpec {
        brand: *b"M4A ",
        timescale: rate,
        faststart: false,
        interleave: false,
        tracks: vec![TrackSpec {
            handler: *b"soun",
            codec: *b"sowt",
            timescale: rate,
            width: 0,
            height: 0,
            channels: 1,
            bits: 16,
            sample_rate: rate,
            deltas: vec![1; n],
            payloads,
        }],
    })
    .unwrap();
    let mut dedup = Dedup::new();
    let remuxed = export(
        &built,
        Job::Remux {
            faststart: true,
            audio_only: true,
            video_only: false,
        },
        &mut dedup,
    )
    .unwrap();
    let Output::Fresh { data, .. } = remuxed else {
        panic!("expected a fresh remux");
    };
    fs::write(&wav_path, wav).unwrap();
    fs::write(&pcm_path, &pcm).unwrap();
    fs::write(&mp4_path, data).unwrap();
}
