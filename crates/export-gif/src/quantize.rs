//! Uniform 3-3-2 palette for one 24-bit frame.
//!
//! Eight red levels, eight green levels, and four blue levels. The levels are
//! `i * 255 / (n - 1)`. Each source channel snaps to the nearest level, so the
//! absolute error on every byte is bounded. Indexed LZW round-trip is a
//! different path: it does not quantize RGB.

use crate::error::{Error, INVALID, TOO_LARGE};
use crate::gif::{decode_gif, encode_gif, Frame, Gif, Version};
use crate::MAX_PIXELS;

/// Proven maximum absolute error per channel for [`quantize_gif89a`].
/// Red, green, then blue. Checked on every byte 0..=255.
pub const QUANTIZE_MAX_ABS: [u8; 3] = [18, 18, 42];

const R_LEVELS: [u8; 8] = [0, 36, 72, 109, 145, 182, 218, 255];
const B_LEVELS: [u8; 4] = [0, 85, 170, 255];

const R_MAP: [u8; 256] = nearest_map(&R_LEVELS);
const B_MAP: [u8; 256] = nearest_map_b(&B_LEVELS);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbRaster {
    pub width: u16,
    pub height: u16,
    /// Packed RGB, length `width * height * 3`.
    pub rgb: Vec<u8>,
}

#[derive(Clone, Debug)]
pub struct QuantizeResult {
    /// GIF89a bytes of the quantized frame.
    pub bytes: Vec<u8>,
    /// Mean absolute error per channel after encode and decode, in 8-bit units.
    pub mae: [f64; 3],
    /// Maximum absolute error per channel on this image.
    pub max_abs: [u8; 3],
}

/// Quantize `raster`, write GIF89a, decode it, and score the pixels.
///
/// The returned error is the file round-trip, not the in-memory indices.
/// [`video_to_gif`](crate::video_to_gif) is not used.
pub fn quantize_gif89a(raster: &RgbRaster) -> Result<QuantizeResult, Error> {
    let n = pixel_len(raster)?;
    let gif = quantize_image(raster, n)?;
    let bytes = encode_gif(&gif)?;
    let decoded = decode_gif(&bytes)?;
    let back = expand_frame(&decoded)?;
    if back.len() != raster.rgb.len() {
        return Err(INVALID);
    }
    let (mae, max_abs) = channel_error(&raster.rgb, &back)?;
    if max_abs[0] > QUANTIZE_MAX_ABS[0]
        || max_abs[1] > QUANTIZE_MAX_ABS[1]
        || max_abs[2] > QUANTIZE_MAX_ABS[2]
    {
        return Err(INVALID);
    }
    Ok(QuantizeResult {
        bytes,
        mae,
        max_abs,
    })
}

fn pixel_len(raster: &RgbRaster) -> Result<usize, Error> {
    if raster.width == 0 || raster.height == 0 {
        return Err(INVALID);
    }
    let pixels = u32::from(raster.width)
        .checked_mul(u32::from(raster.height))
        .ok_or(TOO_LARGE)?;
    if u64::from(pixels) > MAX_PIXELS {
        return Err(TOO_LARGE);
    }
    let bytes = (pixels as usize).checked_mul(3).ok_or(TOO_LARGE)?;
    if raster.rgb.len() != bytes {
        return Err(INVALID);
    }
    Ok(pixels as usize)
}

fn quantize_image(raster: &RgbRaster, pixels: usize) -> Result<Gif, Error> {
    let mut indices = Vec::with_capacity(pixels);
    let rgb = &raster.rgb;
    for i in 0..pixels {
        let o = i * 3;
        let ri = R_MAP[rgb[o] as usize];
        let gi = R_MAP[rgb[o + 1] as usize];
        let bi = B_MAP[rgb[o + 2] as usize];
        indices.push((ri << 5) | (gi << 2) | bi);
    }
    Ok(Gif {
        version: Version::Gif89a,
        width: raster.width,
        height: raster.height,
        background: 0,
        aspect: 0,
        global_table: Some(palette()),
        loop_count: None,
        frames: vec![Frame {
            left: 0,
            top: 0,
            width: raster.width,
            height: raster.height,
            delay_cs: 0,
            disposal: 0,
            transparent: None,
            local_table: None,
            indices,
        }],
    })
}

fn palette() -> Vec<[u8; 3]> {
    let mut table = Vec::with_capacity(256);
    for i in 0..256u16 {
        let ri = (i >> 5) as usize;
        let gi = ((i >> 2) & 7) as usize;
        let bi = (i & 3) as usize;
        table.push([R_LEVELS[ri], R_LEVELS[gi], B_LEVELS[bi]]);
    }
    table
}

