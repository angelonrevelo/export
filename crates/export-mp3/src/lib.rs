//! One MPEG-1 Layer III frame, mono, 44.1 kHz.
//!
//! [`encode_frame`] writes a single frame. [`decode_frame`] reads that frame
//! back to 1152 PCM samples. A non-silent block stores the first 32 quantized
//! long-block MDCT lines of each granule (indices 0 through 31), not a
//! waveform. There is no bit
//! reservoir. The library does not call ffmpeg.

#![forbid(unsafe_code)]

mod decoder;
mod encoder;
mod window;

pub use decoder::{decode_frame, mpeg1_layer3_frame_bytes, Error};
pub use encoder::{
    encode_frame, half_scale_440hz, BITRATE_BPS, FRAME_BYTES, FRAME_SAMPLES, SAMPLE_RATE_HZ,
};
