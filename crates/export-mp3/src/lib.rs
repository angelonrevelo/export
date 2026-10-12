//! One MPEG-1 Layer III frame, mono, 44.1 kHz.
//!
//! [`encode_frame`] writes a single frame. [`decode_frame`] reads that frame
//! back to 1152 PCM samples. This is not a waveform encoder and not a song
//! encoder: a half-scale 440 Hz sine does not decode back to that sine.
//! There is no bit reservoir. The library does not call ffmpeg.

#![forbid(unsafe_code)]

mod decoder;
mod encoder;

pub use decoder::{decode_frame, mpeg1_layer3_frame_bytes, Error};
pub use encoder::{
    encode_frame, half_scale_440hz, BITRATE_BPS, FRAME_BYTES, FRAME_SAMPLES, SAMPLE_RATE_HZ,
};
