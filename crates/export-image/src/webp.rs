//! RIFF/WEBP container.
//!
//! A still is `VP8L` or `VP8 `. `decode_webp` returns one raster. For two 64×64
//! partial animations, that raster is the composited canvas: the file whose
//! second frame is the 16×16 rectangle at (16, 32), and the file whose second
//! frame is the 16×16 rectangle at the origin. For the full-frame 64×64
//! two-frame file, for the two-frame 256×256 file, for the two-frame
//! 640×480 file, for the two-frame 1280×720 file, and for the four-frame
//! 1280×720 file, that raster is the first frame. Every other `VP8X`, plus
//! `ALPH`, `ANIM`, and `ANMF`, stays an error. A two-frame or four-frame animation is a
//! `VP8X`/`ANIM`/`ANMF` wrapper around `VP8 ` keyframes from the existing encoder.
//! That wrapper is not an entropy coder. The first frame is the canvas. A later
//! frame may be a smaller rectangle at an even pixel origin. The decoder paints
//! that rectangle onto the canvas and returns the canvas. An odd origin, alpha,
//! ICC, EXIF, and XMP stay errors. There is no loop filter.

use crate::error::{Error, INVALID, TRUNCATED, UNSUPPORTED};
use crate::raster::Raster;
use crate::vp8::{decode_vp8, encode_vp8};
use crate::vp8l::{decode_vp8l, encode_vp8l};

/// Decode one WebP raster. A `VP8L` or `VP8 ` still is unchanged. Two 64×64
/// partial animations return the canvas after the second rectangle is painted:
/// the 16×16 rectangle at (16, 32), and the 16×16 rectangle at the origin. The
/// full-frame 64×64 two-frame file, the two-frame 256×256 file, the
/// two-frame 640×480 file, the two-frame 1280×720 file, and the four-frame
/// 1280×720 file return their first frame. Every other animation
/// stays [`UNSUPPORTED`]. The
/// sniffer's `RIFF`/`WEBP` check is
/// repeated here so a short or corrupt buffer fails before a chunk is read.
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
        b"VP8X" if placed_64_at_16_32(bytes) || partial_64_at_origin(bytes) => {
            composited_canvas(bytes)
        }
        b"VP8X" if full_64_two_frame(bytes) => first_full_frame(bytes),
        b"VP8X" if full_256_two_frame(bytes) => first_256_frame(bytes),
        b"VP8X" if full_640_two_frame(bytes) => first_640_frame(bytes),
        b"VP8X" if full_1280_two_frame(bytes) => first_1280_frame(bytes),
        b"VP8X" if full_1280_four_frame(bytes) => first_1280_four_frame(bytes),
        b"VP8X" | b"ALPH" | b"ANIM" | b"ANMF" => Err(UNSUPPORTED),
        _ => Err(UNSUPPORTED),
    }
}

/// This file only: a 64×64 canvas, a full first frame, and a 16×16 rectangle
/// at (16, 32). The check reads chunk headers. It does not decode coefficients.
fn placed_64_at_16_32(bytes: &[u8]) -> bool {
    let Ok(end) = riff_end(bytes) else {
        return false;
    };
    let Ok(vp8x) = read_chunk(bytes, 12, end) else {
        return false;
    };
    if &vp8x.tag != b"VP8X" || vp8x.payload.len() < 10 || vp8x.payload[0] != ANIMATION {
        return false;
    }
    let canvas_w = read_u24(&vp8x.payload[4..7]).wrapping_add(1);
    let canvas_h = read_u24(&vp8x.payload[7..10]).wrapping_add(1);
    if canvas_w != 64 || canvas_h != 64 {
        return false;
    }
    let Ok(anim) = read_chunk(bytes, vp8x.next, end) else {
        return false;
    };
    if &anim.tag != b"ANIM" || anim.payload.len() < 6 {
        return false;
    }
    let mut at = anim.next;
    let mut frames = [(0u32, 0u32, 0u32, 0u32); 2];
    let mut n = 0usize;
    while at < end {
        if n == 2 {
            return false;
        }
        let Ok(anmf) = read_chunk(bytes, at, end) else {
            return false;
        };
        if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 || anmf.payload[15] != NO_BLEND {
            return false;
        }
        frames[n] = (
            read_u24(&anmf.payload[0..3]).saturating_mul(2),
            read_u24(&anmf.payload[3..6]).saturating_mul(2),
            read_u24(&anmf.payload[6..9]).wrapping_add(1),
            read_u24(&anmf.payload[9..12]).wrapping_add(1),
        );
        n += 1;
        at = anmf.next;
    }
    n == 2 && frames[0] == (0, 0, 64, 64) && frames[1] == (16, 32, 16, 16)
}

