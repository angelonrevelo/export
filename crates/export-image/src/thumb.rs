//! Thumbnail: long edge at most 220 pixels, baseline JPEG quality 72.
//!
//! This is the preview path. Quality 72 is the setting for this path, measured
//! on its own. It is not the shrink search. Images that already fit in the
//! 220 box are not enlarged. Inputs longer than [`THUMB_READ_LIMIT`] are
//! rejected before decode.

use crate::error::{Error, TOO_LARGE, UNSUPPORTED};
use crate::jpeg::{decode_jpeg, encode_jpeg};
use crate::png::decode_png;
use crate::resize::{resize, Filter};
use crate::shrink::into_rgb;
use crate::sniff::{sniff, Format};

/// Longest side of a thumbnail, in pixels.
pub const THUMB_LONG_EDGE: u32 = 220;

/// JPEG quality used only by [`thumbnail_jpeg`].
pub const THUMB_QUALITY: u8 = 72;

/// Bytes accepted on the thumbnail path. Larger buffers are not decoded.
pub const THUMB_READ_LIMIT: usize = 50 * 1024 * 1024;

#[derive(Debug)]
pub struct Thumbnail {
    pub jpeg: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Decode a PNG or baseline JPEG, fit the long edge, and encode JPEG q=72.
pub fn thumbnail_jpeg(bytes: &[u8]) -> Result<Thumbnail, Error> {
    if bytes.len() > THUMB_READ_LIMIT {
        return Err(TOO_LARGE);
    }
    let decoded = match sniff(bytes) {
        Format::Png => decode_png(bytes)?,
        Format::Jpeg => decode_jpeg(bytes)?,
        _ => return Err(UNSUPPORTED),
    };
    let rgb = into_rgb(decoded)?;
    let (width, height) = fit_long_edge(rgb.width, rgb.height, THUMB_LONG_EDGE);
    let image = if width == rgb.width && height == rgb.height {
        rgb
    } else {
        resize(&rgb, width, height, Filter::Area)?
    };
    let jpeg = encode_jpeg(&image, THUMB_QUALITY)?;
    Ok(Thumbnail {
        jpeg,
        width,
        height,
    })
}

/// Scale so `max(width, height)` is `edge`, unless both sides are already
/// inside the box. The short side is rounded half up, and at least 1.
pub(crate) fn fit_long_edge(width: u32, height: u32, edge: u32) -> (u32, u32) {
    if width <= edge && height <= edge {
        return (width, height);
    }
    let w = u64::from(width);
    let h = u64::from(height);
    let edge = u64::from(edge);
    if w >= h {
        let nh = ((h * edge + w / 2) / w).max(1);
        (edge as u32, nh as u32)
    } else {
        let nw = ((w * edge + h / 2) / h).max(1);
        (nw as u32, edge as u32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_edge_of_the_640_by_480_fixture_is_220() {
        assert_eq!(fit_long_edge(640, 480, 220), (220, 165));
        assert_eq!(fit_long_edge(480, 640, 220), (165, 220));
        assert_eq!(fit_long_edge(100, 80, 220), (100, 80));
        assert_eq!(fit_long_edge(221, 100, 220), (220, 100));
    }

    #[test]
    fn over_the_read_limit_is_not_decoded() {
        let bytes = vec![0u8; THUMB_READ_LIMIT + 1];
        let err = thumbnail_jpeg(&bytes).unwrap_err();
        assert_eq!(err.code, crate::error::ErrorCode::TooLarge);
    }
}
