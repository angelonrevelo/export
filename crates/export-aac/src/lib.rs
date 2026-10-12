//! Mono AAC-LC ADTS frames at 44.1 kHz, 1024 samples each.
//!
//! This is not a song encoder. [`encode_frame`] starts from a zero history and
//! does not keep overlap. [`encode_second_frame`] is one more frame whose long
//! window starts with [`one_frame_pcm`]. HE-AAC, SBR, and parametric stereo
//! return [`Error::Unsupported`].

#![forbid(unsafe_code)]

mod bits;
mod codec;
mod huff;
mod mdct;

pub use codec::{
    decode_frame, decode_second_frame, encode_frame, encode_frame_following, encode_second_frame,
    Error,
};

/// PCM samples in one AAC long-window frame.
pub const FRAME_SAMPLES: usize = 1024;

/// Sample rate of the one frame this crate implements.
pub const SAMPLE_RATE: u32 = 44_100;

/// Recorded maximum absolute PCM error for [`one_frame_pcm`] after one
/// encode/decode round trip. Full scale is 1.0.
///
/// Measured max was 0.5003551. A lone long window overlap-adds against a
/// zero history, so this frame does not come back as the sine. The bound is
/// that measurement, padded only enough to stay stable. It is not a song
/// encoder's quality target.
pub const RECORDED_MAX_ABS_ERROR: f32 = 0.501;

/// Recorded maximum absolute PCM error of [`decode_second_frame`] against
/// [`one_frame_pcm`]. Full scale is 1.0.
///
/// Measured max was 0.0018240213. The second frame overlap-adds against the
/// first frame's history, so the samples it emits are that sine, one frame
/// late, not [`second_frame_pcm`]. The bound is that measurement, padded only
/// enough to stay stable.
pub const RECORDED_SECOND_FRAME_MAX_ABS_ERROR: f32 = 0.002;

/// Half-scale 440 Hz sine, 1024 samples at 44.1 kHz. This is the one frame
/// the recorded error bound refers to.
pub fn one_frame_pcm() -> [f32; FRAME_SAMPLES] {
    sine_frame(0)
}

/// The next 1024 samples of the same half-scale 440 Hz sine.
///
/// This is the right half of [`encode_second_frame`]'s long window. Overlap-add
/// of that frame emits [`one_frame_pcm`], not these samples.
pub fn second_frame_pcm() -> [f32; FRAME_SAMPLES] {
    sine_frame(FRAME_SAMPLES)
}

fn sine_frame(offset: usize) -> [f32; FRAME_SAMPLES] {
    let mut pcm = [0.0; FRAME_SAMPLES];
    for (i, sample) in pcm.iter_mut().enumerate() {
        let t = (offset + i) as f32 / SAMPLE_RATE as f32;
        *sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5;
    }
    pcm
}
