//! One MPEG-1 Layer III frame, mono, 44.1 kHz.
//!
//! [`encode_frame`] writes a single frame. [`decode_frame`] reads that frame
//! back to 1152 PCM samples. This crate does not encode a song, does not keep
//! a bit reservoir, and does not call out to ffmpeg.

#![forbid(unsafe_code)]

mod decoder;
mod encoder;

pub use decoder::{decode_frame, mpeg1_layer3_frame_bytes, Error};
pub use encoder::{encode_frame, BITRATE_BPS, FRAME_BYTES, FRAME_SAMPLES, SAMPLE_RATE_HZ};
