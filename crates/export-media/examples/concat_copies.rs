//! Stream-copy two copies of the one-frame audiogram into one MP4.
//! This binary does not call ffmpeg.

use export_media::{audiogram, concat_mp4, encode_wav, WavSpec};
use std::env;
use std::fs;

fn one_second() -> Vec<u8> {
    let rate = 48_000usize;
    let mut pcm = Vec::with_capacity(rate * 2);
    for i in 0..rate {
        let s = ((i as i16).wrapping_mul(17)).wrapping_add((i as i16) >> 3);
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    encode_wav(
        WavSpec {
            channels: 1,
            sample_rate: rate as u32,
            bits: 16,
        },
        &pcm,
    )
    .expect("wav")
}

fn main() {
    let path = env::args().nth(1).expect("mp4 path");
    let one = audiogram(&one_second()).expect("audiogram");
    let out = concat_mp4(&one.data, &one.data).expect("concat");
    fs::write(&path, &out).expect("write");
}
