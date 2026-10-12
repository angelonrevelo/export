//! RIFF/WEBP container.
//!
//! A still is `VP8L` or `VP8 `. `decode_webp` rejects `VP8X`, `ALPH`, `ANIM`,
//! and `ANMF`. A two-frame or four-frame animation is a `VP8X`/`ANIM`/`ANMF`
//! wrapper around `VP8 ` keyframes from the existing encoder. That wrapper is not an
//! entropy coder. Alpha, ICC, EXIF, XMP, and a frame that does not cover the
//! canvas stay errors.

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

/// Animation flag in the `VP8X` feature byte. Bit 1, value `0x02`.
const ANIMATION: u8 = 0x02;
/// `ANMF` flag bit 1: overwrite the frame rectangle. Bit 0 (disposal) stays 0.
const NO_BLEND: u8 = 0x02;
const FRAME_MS: u32 = 100;

/// Two-frame or four-frame lossy animation. Every frame must be the same
/// size, and each one must be a frame the lossy encoder already accepts: RGB,
/// sides a multiple of 16. Each `ANMF` payload is that keyframe. There is no
/// second entropy coder.
pub fn encode_webp_animation(frames: &[Raster]) -> Result<Vec<u8>, Error> {
    if frames.len() != 2 && frames.len() != 4 {
        return Err(UNSUPPORTED);
    }
    let width = frames[0].width;
    let height = frames[0].height;
    if frames.iter().any(|frame| frame.width != width || frame.height != height) {
        return Err(UNSUPPORTED);
    }
    let mut bitstreams = Vec::with_capacity(frames.len());
    for frame in frames {
        bitstreams.push(encode_vp8(frame)?);
    }
    let mut vp8x = [0u8; 10];
    vp8x[0] = ANIMATION;
    write_u24(&mut vp8x[4..7], width - 1);
    write_u24(&mut vp8x[7..10], height - 1);
    let mut body = Vec::new();
    push_chunk(&mut body, b"VP8X", &vp8x);
    push_chunk(&mut body, b"ANIM", &[0; 6]);
    for bitstream in &bitstreams {
        push_chunk(&mut body, b"ANMF", &anmf(width, height, bitstream));
    }
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + body.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Ok(out)
}

/// Frames of an animation this crate wrote: full-canvas `VP8 ` or `VP8L`
/// bitstreams, in order. [`decode_webp`] still returns [`UNSUPPORTED`] for the
/// same bytes, because that function returns one raster.
pub fn decode_webp_animation(bytes: &[u8]) -> Result<Vec<Raster>, Error> {
    let end = riff_end(bytes)?;
    let vp8x = read_chunk(bytes, 12, end)?;
    if &vp8x.tag != b"VP8X" {
        return Err(UNSUPPORTED);
    }
    if vp8x.payload.len() < 10 {
        return Err(TRUNCATED);
    }
    if vp8x.payload[0] & ANIMATION == 0 || vp8x.payload[0] & !ANIMATION != 0 {
        return Err(UNSUPPORTED);
    }
    let canvas_w = read_u24(&vp8x.payload[4..7]).wrapping_add(1);
    let canvas_h = read_u24(&vp8x.payload[7..10]).wrapping_add(1);
    let anim = read_chunk(bytes, vp8x.next, end)?;
    if &anim.tag != b"ANIM" {
        return Err(UNSUPPORTED);
    }
    if anim.payload.len() < 6 {
        return Err(TRUNCATED);
    }
    let mut at = anim.next;
    let mut frames = Vec::new();
    while at < end {
        let anmf = read_chunk(bytes, at, end)?;
        if &anmf.tag != b"ANMF" {
            return Err(UNSUPPORTED);
        }
        if anmf.payload.len() < 16 {
            return Err(TRUNCATED);
        }
        let origin_x = read_u24(&anmf.payload[0..3]).saturating_mul(2);
        let origin_y = read_u24(&anmf.payload[3..6]).saturating_mul(2);
        let frame_w = read_u24(&anmf.payload[6..9]).wrapping_add(1);
        let frame_h = read_u24(&anmf.payload[9..12]).wrapping_add(1);
        if origin_x != 0 || origin_y != 0 || frame_w != canvas_w || frame_h != canvas_h {
            return Err(UNSUPPORTED);
        }
        let raster = decode_frame(&anmf.payload[16..])?;
        if raster.width != frame_w || raster.height != frame_h {
            return Err(INVALID);
        }
        frames.push(raster);
        at = anmf.next;
    }
    if frames.len() != 2 && frames.len() != 4 {
        return Err(UNSUPPORTED);
    }
    Ok(frames)
}