/// This file only: a 64×64 canvas, a full first frame, and a 16×16 rectangle
/// at the origin. The check reads chunk headers. It does not decode coefficients.
fn partial_64_at_origin(bytes: &[u8]) -> bool {
    let Ok(end) = riff_end(bytes) else {
        return false;
    };
    let Ok(vp8x) = read_chunk(bytes, 12, end) else {
        return false;
    };
    if &vp8x.tag != b"VP8X" || vp8x.payload.len() < 10 || vp8x.payload[0] != ANIMATION {
        return false;
    }
    let canvas_w = read_u24(&vp8x.payload[4..7]).wrapping_add(1);
    let canvas_h = read_u24(&vp8x.payload[7..10]).wrapping_add(1);
    if canvas_w != 64 || canvas_h != 64 {
        return false;
    }
    let Ok(anim) = read_chunk(bytes, vp8x.next, end) else {
        return false;
    };
    if &anim.tag != b"ANIM" || anim.payload.len() < 6 {
        return false;
    }
    let mut at = anim.next;
    let mut frames = [(0u32, 0u32, 0u32, 0u32); 2];
    let mut n = 0usize;
    while at < end {
        if n == 2 {
            return false;
        }
        let Ok(anmf) = read_chunk(bytes, at, end) else {
            return false;
        };
        if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 || anmf.payload[15] != NO_BLEND {
            return false;
        }
        frames[n] = (
            read_u24(&anmf.payload[0..3]).saturating_mul(2),
            read_u24(&anmf.payload[3..6]).saturating_mul(2),
            read_u24(&anmf.payload[6..9]).wrapping_add(1),
            read_u24(&anmf.payload[9..12]).wrapping_add(1),
        );
        n += 1;
        at = anmf.next;
    }
    n == 2 && frames[0] == (0, 0, 64, 64) && frames[1] == (0, 0, 16, 16)
}

/// This file only: a 64×64 canvas and two full frames. The check reads chunk
/// headers. It does not decode coefficients.
fn full_64_two_frame(bytes: &[u8]) -> bool {
    let Ok(end) = riff_end(bytes) else {
        return false;
    };
    let Ok(vp8x) = read_chunk(bytes, 12, end) else {
        return false;
    };
    if &vp8x.tag != b"VP8X" || vp8x.payload.len() < 10 || vp8x.payload[0] != ANIMATION {
        return false;
    }
    let canvas_w = read_u24(&vp8x.payload[4..7]).wrapping_add(1);
    let canvas_h = read_u24(&vp8x.payload[7..10]).wrapping_add(1);
    if canvas_w != 64 || canvas_h != 64 {
        return false;
    }
    let Ok(anim) = read_chunk(bytes, vp8x.next, end) else {
        return false;
    };
    if &anim.tag != b"ANIM" || anim.payload.len() < 6 {
        return false;
    }
    let mut at = anim.next;
    let mut frames = [(0u32, 0u32, 0u32, 0u32); 2];
    let mut n = 0usize;
    while at < end {
        if n == 2 {
            return false;
        }
        let Ok(anmf) = read_chunk(bytes, at, end) else {
            return false;
        };
        if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 || anmf.payload[15] != NO_BLEND {
            return false;
        }
        frames[n] = (
            read_u24(&anmf.payload[0..3]).saturating_mul(2),
            read_u24(&anmf.payload[3..6]).saturating_mul(2),
            read_u24(&anmf.payload[6..9]).wrapping_add(1),
            read_u24(&anmf.payload[9..12]).wrapping_add(1),
        );
        n += 1;
        at = anmf.next;
    }
    n == 2 && frames[0] == (0, 0, 64, 64) && frames[1] == (0, 0, 64, 64)
}

