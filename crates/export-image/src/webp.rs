//! RIFF/WEBP container for a single still image.
//!
//! Accepted chunks are `VP8L` (lossless) and `VP8 ` (lossy keyframe). `VP8X`,
//! `ALPH`, `ANIM`, and `ANMF` are errors: this crate does not round-trip
//! extended, alpha-only, or animated WebP.

use crate::error::{Error, INVALID, TRUNCATED, UNSUPPORTED};
use crate::raster::Raster;
use crate::vp8::{decode_vp8, encode_vp8};
use crate::vp8l::{decode_vp8l, encode_vp8l};

/// Decode a simple WebP still. The sniffer's `RIFF`/`WEBP` check is repeated
/// here so a short or corrupt buffer fails before a chunk is read.
pub fn decode_webp(bytes: &[u8]) -> Result<Raster, Error> {
    if bytes.len() < 4 || &bytes[..4] != b"RIFF" {
        return Err(INVALID);
    }
    if bytes.len() < 12 {
        return Err(TRUNCATED);
    }
    if &bytes[8..12] != b"WEBP" {
        return Err(INVALID);
    }
    let riff = u32::from_le_bytes(bytes[4..8].try_into().unwrap()) as usize;
    if bytes.len() < 8 + riff {
        return Err(TRUNCATED);
    }
    let end = 8 + riff;
    if end < 20 {
        return Err(TRUNCATED);
    }
    let tag = &bytes[12..16];
    let size = u32::from_le_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let payload_end = 20 + size;
    if payload_end > end {
        return Err(TRUNCATED);
    }
    let payload = &bytes[20..payload_end];
    match tag {
        b"VP8L" => decode_vp8l(payload),
        b"VP8 " => decode_vp8(payload),
        b"VP8X" | b"ALPH" | b"ANIM" | b"ANMF" => Err(UNSUPPORTED),
        _ => Err(UNSUPPORTED),
    }
}

/// Lossless WebP. Pixels come back unchanged for RGB and RGBA.
pub fn encode_webp_lossless(raster: &Raster) -> Result<Vec<u8>, Error> {
    Ok(wrap(b"VP8L", &encode_vp8l(raster)?))
}

/// Lossy WebP. See [`crate::vp8`] for the keyframe subset and quantizer.
pub fn encode_webp_lossy(raster: &Raster) -> Result<Vec<u8>, Error> {
    Ok(wrap(b"VP8 ", &encode_vp8(raster)?))
}

fn wrap(tag: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(8 + payload.len() + 1);
    body.extend_from_slice(tag);
    body.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    body.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        body.push(0);
    }
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(4 + body.len() as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{INVALID, TRUNCATED, UNSUPPORTED};

    fn solid(rgb: [u8; 3]) -> Raster {
        let mut pixels = Vec::new();
        for _ in 0..16 * 16 {
            pixels.extend_from_slice(&rgb);
        }
        Raster::new(16, 16, 3, pixels).unwrap()
    }

    #[test]
    fn lossless_bytes_round_trip() {
        let mut pixels = Vec::new();
        for y in 0..8u8 {
            for x in 0..8u8 {
                pixels.extend_from_slice(&[x.wrapping_mul(30), y.wrapping_mul(20), 90]);
            }
        }
        let src = Raster::new(8, 8, 3, pixels.clone()).unwrap();
        let file = encode_webp_lossless(&src).unwrap();
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(&file[8..12], b"WEBP");
        assert_eq!(&file[12..16], b"VP8L");
        let back = decode_webp(&file).unwrap();
        assert_eq!(back.pixels, pixels);

        let mut rgba = Vec::new();
        for i in 0..4u8 {
            rgba.extend_from_slice(&[i, 20, 40, 10 + i * 40]);
        }
        let src = Raster::new(2, 2, 4, rgba.clone()).unwrap();
        let back = decode_webp(&encode_webp_lossless(&src).unwrap()).unwrap();
        assert_eq!(back.channels, 4);
        assert_eq!(back.pixels, rgba);
    }

    #[test]
    fn lossy_file_decodes_to_the_same_raster_twice() {
        let src = solid([180, 70, 40]);
        let file = encode_webp_lossy(&src).unwrap();
        assert_eq!(&file[12..16], b"VP8 ");
        let a = decode_webp(&file).unwrap();
        let b = decode_webp(&file).unwrap();
        assert_eq!(a.pixels, b.pixels);
        assert_eq!(a.width, 16);
    }

    #[test]
    fn bad_signature_and_truncated_and_extended_fail() {
        assert_eq!(decode_webp(b"nope").unwrap_err(), INVALID);
        assert_eq!(decode_webp(b"RIFF").unwrap_err(), TRUNCATED);
        assert_eq!(decode_webp(b"RIFF\x04\x00\x00\x00JPEG").unwrap_err(), INVALID);
        let mut short = encode_webp_lossless(&solid([1, 2, 3])).unwrap();
        short.truncate(18);
        assert_eq!(decode_webp(&short).unwrap_err(), TRUNCATED);
        let chopped = {
            let file = encode_webp_lossy(&solid([9, 9, 9])).unwrap();
            file[..file.len() / 2].to_vec()
        };
        assert_eq!(decode_webp(&chopped).unwrap_err(), TRUNCATED);
        assert_eq!(decode_webp(&wrap(b"VP8X", &[0; 10])).unwrap_err(), UNSUPPORTED);
        assert_eq!(decode_webp(&wrap(b"ALPH", &[0; 8])).unwrap_err(), UNSUPPORTED);
        assert_eq!(decode_webp(&wrap(b"ANIM", &[0; 6])).unwrap_err(), UNSUPPORTED);
        assert_eq!(decode_webp(&wrap(b"ANMF", &[0; 16])).unwrap_err(), UNSUPPORTED);
        assert_eq!(decode_webp(&wrap(b"VP8L", &[0x00, 0x11])).unwrap_err(), INVALID);
        assert_eq!(decode_webp(&wrap(b"VP8 ", &[0, 0, 0])).unwrap_err(), TRUNCATED);
    }
}
