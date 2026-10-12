//! Decode one generated 8×8, 16×16, 32×32, 64×64, 128×128, 256×256, 512×512, 1024×1024, or 2048×2048 VP9 keyframe.
//!
//! This is not a video decoder. Inter frames, extra frames, and VP8 are unsupported.

#![forbid(unsafe_code)]

mod bits;
mod bool_coder;
mod decode;
mod ivf;
mod probs;

use std::fmt;

pub use decode::decode_keyframe;
pub use ivf::decode_ivf;

/// Failure while reading one VP9 keyframe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The buffer ended before the header or the frame finished.
    Truncated,
    /// The marker, sync code, or padding is not a VP9 frame.
    BadHeader,
    /// The frame is VP8, inter, multi-frame, or outside the supported keyframe.
    Unsupported,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Truncated => "truncated VP9 frame",
            Error::BadHeader => "bad VP9 header",
            Error::Unsupported => "unsupported VP9 frame",
        })
    }
}

impl std::error::Error for Error {}

/// 8-bit 4:2:0 picture from one keyframe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub width: usize,
    pub height: usize,
    pub y: Vec<u8>,
    pub u: Vec<u8>,
    pub v: Vec<u8>,
}