fn anmf(width: u32, height: u32, bitstream: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(16 + 8 + bitstream.len() + 1);
    write_u24_push(&mut payload, 0);
    write_u24_push(&mut payload, 0);
    write_u24_push(&mut payload, width - 1);
    write_u24_push(&mut payload, height - 1);
    write_u24_push(&mut payload, FRAME_MS);
    payload.push(NO_BLEND);
    push_chunk(&mut payload, b"VP8 ", bitstream);
    payload
}

fn decode_frame(data: &[u8]) -> Result<Raster, Error> {
    let mut at = 0;
    let mut raster = None;
    while at < data.len() {
        let chunk = read_chunk(data, at, data.len())?;
        let decoded = match &chunk.tag {
            b"VP8 " => decode_vp8(chunk.payload)?,
            b"VP8L" => decode_vp8l(chunk.payload)?,
            b"ALPH" => return Err(UNSUPPORTED),
            _ => return Err(UNSUPPORTED),
        };
        if raster.is_some() {
            return Err(INVALID);
        }
        raster = Some(decoded);
        at = chunk.next;
    }
    raster.ok_or(INVALID)
}

struct Chunk<'a> {
    tag: [u8; 4],
    payload: &'a [u8],
    next: usize,
}

fn read_chunk(bytes: &[u8], at: usize, end: usize) -> Result<Chunk<'_>, Error> {
    if at.saturating_add(8) > end || end > bytes.len() {
        return Err(TRUNCATED);
    }
    let mut tag = [0u8; 4];
    tag.copy_from_slice(&bytes[at..at + 4]);
    let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
    let payload_at = at + 8;
    let payload_end = payload_at.saturating_add(size);
    if payload_end > end {
        return Err(TRUNCATED);
    }
    let next = payload_end + (size & 1);
    if next > end {
        return Err(TRUNCATED);
    }
    Ok(Chunk {
        tag,
        payload: &bytes[payload_at..payload_end],
        next,
    })
}

fn riff_end(bytes: &[u8]) -> Result<usize, Error> {
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
    let end = 8usize.saturating_add(riff);
    if bytes.len() < end || end < 20 {
        return Err(TRUNCATED);
    }
    Ok(end)
}

fn write_u24(dst: &mut [u8], value: u32) {
    dst[0] = value as u8;
    dst[1] = (value >> 8) as u8;
    dst[2] = (value >> 16) as u8;
}

fn write_u24_push(dst: &mut Vec<u8>, value: u32) {
    dst.extend_from_slice(&[value as u8, (value >> 8) as u8, (value >> 16) as u8]);
}

