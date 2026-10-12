//! AAC-LC ADTS frames at 44.1 kHz, 1024 samples per channel.
//!
//! This is not a song encoder. [`encode_frame`] starts from a zero history and
//! does not keep overlap. Later frames of the same sine each overlap-add one
//! frame late. A flush of silence after the fourth frame emits the sine that
//! frame still delays. One stereo frame is a channel pair: the half-scale
//! 440 Hz sine on the left and silence on the right. The next stereo frame
//! continues that pair and is still one frame late.
//! HE-AAC, SBR, and parametric stereo return [`Error::Unsupported`].

#![forbid(unsafe_code)]

mod bits;
mod codec;
mod huff;
mod mdct;

pub use codec::{
    decode_flush_frame, decode_fourth_frame, decode_frame, decode_frame_with_tail,
    decode_second_frame, decode_second_stereo_frame, decode_stereo_frame, decode_third_frame,
    encode_flush_frame, encode_fourth_frame, encode_frame, encode_frame_following,
    encode_second_frame, encode_second_stereo_frame, encode_stereo_frame,
    encode_stereo_frame_following, encode_third_frame, overlap_tail_after_third, Error,
    OverlapTail, StereoPcm,
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

/// Recorded maximum absolute PCM error of [`decode_third_frame`] against
/// [`second_frame_pcm`]. Full scale is 1.0.
///
/// Measured max was 0.0008997023. The third frame is still one frame late:
/// overlap-add emits the second frame's sine, not [`third_frame_pcm`]. The
/// bound is that measurement, padded only enough to stay stable.
pub const RECORDED_THIRD_FRAME_MAX_ABS_ERROR: f32 = 0.001;

/// Recorded maximum absolute PCM error of [`decode_fourth_frame`] against
/// [`third_frame_pcm`]. Full scale is 1.0.
///
/// Measured max was 0.0012165308. The fourth frame is still one frame late:
/// overlap-add emits the third frame's sine, not [`fourth_frame_pcm`]. The
/// bound is that measurement, padded only enough to stay stable.
pub const RECORDED_FOURTH_FRAME_MAX_ABS_ERROR: f32 = 0.0014;

/// Recorded maximum absolute PCM error of [`decode_flush_frame`] against
/// [`fourth_frame_pcm`]. Full scale is 1.0.
///
/// Measured max was 0.0025660098. The flush frame's own samples are silence.
/// Overlap-add against the fourth frame emits the sine that frame delays.
/// The bound is that measurement, padded only enough to stay stable.
pub const RECORDED_FLUSH_MAX_ABS_ERROR: f32 = 0.0027;

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

/// Samples 2048..3072 of the same half-scale 440 Hz sine.
///
/// This is the right half of [`encode_third_frame`]'s long window. Overlap-add
/// of that frame emits [`second_frame_pcm`], not these samples.
pub fn third_frame_pcm() -> [f32; FRAME_SAMPLES] {
    sine_frame(FRAME_SAMPLES * 2)
}

/// Samples 3072..4096 of the same half-scale 440 Hz sine.
///
/// This is the right half of [`encode_fourth_frame`]'s long window. Overlap-add
/// of that frame emits [`third_frame_pcm`], not these samples.
pub fn fourth_frame_pcm() -> [f32; FRAME_SAMPLES] {
    sine_frame(FRAME_SAMPLES * 3)
}

fn sine_frame(offset: usize) -> [f32; FRAME_SAMPLES] {
    let mut pcm = [0.0; FRAME_SAMPLES];
    for (i, sample) in pcm.iter_mut().enumerate() {
        let t = (offset + i) as f32 / SAMPLE_RATE as f32;
        *sample = (2.0 * std::f32::consts::PI * 440.0 * t).sin() * 0.5;
    }
    pcm
}