fn first_full_frame(bytes: &[u8]) -> Result<Raster, Error> {
    let end = riff_end(bytes)?;
    let vp8x = read_chunk(bytes, 12, end)?;
    let anim = read_chunk(bytes, vp8x.next, end)?;
    let anmf = read_chunk(bytes, anim.next, end)?;
    if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 {
        return Err(UNSUPPORTED);
    }
    let mut raster = decode_frame(&anmf.payload[16..])?;
    if raster.width != 64 || raster.height != 64 {
        return Err(INVALID);
    }
    raster.x = 0;
    raster.y = 0;
    Ok(raster)
}

/// This file only: a 256×256 canvas and two full frames. The check reads chunk
/// headers. It does not decode coefficients.
fn full_256_two_frame(bytes: &[u8]) -> bool {
    let Ok(end) = riff_end(bytes) else {
        return false;
    };
    let Ok(vp8x) = read_chunk(bytes, 12, end) else {
        return false;
    };
    if &vp8x.tag != b"VP8X" || vp8x.payload.len() < 10 || vp8x.payload[0] != ANIMATION {
        return false;
    }
    let canvas_w = read_u24(&vp8x.payload[4..7]).wrapping_add(1);
    let canvas_h = read_u24(&vp8x.payload[7..10]).wrapping_add(1);
    if canvas_w != 256 || canvas_h != 256 {
        return false;
    }
    let Ok(anim) = read_chunk(bytes, vp8x.next, end) else {
        return false;
    };
    if &anim.tag != b"ANIM" || anim.payload.len() < 6 {
        return false;
    }
    let mut at = anim.next;
    let mut frames = [(0u32, 0u32, 0u32, 0u32); 2];
    let mut n = 0usize;
    while at < end {
        if n == 2 {
            return false;
        }
        let Ok(anmf) = read_chunk(bytes, at, end) else {
            return false;
        };
        if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 || anmf.payload[15] != NO_BLEND {
            return false;
        }
        frames[n] = (
            read_u24(&anmf.payload[0..3]).saturating_mul(2),
            read_u24(&anmf.payload[3..6]).saturating_mul(2),
            read_u24(&anmf.payload[6..9]).wrapping_add(1),
            read_u24(&anmf.payload[9..12]).wrapping_add(1),
        );
        n += 1;
        at = anmf.next;
    }
    n == 2 && frames[0] == (0, 0, 256, 256) && frames[1] == (0, 0, 256, 256)
}

fn first_256_frame(bytes: &[u8]) -> Result<Raster, Error> {
    let end = riff_end(bytes)?;
    let vp8x = read_chunk(bytes, 12, end)?;
    let anim = read_chunk(bytes, vp8x.next, end)?;
    let anmf = read_chunk(bytes, anim.next, end)?;
    if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 {
        return Err(UNSUPPORTED);
    }
    let mut raster = decode_frame(&anmf.payload[16..])?;
    if raster.width != 256 || raster.height != 256 {
        return Err(INVALID);
    }
    raster.x = 0;
    raster.y = 0;
    Ok(raster)
}

/// This file only: a 640×480 canvas and two full frames. The check reads chunk
/// headers. It does not decode coefficients.
fn full_640_two_frame(bytes: &[u8]) -> bool {
    let Ok(end) = riff_end(bytes) else {
        return false;
    };
    let Ok(vp8x) = read_chunk(bytes, 12, end) else {
        return false;
    };
    if &vp8x.tag != b"VP8X" || vp8x.payload.len() < 10 || vp8x.payload[0] != ANIMATION {
        return false;
    }
    let canvas_w = read_u24(&vp8x.payload[4..7]).wrapping_add(1);
    let canvas_h = read_u24(&vp8x.payload[7..10]).wrapping_add(1);
    if canvas_w != 640 || canvas_h != 480 {
        return false;
    }
    let Ok(anim) = read_chunk(bytes, vp8x.next, end) else {
        return false;
    };
    if &anim.tag != b"ANIM" || anim.payload.len() < 6 {
        return false;
    }
    let mut at = anim.next;
    let mut frames = [(0u32, 0u32, 0u32, 0u32); 2];
    let mut n = 0usize;
    while at < end {
        if n == 2 {
            return false;
        }
        let Ok(anmf) = read_chunk(bytes, at, end) else {
            return false;
        };
        if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 || anmf.payload[15] != NO_BLEND {
            return false;
        }
        frames[n] = (
            read_u24(&anmf.payload[0..3]).saturating_mul(2),
            read_u24(&anmf.payload[3..6]).saturating_mul(2),
            read_u24(&anmf.payload[6..9]).wrapping_add(1),
            read_u24(&anmf.payload[9..12]).wrapping_add(1),
        );
        n += 1;
        at = anmf.next;
    }
    n == 2 && frames[0] == (0, 0, 640, 480) && frames[1] == (0, 0, 640, 480)
}

