use crate::error::{Error, INVALID, TOO_LARGE};
use crate::MAX_PIXELS;

/// Tightly packed 8-bit raster, row-major, no padding.
///
/// `channels` is 1 (gray), 2 (gray+alpha), 3 (RGB), or 4 (RGBA). Samples are
/// straight (not premultiplied). This is the only pixel layout the hot path
/// allocates: one `Vec<u8>` of `width * height * channels`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Raster {
    pub width: u32,
    pub height: u32,
    pub channels: u8,
    pub pixels: Vec<u8>,
}

impl Raster {
    pub fn new(width: u32, height: u32, channels: u8, pixels: Vec<u8>) -> Result<Self, Error> {
        if !(1..=4).contains(&channels) || width == 0 || height == 0 {
            return Err(INVALID);
        }
        let n = (width as u64)
            .checked_mul(height as u64)
            .ok_or(TOO_LARGE)?;
        if n > MAX_PIXELS {
            return Err(TOO_LARGE);
        }
        let bytes = n
            .checked_mul(channels as u64)
            .ok_or(TOO_LARGE)?;
        if bytes != pixels.len() as u64 {
            return Err(INVALID);
        }
        Ok(Self {
            width,
            height,
            channels,
            pixels,
        })
    }

    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }

    pub fn sample(&self, x: u32, y: u32, channel: u8) -> u8 {
        let i = ((y as usize * self.width as usize) + x as usize) * self.channels as usize
            + channel as usize;
        self.pixels[i]
    }
}
