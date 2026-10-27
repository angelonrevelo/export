//! EXIF orientation for camera JPEGs.
//!
//! Phone photos carry an orientation tag in the EXIF APP1 segment. A viewer
//! shows the picture turned upright, so a shrink that keeps the sensor
//! dimensions sideways does not match what the app produces. This module
//! reads that one tag and turns the pixels the way the tag says.

use crate::error::{Error, INVALID};
use crate::raster::Raster;

/// Read the EXIF orientation tag (1-8) from a JPEG, or 1 when the file
/// carries none. Parsing walks the segment chain; a malformed chain is not
/// an error, it just means no orientation was found.
pub fn orientation(bytes: &[u8]) -> u16 {
    // SOI, then segments: FF marker, two length bytes (big endian, counting
    // themselves), payload. Standalone markers have no length.
    let mut i = 2usize;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xff {
            return 1;
        }
        let marker = bytes[i + 1];
        if marker == 0xd8 || (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        if marker == 0xda || marker == 0xd9 {
            return 1; // start of scan / end of image: no EXIF ahead of it
        }
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if len < 2 || i + 2 + len > bytes.len() {
            return 1;
        }
        if marker == 0xe1 && bytes[i + 4..].starts_with(b"Exif\x00\x00") {
            return read_tiff_orientation(&bytes[i + 4 + 6..i + 2 + len]).unwrap_or(1);
        }
        i += 2 + len;
    }
    1
}