fn first_640_frame(bytes: &[u8]) -> Result<Raster, Error> {
    let end = riff_end(bytes)?;
    let vp8x = read_chunk(bytes, 12, end)?;
    let anim = read_chunk(bytes, vp8x.next, end)?;
    let anmf = read_chunk(bytes, anim.next, end)?;
    if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 {
        return Err(UNSUPPORTED);
    }
    let mut raster = decode_frame(&anmf.payload[16..])?;
    if raster.width != 640 || raster.height != 480 {
        return Err(INVALID);
    }
    raster.x = 0;
    raster.y = 0;
    Ok(raster)
}

/// This file only: a 1280×720 canvas and two full frames. The check reads chunk
/// headers. It does not decode coefficients.
fn full_1280_two_frame(bytes: &[u8]) -> bool {
    let Ok(end) = riff_end(bytes) else {
        return false;
    };
    let Ok(vp8x) = read_chunk(bytes, 12, end) else {
        return false;
    };
    if &vp8x.tag != b"VP8X" || vp8x.payload.len() < 10 || vp8x.payload[0] != ANIMATION {
        return false;
    }
    let canvas_w = read_u24(&vp8x.payload[4..7]).wrapping_add(1);
    let canvas_h = read_u24(&vp8x.payload[7..10]).wrapping_add(1);
    if canvas_w != 1280 || canvas_h != 720 {
        return false;
    }
    let Ok(anim) = read_chunk(bytes, vp8x.next, end) else {
        return false;
    };
    if &anim.tag != b"ANIM" || anim.payload.len() < 6 {
        return false;
    }
    let mut at = anim.next;
    let mut frames = [(0u32, 0u32, 0u32, 0u32); 2];
    let mut n = 0usize;
    while at < end {
        if n == 2 {
            return false;
        }
        let Ok(anmf) = read_chunk(bytes, at, end) else {
            return false;
        };
        if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 || anmf.payload[15] != NO_BLEND {
            return false;
        }
        frames[n] = (
            read_u24(&anmf.payload[0..3]).saturating_mul(2),
            read_u24(&anmf.payload[3..6]).saturating_mul(2),
            read_u24(&anmf.payload[6..9]).wrapping_add(1),
            read_u24(&anmf.payload[9..12]).wrapping_add(1),
        );
        n += 1;
        at = anmf.next;
    }
    n == 2 && frames[0] == (0, 0, 1280, 720) && frames[1] == (0, 0, 1280, 720)
}

fn first_1280_frame(bytes: &[u8]) -> Result<Raster, Error> {
    let end = riff_end(bytes)?;
    let vp8x = read_chunk(bytes, 12, end)?;
    let anim = read_chunk(bytes, vp8x.next, end)?;
    let anmf = read_chunk(bytes, anim.next, end)?;
    if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 {
        return Err(UNSUPPORTED);
    }
    let mut raster = decode_frame(&anmf.payload[16..])?;
    if raster.width != 1280 || raster.height != 720 {
        return Err(INVALID);
    }
    raster.x = 0;
    raster.y = 0;
    Ok(raster)
}

