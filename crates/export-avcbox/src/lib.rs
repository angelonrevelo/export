//! Box one Annex B H.264 I-frame into a non-fragmented MP4.
//!
//! The file is `ftyp`, `moov`, and `mdat`, with one `avc1` sample. Threading
//! is off. This is one boxed frame, not a shrink of a real video. Nothing here
//! encodes H.264, and nothing here calls ffmpeg.

#![forbid(unsafe_code)]

mod annexb;
mod bits;
mod error;
mod fixture;
mod mp4;
mod param;
mod walk;

pub use error::Error;
pub use fixture::{IFRAME_160X160, IFRAME_32X32};

/// Encode settings for the boxer. Threading is off: `threads` must be 1.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub threads: u8,
}

impl Default for Options {
    fn default() -> Self {
        Self { threads: 1 }
    }
}

/// One non-fragmented MP4 holding a single `avc1` sample.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Boxed {
    pub mp4: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Wrap one Annex B I-frame. A truncated buffer returns [`Error::Truncated`].
pub fn box_iframe(annex_b: &[u8], opts: &Options) -> Result<Boxed, Error> {
    if opts.threads != 1 {
        return Err(Error::Threading);
    }
    let access = annexb::split(annex_b)?;
    let sps = param::sps(access.sps)?;
    let pps = param::pps(access.pps)?;
    if pps.sps_id != sps.sps_id {
        return Err(Error::Frame);
    }
    walk::idr(&sps, &pps, access.idr)?;
    let mp4 = mp4::write(&sps, access.sps, access.pps, access.idr, &access.sei)?;
    Ok(Boxed {
        mp4,
        width: sps.width,
        height: sps.height,
    })
}