/// Orientation tag 0x0112 from the TIFF IFD0 of an EXIF payload.
fn read_tiff_orientation(tiff: &[u8]) -> Option<u16> {
    if tiff.len() < 8 {
        return None;
    }
    let le = match &tiff[0..2] {
        b"II" => true,
        b"MM" => false,
        _ => return None,
    };
    let u16_at = |p: usize| -> Option<u16> {
        let b = tiff.get(p..p + 2)?;
        Some(if le {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    };
    let u32_at = |p: usize| -> Option<u32> {
        let b = tiff.get(p..p + 4)?;
        Some(if le {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    };
    let ifd0 = u32_at(4)? as usize;
    let count = u16_at(ifd0)? as usize;
    for entry in 0..count {
        let at = ifd0 + 2 + entry * 12;
        if u16_at(at)? == 0x0112 {
            return u16_at(at + 8);
        }
    }
    None
}

/// Turn the pixels the way a viewer applies `orientation`, mirroring
/// `ImageOps.exif_transpose`. Orientation 0 means a file with no tag; it
/// behaves like 1.
pub fn transpose(src: &Raster, orientation: u16) -> Result<Raster, Error> {
    match orientation {
        0 | 1 => Ok(src.clone()),
        2 => flip_horizontal(src),
        3 => rotate_180(src),
        4 => flip_vertical(src),
        5 => transpose_main(src),
        6 => rotate_270(src),
        7 => transpose_anti(src),
        8 => rotate_90(src),
        _ => Err(INVALID),
    }
}

/// Build a raster of `width`×`height` where output pixel `(x, y)` copies the
/// source pixel at the linear offset `pick(x, y)`.
fn rewrite(
    src: &Raster,
    width: u32,
    height: u32,
    pick: impl Fn(usize, usize) -> usize,
) -> Result<Raster, Error> {
    let ch = src.channels as usize;
    let mut pixels = vec![0u8; src.pixels.len()];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let from = pick(x, y);
            let to = (y * width as usize + x) * ch;
            pixels[to..to + ch].copy_from_slice(&src.pixels[from..from + ch]);
        }
    }
    Raster::new(width, height, src.channels, pixels)
}

fn flip_horizontal(src: &Raster) -> Result<Raster, Error> {
    let w = src.width as usize;
    rewrite(src, src.width, src.height, |x, y| {
        (y * w + (w - 1 - x)) * src.channels as usize
    })
}

fn flip_vertical(src: &Raster) -> Result<Raster, Error> {
    let w = src.width as usize;
    let h = src.height as usize;
    rewrite(src, src.width, src.height, |x, y| {
        ((h - 1 - y) * w + x) * src.channels as usize
    })
}

fn rotate_180(src: &Raster) -> Result<Raster, Error> {
    let w = src.width as usize;
    let h = src.height as usize;
    rewrite(src, src.width, src.height, |x, y| {
        ((h - 1 - y) * w + (w - 1 - x)) * src.channels as usize
    })
}

fn rotate_90(src: &Raster) -> Result<Raster, Error> {
    // 90° counterclockwise: the right column becomes the top row.
    let w = src.width as usize;
    rewrite(src, src.height, src.width, |x, y| {
        (x * w + (w - 1 - y)) * src.channels as usize
    })
}

fn rotate_270(src: &Raster) -> Result<Raster, Error> {
    // 90° clockwise: the left column becomes the top row.
    let w = src.width as usize;
    let h = src.height as usize;
    rewrite(src, src.height, src.width, |x, y| {
        ((h - 1 - x) * w + y) * src.channels as usize
    })
}

fn transpose_main(src: &Raster) -> Result<Raster, Error> {
    let w = src.width as usize;
    rewrite(src, src.height, src.width, |x, y| (x * w + y) * src.channels as usize)
}

fn transpose_anti(src: &Raster) -> Result<Raster, Error> {
    let w = src.width as usize;
    let h = src.height as usize;
    rewrite(src, src.height, src.width, |x, y| {
        ((h - 1 - x) * w + (w - 1 - y)) * src.channels as usize
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raster(width: u32, height: u32, pixels: &[u8]) -> Raster {
        Raster::new(width, height, 1, pixels.to_vec()).unwrap()
    }

    fn body(src: &Raster) -> Vec<u8> {
        src.pixels.clone()
    }

    #[test]
    fn three_by_two_letters_survive_every_turn() {
        // A B C
        // D E F
        let src = raster(3, 2, b"ABCDEF");
        assert_eq!(body(&transpose(&src, 1).unwrap()), src.pixels);
        assert_eq!(body(&transpose(&src, 2).unwrap()), b"CBAFED");
        assert_eq!(body(&transpose(&src, 3).unwrap()), b"FEDCBA");
        assert_eq!(body(&transpose(&src, 4).unwrap()), b"DEFABC");
        assert_eq!(body(&transpose(&src, 5).unwrap()), b"ADBECF");        assert_eq!(body(&transpose(&src, 6).unwrap()), b"DAEBFC");
        assert_eq!(body(&transpose(&src, 7).unwrap()), b"FCEBDA");
        assert_eq!(body(&transpose(&src, 8).unwrap()), b"CFBEAD");
    }

    #[test]
    fn transposes_swap_the_dimensions() {
        let src = raster(3, 2, &[1, 2, 3, 4, 5, 6]);
        for orientation in [5u16, 6, 7, 8] {
            let out = transpose(&src, orientation).unwrap();
            assert_eq!((out.width, out.height), (2, 3));
        }
        for orientation in [1u16, 2, 3, 4] {
            let out = transpose(&src, orientation).unwrap();
            assert_eq!((out.width, out.height), (3, 2));
        }
    }

    #[test]
    fn an_orientation_tag_is_read_from_the_exif_segment() {
        // SOI + APP1 with a little-endian TIFF: IFD0 has one entry, tag
        // 0x0112, type SHORT, value 6.
        let mut jpeg = vec![0xff, 0xd8];
        let mut tiff = Vec::new();
        tiff.extend_from_slice(b"II\x2a\x00");
        tiff.extend_from_slice(&8u32.to_le_bytes());
        tiff.extend_from_slice(&1u16.to_le_bytes());
        tiff.extend_from_slice(&0x0112u16.to_le_bytes());
        tiff.extend_from_slice(&3u16.to_le_bytes());
        tiff.extend_from_slice(&1u32.to_le_bytes());
        tiff.extend_from_slice(&6u16.to_le_bytes());
        tiff.extend_from_slice(&0u16.to_le_bytes());
        tiff.extend_from_slice(&0u32.to_le_bytes());
        let payload_len = 6 + tiff.len();
        jpeg.extend_from_slice(&[0xff, 0xe1]);
        jpeg.extend_from_slice(&((payload_len + 2) as u16).to_be_bytes());
        jpeg.extend_from_slice(b"Exif\x00\x00");
        jpeg.extend_from_slice(&tiff);
        assert_eq!(orientation(&jpeg), 6);
        assert_eq!(orientation(&[0xff, 0xd8, 0xff, 0xda]), 1);
    }
}
