//! Box Annex B H.264 pictures into a non-fragmented MP4.
//!
//! One I-frame is one `avc1` sample. P-frames that follow that I-frame are
//! further samples in the same `ftyp` / `moov` / `mdat` file. Threading is
//! off. Nothing here encodes H.264, and nothing here calls ffmpeg.

#![forbid(unsafe_code)]

mod annexb;
mod bits;
mod error;
mod fixture;
mod mp4;
mod param;
mod walk;

pub use error::Error;
pub use fixture::{
    EIGHT_1280X720, EIGHT_320X240, FOUR_1280X720, FOUR_320X240, IFRAME_1280X720, IFRAME_160X160,
    IFRAME_320X240, IFRAME_32X32, PAIR_1280X720, PAIR_320X240, SEQUENCE_160X160,
};

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
    /// Pictures written as `avc1` samples. This is not an ffmpeg frame count.
    pub samples: u32,
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
    for nal in &access.predicted {
        walk::predicted(&sps, &pps, nal)?;
    }
    let samples = 1 + access.predicted.len();
    let mp4 = if access.predicted.is_empty() {
        mp4::write(&sps, access.sps, access.pps, access.idr, &access.sei)?
    } else {
        let mut pictures = Vec::with_capacity(samples);
        pictures.push(access.idr);
        pictures.extend(access.predicted.iter().copied());
        mp4::write_pictures(&sps, access.sps, access.pps, &access.sei, &pictures)?
    };
    Ok(Boxed {
        mp4,
        width: sps.width,
        height: sps.height,
        samples: samples as u32,
    })
}