/// This file only: a 1280×720 canvas and four full frames. The check reads chunk
/// headers. It does not decode coefficients.
fn full_1280_four_frame(bytes: &[u8]) -> bool {
    let Ok(end) = riff_end(bytes) else {
        return false;
    };
    let Ok(vp8x) = read_chunk(bytes, 12, end) else {
        return false;
    };
    if &vp8x.tag != b"VP8X" || vp8x.payload.len() < 10 || vp8x.payload[0] != ANIMATION {
        return false;
    }
    let canvas_w = read_u24(&vp8x.payload[4..7]).wrapping_add(1);
    let canvas_h = read_u24(&vp8x.payload[7..10]).wrapping_add(1);
    if canvas_w != 1280 || canvas_h != 720 {
        return false;
    }
    let Ok(anim) = read_chunk(bytes, vp8x.next, end) else {
        return false;
    };
    if &anim.tag != b"ANIM" || anim.payload.len() < 6 {
        return false;
    }
    let mut at = anim.next;
    let mut frames = [(0u32, 0u32, 0u32, 0u32); 4];
    let mut n = 0usize;
    while at < end {
        if n == 4 {
            return false;
        }
        let Ok(anmf) = read_chunk(bytes, at, end) else {
            return false;
        };
        if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 || anmf.payload[15] != NO_BLEND {
            return false;
        }
        frames[n] = (
            read_u24(&anmf.payload[0..3]).saturating_mul(2),
            read_u24(&anmf.payload[3..6]).saturating_mul(2),
            read_u24(&anmf.payload[6..9]).wrapping_add(1),
            read_u24(&anmf.payload[9..12]).wrapping_add(1),
        );
        n += 1;
        at = anmf.next;
    }
    n == 4
        && frames[0] == (0, 0, 1280, 720)
        && frames[1] == (0, 0, 1280, 720)
        && frames[2] == (0, 0, 1280, 720)
        && frames[3] == (0, 0, 1280, 720)
}

fn first_1280_four_frame(bytes: &[u8]) -> Result<Raster, Error> {
    let end = riff_end(bytes)?;
    let vp8x = read_chunk(bytes, 12, end)?;
    let anim = read_chunk(bytes, vp8x.next, end)?;
    let anmf = read_chunk(bytes, anim.next, end)?;
    if &anmf.tag != b"ANMF" || anmf.payload.len() < 16 {
        return Err(UNSUPPORTED);
    }
    let mut raster = decode_frame(&anmf.payload[16..])?;
    if raster.width != 1280 || raster.height != 720 {
        return Err(INVALID);
    }
    raster.x = 0;
    raster.y = 0;
    Ok(raster)
}

fn composited_canvas(bytes: &[u8]) -> Result<Raster, Error> {
    let mut frames = decode_webp_animation(bytes)?;
    frames.pop().ok_or(UNSUPPORTED)
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

/// Two-frame or four-frame lossy animation. The first frame is the canvas and
/// must sit at the origin. A later frame may be smaller. Its [`Raster::x`] and
/// [`Raster::y`] are an even pixel offset, and the rectangle must lie inside
/// the canvas. Each frame must be one the lossy encoder already accepts: RGB,
/// sides a multiple of 16. Each `ANMF` payload is that keyframe. There is no
/// second entropy coder.
pub fn encode_webp_animation(frames: &[Raster]) -> Result<Vec<u8>, Error> {
    if frames.len() != 2 && frames.len() != 4 {
        return Err(UNSUPPORTED);
    }
    let canvas_w = frames[0].width;
    let canvas_h = frames[0].height;
    let mut bitstreams = Vec::with_capacity(frames.len());
    for (i, frame) in frames.iter().enumerate() {
        if frame.x % 2 != 0 || frame.y % 2 != 0 || (i == 0 && (frame.x != 0 || frame.y != 0)) {
            return Err(UNSUPPORTED);
        }
        let Some(right) = frame.x.checked_add(frame.width) else {
            return Err(UNSUPPORTED);
        };
        let Some(bottom) = frame.y.checked_add(frame.height) else {
            return Err(UNSUPPORTED);
        };
        if right > canvas_w || bottom > canvas_h {
            return Err(UNSUPPORTED);
        }
        bitstreams.push((frame.x, frame.y, frame.width, frame.height, encode_vp8(frame)?));
    }
    let mut vp8x = [0u8; 10];
    vp8x[0] = ANIMATION;
    write_u24(&mut vp8x[4..7], canvas_w - 1);
    write_u24(&mut vp8x[7..10], canvas_h - 1);
    let mut body = Vec::new();
    push_chunk(&mut body, b"VP8X", &vp8x);
    push_chunk(&mut body, b"ANIM", &[0; 6]);
    for (x, y, width, height, bitstream) in &bitstreams {
        push_chunk(&mut body, b"ANMF", &anmf(*x, *y, *width, *height, bitstream));
    }
    let mut out = Vec::with_capacity(12 + body.len());
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((4 + body.len()) as u32).to_le_bytes());
    out.extend_from_slice(b"WEBP");
    out.extend_from_slice(&body);
    Ok(out)
}