fn read_u24(bytes: &[u8]) -> u32 {
    u32::from(bytes[0]) | (u32::from(bytes[1]) << 8) | (u32::from(bytes[2]) << 16)
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], payload: &[u8]) {
    out.extend_from_slice(tag);
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    if payload.len() % 2 == 1 {
        out.push(0);
    }
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

    #[test]
    fn two_frame_animation_returns_both_frames() {
        let first = solid([220, 40, 60]);
        let second = solid([20, 180, 90]);
        assert_eq!(
            encode_webp_animation(&[first.clone()]).unwrap_err(),
            UNSUPPORTED
        );
        let odd = Raster::new(8, 8, 3, vec![0; 8 * 8 * 3]).unwrap();
        assert_eq!(
            encode_webp_animation(&[odd.clone(), odd]).unwrap_err(),
            UNSUPPORTED
        );
        let rgba = Raster::new(16, 16, 4, vec![0; 16 * 16 * 4]).unwrap();
        assert_eq!(
            encode_webp_animation(&[rgba.clone(), rgba]).unwrap_err(),
            UNSUPPORTED
        );
        let file = encode_webp_animation(&[first.clone(), second.clone()]).unwrap();
        assert_eq!(&file[..4], b"RIFF");
        assert_eq!(&file[8..12], b"WEBP");
        assert_eq!(&file[12..16], b"VP8X");
        assert_eq!(file[20], ANIMATION);
        assert_eq!(decode_webp(&file).unwrap_err(), UNSUPPORTED);
        let frames = decode_webp_animation(&file).unwrap();
        assert_eq!(frames.len(), 2);
        let still_first = decode_webp(&encode_webp_lossy(&first).unwrap()).unwrap();
        let still_second = decode_webp(&encode_webp_lossy(&second).unwrap()).unwrap();
        assert_eq!(frames[0].pixels, still_first.pixels);
        assert_eq!(frames[1].pixels, still_second.pixels);
        assert_ne!(frames[0].pixels, frames[1].pixels);
        let mut short = file.clone();
        short.truncate(file.len() / 2);
        assert_eq!(decode_webp_animation(&short).unwrap_err(), TRUNCATED);
    }

    #[test]
    fn four_frame_animation_returns_all_frames() {
        let frames = [
            solid([220, 40, 60]),
            solid([20, 180, 90]),
            solid([10, 10, 200]),
            solid([240, 240, 240]),
        ];
        assert_eq!(
            encode_webp_animation(&frames[..3]).unwrap_err(),
            UNSUPPORTED
        );
        let file = encode_webp_animation(&frames).unwrap();
        assert_eq!(&file[12..16], b"VP8X");
        assert_eq!(decode_webp(&file).unwrap_err(), UNSUPPORTED);
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 4);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
        assert_ne!(back[2].pixels, back[3].pixels);
    }

    #[test]
    fn two_frame_64_animation_returns_both_frames() {
        let mut frames = Vec::new();
        for rgb in [[180u8, 70, 40], [30, 30, 200]] {
            let mut pixels = vec![0u8; 64 * 64 * 3];
            for px in pixels.chunks_exact_mut(3) {
                px.copy_from_slice(&rgb);
            }
            frames.push(Raster::new(64, 64, 3, pixels).unwrap());
        }
        let file = encode_webp_animation(&frames).unwrap();
        assert_eq!(decode_webp(&file).unwrap_err(), UNSUPPORTED);
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].width, 64);
        assert_eq!(back[1].height, 64);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
    }

    #[test]
    fn two_frame_256_animation_returns_both_frames() {
        let mut frames = Vec::new();
        for rgb in [[90u8, 140, 20], [200, 20, 80]] {
            let mut pixels = vec![0u8; 256 * 256 * 3];
            for px in pixels.chunks_exact_mut(3) {
                px.copy_from_slice(&rgb);
            }
            frames.push(Raster::new(256, 256, 3, pixels).unwrap());
        }
        let file = encode_webp_animation(&frames).unwrap();
        assert_eq!(decode_webp(&file).unwrap_err(), UNSUPPORTED);
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].width, 256);
        assert_eq!(back[1].height, 256);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
    }

    #[test]
    fn two_frame_640_by_480_matches_still_decodes() {
        let mut frames = Vec::new();
        for rgb in [[40u8, 90, 160], [210, 50, 30]] {
            let mut pixels = vec![0u8; 640 * 480 * 3];
            for px in pixels.chunks_exact_mut(3) {
                px.copy_from_slice(&rgb);
            }
            frames.push(Raster::new(640, 480, 3, pixels).unwrap());
        }
        let file = encode_webp_animation(&frames).unwrap();
        assert_eq!(decode_webp(&file).unwrap_err(), UNSUPPORTED);
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.width, 640);
            assert_eq!(got.height, 480);
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
    }

    #[test]
    fn four_frame_640_by_480_matches_still_decodes() {
        let mut frames = Vec::new();
        for rgb in [[40u8, 90, 160], [210, 50, 30], [15, 180, 70], [240, 220, 40]] {
            let mut pixels = vec![0u8; 640 * 480 * 3];
            for px in pixels.chunks_exact_mut(3) {
                px.copy_from_slice(&rgb);
            }
            frames.push(Raster::new(640, 480, 3, pixels).unwrap());
        }
        let file = encode_webp_animation(&frames).unwrap();
        assert_eq!(decode_webp(&file).unwrap_err(), UNSUPPORTED);
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 4);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.width, 640);
            assert_eq!(got.height, 480);
            assert_eq!(got.pixels, still.pixels);
        }
    }

    #[test]
    fn two_frame_1280_by_720_matches_still_decodes() {
        let mut frames = Vec::new();
        for rgb in [[40u8, 90, 160], [210, 50, 30]] {
            let mut pixels = vec![0u8; 1280 * 720 * 3];
            for px in pixels.chunks_exact_mut(3) {
                px.copy_from_slice(&rgb);
            }
            frames.push(Raster::new(1280, 720, 3, pixels).unwrap());
        }
        let file = encode_webp_animation(&frames).unwrap();
        assert_eq!(decode_webp(&file).unwrap_err(), UNSUPPORTED);
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.width, 1280);
            assert_eq!(got.height, 720);
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
    }

    #[test]
    fn four_frame_1280_by_720_matches_still_decodes() {
        let mut frames = Vec::new();
        for rgb in [[40u8, 90, 160], [210, 50, 30], [15, 180, 70], [240, 220, 40]] {
            let mut pixels = vec![0u8; 1280 * 720 * 3];
            for px in pixels.chunks_exact_mut(3) {
                px.copy_from_slice(&rgb);
            }
            frames.push(Raster::new(1280, 720, 3, pixels).unwrap());
        }
        let file = encode_webp_animation(&frames).unwrap();
        assert_eq!(decode_webp(&file).unwrap_err(), UNSUPPORTED);
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 4);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.width, 1280);
            assert_eq!(got.height, 720);
            assert_eq!(got.pixels, still.pixels);
        }
    }
}