fn expand_frame(gif: &Gif) -> Result<Vec<u8>, Error> {
    if gif.frames.len() != 1 {
        return Err(INVALID);
    }
    let frame = &gif.frames[0];
    if frame.left != 0 || frame.top != 0 || frame.width != gif.width || frame.height != gif.height {
        return Err(INVALID);
    }
    let table = frame
        .local_table
        .as_ref()
        .or(gif.global_table.as_ref())
        .ok_or(INVALID)?;
    if frame.indices.len() != usize::from(frame.width) * usize::from(frame.height) {
        return Err(INVALID);
    }
    let mut rgb = Vec::with_capacity(frame.indices.len() * 3);
    for &index in &frame.indices {
        let color = table.get(usize::from(index)).ok_or(INVALID)?;
        rgb.extend_from_slice(color);
    }
    Ok(rgb)
}

fn channel_error(src: &[u8], dst: &[u8]) -> Result<([f64; 3], [u8; 3]), Error> {
    if src.len() != dst.len() || src.len() % 3 != 0 || src.is_empty() {
        return Err(INVALID);
    }
    let n = src.len() / 3;
    let mut sum = [0u64; 3];
    let mut max_abs = [0u8; 3];
    for i in 0..n {
        for c in 0..3 {
            let d = src[i * 3 + c].abs_diff(dst[i * 3 + c]);
            sum[c] += u64::from(d);
            if d > max_abs[c] {
                max_abs[c] = d;
            }
        }
    }
    let mae = [
        sum[0] as f64 / n as f64,
        sum[1] as f64 / n as f64,
        sum[2] as f64 / n as f64,
    ];
    Ok((mae, max_abs))
}

const fn nearest_map(levels: &[u8; 8]) -> [u8; 256] {
    let mut map = [0u8; 256];
    let mut v = 0;
    while v < 256 {
        map[v] = nearest(levels, v as u8);
        v += 1;
    }
    map
}

const fn nearest_map_b(levels: &[u8; 4]) -> [u8; 256] {
    let mut map = [0u8; 256];
    let mut v = 0;
    while v < 256 {
        map[v] = nearest4(levels, v as u8);
        v += 1;
    }
    map
}

const fn nearest(levels: &[u8; 8], v: u8) -> u8 {
    let mut best = 0u8;
    let mut best_d = 256u16;
    let mut i = 0;
    while i < 8 {
        let level = levels[i];
        let d = if level > v {
            (level - v) as u16
        } else {
            (v - level) as u16
        };
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
        i += 1;
    }
    best
}

const fn nearest4(levels: &[u8; 4], v: u8) -> u8 {
    let mut best = 0u8;
    let mut best_d = 256u16;
    let mut i = 0;
    while i < 4 {
        let level = levels[i];
        let d = if level > v {
            (level - v) as u16
        } else {
            (v - level) as u16
        };
        if d < best_d {
            best_d = d;
            best = i as u8;
        }
        i += 1;
    }
    best
}

/// 96×64 generated RGB. Not a photograph.
pub fn quantize_fixture() -> RgbRaster {
    let width = 96u16;
    let height = 64u16;
    let mut rgb = Vec::with_capacity(usize::from(width) * usize::from(height) * 3);
    for y in 0..height {
        for x in 0..width {
            let i = u32::from(y) * u32::from(width) + u32::from(x);
            rgb.push((i.wrapping_mul(17)) as u8);
            rgb.push((i.wrapping_mul(29)) as u8);
            rgb.push((i.wrapping_mul(53)) as u8);
        }
    }
    RgbRaster { width, height, rgb }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decode_gif;
    use crate::error::ErrorCode;

    #[test]
    fn channel_bound_holds_for_every_byte() {
        let mut hit = [false; 3];
        for v in 0..=255u8 {
            let rd = v.abs_diff(R_LEVELS[R_MAP[v as usize] as usize]);
            let bd = v.abs_diff(B_LEVELS[B_MAP[v as usize] as usize]);
            assert!(rd <= QUANTIZE_MAX_ABS[0]);
            assert!(bd <= QUANTIZE_MAX_ABS[2]);
            if rd == QUANTIZE_MAX_ABS[0] {
                hit[0] = true;
            }
            if bd == QUANTIZE_MAX_ABS[2] {
                hit[2] = true;
            }
        }
        assert!(hit[0] && hit[2]);
    }

    #[test]
    fn fixture_round_trip_is_inside_the_bound() {
        let raster = quantize_fixture();
        let result = quantize_gif89a(&raster).unwrap();
        assert!(result.bytes.starts_with(b"GIF89a"));
        assert!(result.mae[0] <= f64::from(QUANTIZE_MAX_ABS[0]));
        assert!(result.mae[1] <= f64::from(QUANTIZE_MAX_ABS[1]));
        assert!(result.mae[2] <= f64::from(QUANTIZE_MAX_ABS[2]));
        let err = decode_gif(&result.bytes[..result.bytes.len() / 2]).unwrap_err();
        assert_eq!(err.code, ErrorCode::Truncated);
    }
}