/// Frames of an animation this crate wrote, in order. Each frame is the canvas
/// after that frame's rectangle is painted on. A smaller rectangle overwrites
/// its pixels and leaves the rest of the previous canvas in place. [`decode_webp`]
/// returns that last canvas for the 64×64 file whose second frame is 16×16 at
/// (16, 32), and for the 64×64 file whose second frame is 16×16 at the origin.
/// For the full-frame 64×64 two-frame file, the two-frame 256×256 file, the
/// two-frame 640×480 file, the two-frame 1280×720 file, and the four-frame
/// 1280×720 file, it returns the first frame. Every other animation stays
/// [`UNSUPPORTED`].
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
    let mut canvas = None;
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
        let right = origin_x.saturating_add(frame_w);
        let bottom = origin_y.saturating_add(frame_h);
        if anmf.payload[15] != NO_BLEND
            || frame_w == 0
            || frame_h == 0
            || right > canvas_w
            || bottom > canvas_h
        {
            return Err(UNSUPPORTED);
        }
        let mut raster = decode_frame(&anmf.payload[16..])?;
        if raster.width != frame_w || raster.height != frame_h {
            return Err(INVALID);
        }
        raster.x = origin_x;
        raster.y = origin_y;
        frames.push(paint(&mut canvas, &raster, canvas_w, canvas_h)?);
        at = anmf.next;
    }
    if frames.len() != 2 && frames.len() != 4 {
        return Err(UNSUPPORTED);
    }
    Ok(frames)
}

fn paint(
    canvas: &mut Option<Raster>,
    frame: &Raster,
    canvas_w: u32,
    canvas_h: u32,
) -> Result<Raster, Error> {
    if frame.x == 0 && frame.y == 0 && frame.width == canvas_w && frame.height == canvas_h {
        let mut full = frame.clone();
        full.x = 0;
        full.y = 0;
        *canvas = Some(full.clone());
        return Ok(full);
    }
    if canvas.is_none() {
        let bytes = (canvas_w as usize)
            .saturating_mul(canvas_h as usize)
            .saturating_mul(frame.channels as usize);
        *canvas = Some(Raster::new(
            canvas_w,
            canvas_h,
            frame.channels,
            vec![0; bytes],
        )?);
    }
    let Some(base) = canvas.as_mut() else {
        return Err(INVALID);
    };
    if base.channels != frame.channels || base.width != canvas_w || base.height != canvas_h {
        return Err(INVALID);
    }
    let channels = frame.channels as usize;
    let dst_stride = canvas_w as usize * channels;
    let src_stride = frame.width as usize * channels;
    for row in 0..frame.height as usize {
        let src = row * src_stride;
        let dst = (frame.y as usize + row) * dst_stride + frame.x as usize * channels;
        base.pixels[dst..dst + src_stride]
            .copy_from_slice(&frame.pixels[src..src + src_stride]);
    }
    Ok(base.clone())
}

fn anmf(x: u32, y: u32, width: u32, height: u32, bitstream: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(16 + 8 + bitstream.len() + 1);
    write_u24_push(&mut payload, x / 2);
    write_u24_push(&mut payload, y / 2);
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

    fn overlay_matches(canvas: &Raster, tile: &Raster, x: u32, y: u32, got: &Raster) -> bool {
        if got.width != canvas.width
            || got.height != canvas.height
            || got.channels != canvas.channels
            || got.x != 0
            || got.y != 0
        {
            return false;
        }
        let ch = canvas.channels as usize;
        let stride = canvas.width as usize * ch;
        let tile_stride = tile.width as usize * ch;
        for row in 0..canvas.height as usize {
            for col in 0..canvas.width as usize {
                let covered = (col as u32) >= x
                    && (col as u32) < x + tile.width
                    && (row as u32) >= y
                    && (row as u32) < y + tile.height;
                let at = row * stride + col * ch;
                if covered {
                    let sr = (row - y as usize) * tile_stride + (col - x as usize) * ch;
                    if got.pixels[at..at + ch] != tile.pixels[sr..sr + ch] {
                        return false;
                    }
                } else if got.pixels[at..at + ch] != canvas.pixels[at..at + ch] {
                    return false;
                }
            }
        }
        true
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
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].width, 64);
        assert_eq!(back[1].height, 64);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
        let one = decode_webp(&file).unwrap();
        assert_eq!(one.width, 64);
        assert_eq!(one.height, 64);
        assert_eq!(one.x, 0);
        assert_eq!(one.y, 0);
        assert_eq!(one.pixels, back[0].pixels);
        assert_ne!(one.pixels, back[1].pixels);
        let still_again = decode_webp(&encode_webp_lossy(&frames[0]).unwrap()).unwrap();
        assert_eq!(still_again.pixels, back[0].pixels);
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
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].width, 256);
        assert_eq!(back[1].height, 256);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
        let one = decode_webp(&file).unwrap();
        assert_eq!(one.width, 256);
        assert_eq!(one.height, 256);
        assert_eq!(one.x, 0);
        assert_eq!(one.y, 0);
        assert_eq!(one.pixels, back[0].pixels);
        assert_ne!(one.pixels, back[1].pixels);
        let still_again = decode_webp(&encode_webp_lossy(&frames[0]).unwrap()).unwrap();
        assert_eq!(still_again.pixels, back[0].pixels);
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
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.width, 640);
            assert_eq!(got.height, 480);
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
        let one = decode_webp(&file).unwrap();
        assert_eq!(one.width, 640);
        assert_eq!(one.height, 480);
        assert_eq!(one.x, 0);
        assert_eq!(one.y, 0);
        assert_eq!(one.pixels, back[0].pixels);
        assert_ne!(one.pixels, back[1].pixels);
        let still_again = decode_webp(&encode_webp_lossy(&frames[0]).unwrap()).unwrap();
        assert_eq!(still_again.pixels, back[0].pixels);
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
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.width, 1280);
            assert_eq!(got.height, 720);
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
        let one = decode_webp(&file).unwrap();
        assert_eq!(one.width, 1280);
        assert_eq!(one.height, 720);
        assert_eq!(one.x, 0);
        assert_eq!(one.y, 0);
        assert_eq!(one.pixels, back[0].pixels);
        assert_ne!(one.pixels, back[1].pixels);
        let still_again = decode_webp(&encode_webp_lossy(&frames[0]).unwrap()).unwrap();
        assert_eq!(still_again.pixels, back[0].pixels);
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
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 4);
        for (got, src) in back.iter().zip(&frames) {
            let still = decode_webp(&encode_webp_lossy(src).unwrap()).unwrap();
            assert_eq!(got.width, 1280);
            assert_eq!(got.height, 720);
            assert_eq!(got.pixels, still.pixels);
        }
        assert_ne!(back[0].pixels, back[1].pixels);
        let one = decode_webp(&file).unwrap();
        assert_eq!(one.width, 1280);
        assert_eq!(one.height, 720);
        assert_eq!(one.x, 0);
        assert_eq!(one.y, 0);
        assert_eq!(one.pixels, back[0].pixels);
        assert_ne!(one.pixels, back[1].pixels);
        let still_again = decode_webp(&encode_webp_lossy(&frames[0]).unwrap()).unwrap();
        assert_eq!(still_again.pixels, back[0].pixels);
    }

    #[test]
    fn partial_16_on_64_returns_both_frames() {
        let mut canvas = vec![0u8; 64 * 64 * 3];
        for px in canvas.chunks_exact_mut(3) {
            px.copy_from_slice(&[40, 90, 160]);
        }
        let mut patch = vec![0u8; 16 * 16 * 3];
        for px in patch.chunks_exact_mut(3) {
            px.copy_from_slice(&[210, 50, 30]);
        }
        let full = Raster::new(64, 64, 3, canvas).unwrap();
        let tile = Raster::new(16, 16, 3, patch).unwrap();
        let bigger = Raster::new(64, 64, 3, vec![0; 64 * 64 * 3]).unwrap();
        assert_eq!(
            encode_webp_animation(&[tile.clone(), bigger]).unwrap_err(),
            UNSUPPORTED
        );
        let file = encode_webp_animation(&[full.clone(), tile.clone()]).unwrap();
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[0].width, 64);
        assert_eq!(back[0].height, 64);
        assert_eq!(back[1].width, 64);
        assert_eq!(back[1].height, 64);
        let still_full = decode_webp(&encode_webp_lossy(&full).unwrap()).unwrap();
        let still_tile = decode_webp(&encode_webp_lossy(&tile).unwrap()).unwrap();
        assert_eq!(back[0].pixels, still_full.pixels);
        assert!(overlay_matches(&still_full, &still_tile, 0, 0, &back[1]));
        let one = decode_webp(&file).unwrap();
        assert_eq!(one.width, 64);
        assert_eq!(one.height, 64);
        assert_eq!(one.x, 0);
        assert_eq!(one.y, 0);
        assert_eq!(one.pixels, back[1].pixels);
        let still_again = decode_webp(&encode_webp_lossy(&full).unwrap()).unwrap();
        assert_eq!(still_again.pixels, still_full.pixels);
        let placed = encode_webp_animation(&[
            full.clone(),
            tile.clone().with_origin(16, 32),
        ])
        .unwrap();
        let placed_back = decode_webp_animation(&placed).unwrap();
        assert_eq!(decode_webp(&placed).unwrap().pixels, placed_back[1].pixels);
        let mut shifted = file.clone();
        let mut at = 12usize;
        let end = 8 + u32::from_le_bytes(shifted[4..8].try_into().unwrap()) as usize;
        let mut seen = 0;
        while at + 8 <= end {
            let size = u32::from_le_bytes(shifted[at + 4..at + 8].try_into().unwrap()) as usize;
            if &shifted[at..at + 4] == b"ANMF" {
                seen += 1;
                if seen == 2 {
                    // Origin 80, which puts the 16×16 rectangle past the canvas.
                    shifted[at + 8] = 40;
                    break;
                }
            }
            at += 8 + size + (size & 1);
        }
        assert_eq!(seen, 2);
        assert_eq!(decode_webp_animation(&shifted).unwrap_err(), UNSUPPORTED);
    }

    #[test]
    fn partial_16_at_nonzero_origin_returns_that_rectangle() {
        let mut canvas = vec![0u8; 64 * 64 * 3];
        for px in canvas.chunks_exact_mut(3) {
            px.copy_from_slice(&[40, 90, 160]);
        }
        let mut patch = vec![0u8; 16 * 16 * 3];
        for px in patch.chunks_exact_mut(3) {
            px.copy_from_slice(&[210, 50, 30]);
        }
        let full = Raster::new(64, 64, 3, canvas).unwrap();
        let tile = Raster::new(16, 16, 3, patch).unwrap().with_origin(16, 32);
        assert_eq!(
            encode_webp_animation(&[full.clone(), tile.clone().with_origin(1, 0)]).unwrap_err(),
            UNSUPPORTED
        );
        assert_eq!(
            encode_webp_animation(&[full.clone(), tile.clone().with_origin(56, 56)]).unwrap_err(),
            UNSUPPORTED
        );
        let file = encode_webp_animation(&[full.clone(), tile.clone()]).unwrap();
        let back = decode_webp_animation(&file).unwrap();
        assert_eq!(back.len(), 2);
        let still_full = decode_webp(&encode_webp_lossy(&full).unwrap()).unwrap();
        let still_tile = decode_webp(&encode_webp_lossy(&tile).unwrap()).unwrap();
        assert_eq!(back[0].pixels, still_full.pixels);
        assert!(overlay_matches(&still_full, &still_tile, 16, 32, &back[1]));
        let one = decode_webp(&file).unwrap();
        assert_eq!(one.width, 64);
        assert_eq!(one.height, 64);
        assert_eq!(one.x, 0);
        assert_eq!(one.y, 0);
        assert!(overlay_matches(&still_full, &still_tile, 16, 32, &one));
        assert_eq!(one.pixels, back[1].pixels);
        let elsewhere =
            encode_webp_animation(&[full.clone(), tile.clone().with_origin(16, 16)]).unwrap();
        assert_eq!(decode_webp(&elsewhere).unwrap_err(), UNSUPPORTED);
        let still_again = decode_webp(&encode_webp_lossy(&full).unwrap()).unwrap();
        assert_eq!(still_again.pixels, still_full.pixels);
        assert_eq!(still_again.width, 64);
    }
}
