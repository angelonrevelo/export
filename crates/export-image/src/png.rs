//! PNG decode (ISO/IEC 15948) and a sample-exact encoder.
//!
//! Decode accepts 8-bit gray, gray+alpha, RGB, RGBA, and indexed color, plus
//! 1/2/4-bit gray and indexed images, Adam7 interlace at 8 bits, and every
//! filter. 16-bit and low-bit Adam7 are rejected so a file that cannot be
//! restored exactly is a counted failure instead of a silent truncation.
//!
//! [`restore_lossless`] does not re-encode. A successful call means `bytes`
//! is already the lossless form.

use crate::error::{Error, INVALID, TOO_LARGE, TRUNCATED, UNSUPPORTED};
use crate::inflate::{zlib_compress, zlib_inflate};
use crate::raster::Raster;
use crate::sniff::{sniff, Format};
use crate::MAX_PIXELS;

const SIG: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const X0: [u32; 7] = [0, 4, 0, 2, 0, 1, 0];
const Y0: [u32; 7] = [0, 0, 4, 0, 2, 0, 1];
const XS: [u32; 7] = [8, 8, 4, 4, 2, 2, 1];
const YS: [u32; 7] = [8, 8, 8, 4, 4, 2, 2];

#[derive(Clone, Copy, Debug)]
pub(crate) struct PngInfo {
    pub width: u32,
    pub height: u32,
    pub depth: u8,
    pub color: u8,
    pub interlace: u8,
}

/// Read and check IHDR. Does not inflate, so a byte-cap copy does not expand
/// a decompression bomb. Dimensions above [`MAX_PIXELS`] are [`TOO_LARGE`].
pub(crate) fn png_header(bytes: &[u8]) -> Result<PngInfo, Error> {
    if sniff(bytes) != Format::Png || bytes.len() < 8 + 8 + 13 + 4 {
        return Err(INVALID);
    }
    let len = u32::from_be_bytes(bytes[8..12].try_into().unwrap());
    let tag = &bytes[12..16];
    if len != 13 || tag != b"IHDR" {
        return Err(INVALID);
    }
    let data = &bytes[16..29];
    let stored = u32::from_be_bytes(bytes[29..33].try_into().unwrap());
    if stored != crc32_parts(&[b"IHDR", data]) {
        return Err(INVALID);
    }
    let info = PngInfo {
        width: u32::from_be_bytes(data[0..4].try_into().unwrap()),
        height: u32::from_be_bytes(data[4..8].try_into().unwrap()),
        depth: data[8],
        color: data[9],
        interlace: data[12],
    };
    if info.width == 0 || info.height == 0 {
        return Err(INVALID);
    }
    let n = (info.width as u64)
        .checked_mul(info.height as u64)
        .ok_or(TOO_LARGE)?;
    if n > MAX_PIXELS {
        return Err(TOO_LARGE);
    }
    if data[10] != 0 || data[11] != 0 || info.interlace > 1 {
        return Err(INVALID);
    }
    Ok(info)
}

pub fn decode_png(bytes: &[u8]) -> Result<Raster, Error> {
    let info = png_header(bytes)?;
    let spp = samples_per_pixel(info.color)?;
    check_depth(info.depth, info.color, info.interlace)?;
    let mut idat: Vec<u8> = Vec::new();
    let mut plte: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut i = 8usize;
    let mut saw_iend = false;
    while i + 8 <= bytes.len() {
        let len = u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap()) as usize;
        let tag = [bytes[i + 4], bytes[i + 5], bytes[i + 6], bytes[i + 7]];
        let start = i + 8;
        let end = start.checked_add(len).ok_or(INVALID)?;
        if end + 4 > bytes.len() {
            return Err(TRUNCATED);
        }
        let stored = u32::from_be_bytes(bytes[end..end + 4].try_into().unwrap());
        if stored != crc32_parts(&[&tag, &bytes[start..end]]) {
            return Err(INVALID);
        }
        match &tag {
            b"IHDR" => {}
            b"PLTE" => plte = bytes[start..end].to_vec(),
            b"tRNS" => trns = bytes[start..end].to_vec(),
            b"IDAT" => idat.extend_from_slice(&bytes[start..end]),
            b"IEND" => {
                saw_iend = true;
                break;
            }
            _ => {
                if tag[0].is_ascii_uppercase() {
                    return Err(UNSUPPORTED);
                }
            }
        }
        i = end + 4;
    }
    if !saw_iend {
        return Err(TRUNCATED);
    }
    if info.color == 3 && (plte.is_empty() || plte.len() % 3 != 0) {
        return Err(INVALID);
    }
    let expected = raw_len(info, spp)?;
    let raw = zlib_inflate(&idat, expected)?;
    if raw.len() != expected {
        return Err(INVALID);
    }
    let samples = defilter(&raw, info, spp)?;
    samples_to_raster(info, spp, &samples, &plte, &trns)
}

pub fn encode_png(src: &Raster) -> Result<Vec<u8>, Error> {
    let color: u8 = match src.channels {
        1 => 0,
        2 => 4,
        3 => 2,
        4 => 6,
        _ => return Err(INVALID),
    };
    let stride = src.width as usize * src.channels as usize;
    let mut raw = Vec::with_capacity((stride + 1) * src.height as usize);
    for y in 0..src.height as usize {
        raw.push(0);
        let row = y * stride;
        raw.extend_from_slice(&src.pixels[row..row + stride]);
    }
    let compressed = zlib_compress(&raw);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&src.width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&src.height.to_be_bytes());
    ihdr[8] = 8;
    ihdr[9] = color;
    let mut out = Vec::with_capacity(64 + compressed.len());
    out.extend_from_slice(&SIG);
    push_chunk(&mut out, b"IHDR", &ihdr);
    push_chunk(&mut out, b"IDAT", &compressed);
    push_chunk(&mut out, b"IEND", &[]);
    Ok(out)
}

/// Validate a PNG. On success the caller's buffer is the lossless bytes.
pub fn restore_lossless(bytes: &[u8]) -> Result<(), Error> {
    decode_png(bytes)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LosslessReport {
    pub seen: u32,
    pub restored: u32,
    pub failed: u32,
}

/// Decode each file. `failed` counts buffers that are not a lossless PNG.
/// A restored file is not re-encoded: its original bytes are the result.
pub fn restore_lossless_all(files: &[&[u8]]) -> LosslessReport {
    let mut report = LosslessReport {
        seen: files.len() as u32,
        restored: 0,
        failed: 0,
    };
    for file in files {
        if restore_lossless(file).is_ok() {
            report.restored += 1;
        } else {
            report.failed += 1;
        }
    }
    report
}

fn samples_per_pixel(color: u8) -> Result<usize, Error> {
    match color {
        0 | 3 => Ok(1),
        2 => Ok(3),
        4 => Ok(2),
        6 => Ok(4),
        _ => Err(INVALID),
    }
}

fn check_depth(depth: u8, color: u8, interlace: u8) -> Result<(), Error> {
    let ok = match color {
        0 => matches!(depth, 1 | 2 | 4 | 8),
        3 => matches!(depth, 1 | 2 | 4 | 8),
        2 | 4 | 6 => depth == 8,
        _ => false,
    };
    if !ok {
        // 16-bit would drop samples. Count it as unsupported, not as success.
        return Err(if depth == 16 { UNSUPPORTED } else { INVALID });
    }
    if interlace == 1 && depth != 8 {
        return Err(UNSUPPORTED);
    }
    Ok(())
}

fn raw_len(info: PngInfo, spp: usize) -> Result<usize, Error> {
    let bits = spp * info.depth as usize;
    if info.interlace == 0 {
        let stride = row_bytes(info.width, bits);
        return (stride + 1)
            .checked_mul(info.height as usize)
            .ok_or(TOO_LARGE);
    }
    let mut total = 0usize;
    for pass in 0..7 {
        let pw = pass_count(X0[pass], XS[pass], info.width);
        let ph = pass_count(Y0[pass], YS[pass], info.height);
        if pw == 0 || ph == 0 {
            continue;
        }
        let stride = row_bytes(pw, bits);
        let n = (stride + 1).checked_mul(ph as usize).ok_or(TOO_LARGE)?;
        total = total.checked_add(n).ok_or(TOO_LARGE)?;
    }
    Ok(total)
}

fn row_bytes(width: u32, bits_per_pixel: usize) -> usize {
    (width as usize * bits_per_pixel).div_ceil(8)
}

fn pass_count(start: u32, step: u32, full: u32) -> u32 {
    if start >= full {
        0
    } else {
        (full - start).div_ceil(step)
    }
}

fn defilter(raw: &[u8], info: PngInfo, spp: usize) -> Result<Vec<u8>, Error> {
    let dest_n = (info.width as usize)
        .checked_mul(info.height as usize)
        .and_then(|n| n.checked_mul(spp))
        .ok_or(TOO_LARGE)?;
    let mut samples = vec![0u8; dest_n];
    let bits = spp * info.depth as usize;
    if info.interlace == 0 {
        let stride = row_bytes(info.width, bits);
        let rows = unfilter(raw, info.height as usize, stride, filter_bpp(bits))?;
        unpack_into(
            &rows,
            info.width,
            info.height,
            info.depth,
            spp,
            &mut samples,
            Scatter {
                image_w: info.width,
                x0: 0,
                xstep: 1,
                y0: 0,
                ystep: 1,
            },
        )?;
        return Ok(samples);
    }
    let mut offset = 0usize;
    for pass in 0..7 {
        let pw = pass_count(X0[pass], XS[pass], info.width);
        let ph = pass_count(Y0[pass], YS[pass], info.height);
        if pw == 0 || ph == 0 {
            continue;
        }
        let stride = row_bytes(pw, bits);
        let n = (stride + 1) * ph as usize;
        if offset + n > raw.len() {
            return Err(INVALID);
        }
        let rows = unfilter(
            &raw[offset..offset + n],
            ph as usize,
            stride,
            filter_bpp(bits),
        )?;
        unpack_into(
            &rows,
            pw,
            ph,
            info.depth,
            spp,
            &mut samples,
            Scatter {
                image_w: info.width,
                x0: X0[pass],
                xstep: XS[pass],
                y0: Y0[pass],
                ystep: YS[pass],
            },
        )?;
        offset += n;
    }
    if offset != raw.len() {
        return Err(INVALID);
    }
    Ok(samples)
}

fn filter_bpp(bits_per_pixel: usize) -> usize {
    (bits_per_pixel / 8).max(1)
}

fn unfilter(raw: &[u8], height: usize, stride: usize, bpp: usize) -> Result<Vec<u8>, Error> {
    let row_in = stride + 1;
    if height == 0 {
        return Ok(Vec::new());
    }
    if raw.len() != row_in * height {
        return Err(INVALID);
    }
    let mut out = vec![0u8; stride * height];
    for y in 0..height {
        let filter = raw[y * row_in];
        let src = y * row_in + 1;
        let dst = y * stride;
        for x in 0..stride {
            let left = if x >= bpp { out[dst + x - bpp] } else { 0 };
            let up = if y > 0 { out[dst + x - stride] } else { 0 };
            let ul = if y > 0 && x >= bpp {
                out[dst + x - stride - bpp]
            } else {
                0
            };
            let v = raw[src + x];
            out[dst + x] = match filter {
                0 => v,
                1 => v.wrapping_add(left),
                2 => v.wrapping_add(up),
                3 => v.wrapping_add(((u16::from(left) + u16::from(up)) / 2) as u8),
                4 => v.wrapping_add(paeth(left, up, ul)),
                _ => return Err(INVALID),
            };
        }
    }
    Ok(out)
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let a = i16::from(a);
    let b = i16::from(b);
    let c = i16::from(c);
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        a as u8
    } else if pb <= pc {
        b as u8
    } else {
        c as u8
    }
}

struct Scatter {
    image_w: u32,
    x0: u32,
    xstep: u32,
    y0: u32,
    ystep: u32,
}

fn unpack_into(
    rows: &[u8],
    pass_w: u32,
    pass_h: u32,
    depth: u8,
    spp: usize,
    dest: &mut [u8],
    place: Scatter,
) -> Result<(), Error> {
    let bits = spp * depth as usize;
    let stride = row_bytes(pass_w, bits);
    if rows.len() != stride * pass_h as usize {
        return Err(INVALID);
    }
    let image_w = place.image_w as usize;
    for py in 0..pass_h as usize {
        let row = &rows[py * stride..(py + 1) * stride];
        let mut bit = 0usize;
        for px in 0..pass_w as usize {
            let ox = place.x0 as usize + px * place.xstep as usize;
            let oy = place.y0 as usize + py * place.ystep as usize;
            let di = (oy * image_w + ox) * spp;
            for s in 0..spp {
                let v = if depth == 8 {
                    row[px * spp + s]
                } else {
                    let byte = row[bit / 8];
                    let shift = 8 - depth as usize - (bit % 8);
                    let mask = (1u8 << depth) - 1;
                    bit += depth as usize;
                    (byte >> shift) & mask
                };
                dest[di + s] = v;
            }
        }
    }
    Ok(())
}

fn samples_to_raster(
    info: PngInfo,
    spp: usize,
    samples: &[u8],
    plte: &[u8],
    trns: &[u8],
) -> Result<Raster, Error> {
    let n = info.width as usize * info.height as usize;
    if samples.len() != n * spp {
        return Err(INVALID);
    }
    if info.color == 3 {
        let colors = plte.len() / 3;
        if colors == 0 || plte.len() != colors * 3 {
            return Err(INVALID);
        }
        if trns.is_empty() {
            let mut pixels = Vec::with_capacity(n * 3);
            for &sample in &samples[..n] {
                let idx = usize::from(sample);
                if idx >= colors {
                    return Err(INVALID);
                }
                pixels.extend_from_slice(&plte[idx * 3..idx * 3 + 3]);
            }
            return Raster::new(info.width, info.height, 3, pixels);
        }
        let mut pixels = Vec::with_capacity(n * 4);
        for &sample in &samples[..n] {
            let idx = usize::from(sample);
            if idx >= colors {
                return Err(INVALID);
            }
            pixels.extend_from_slice(&plte[idx * 3..idx * 3 + 3]);
            pixels.push(trns.get(idx).copied().unwrap_or(255));
        }
        return Raster::new(info.width, info.height, 4, pixels);
    }
    let scale = |v: u8| -> u8 {
        match info.depth {
            1 => v.saturating_mul(255),
            2 => v.saturating_mul(85),
            4 => v.saturating_mul(17),
            _ => v,
        }
    };
    if info.color == 0 && trns.len() >= 2 {
        let key = u16::from_be_bytes([trns[0], trns[1]]);
        let mut pixels = Vec::with_capacity(n * 2);
        for &raw in &samples[..n] {
            let alpha = if u16::from(raw) == key { 0 } else { 255 };
            pixels.push(scale(raw));
            pixels.push(alpha);
        }
        return Raster::new(info.width, info.height, 2, pixels);
    }
    if info.color == 2 && trns.len() >= 6 {
        let key = [
            u16::from_be_bytes([trns[0], trns[1]]),
            u16::from_be_bytes([trns[2], trns[3]]),
            u16::from_be_bytes([trns[4], trns[5]]),
        ];
        let mut pixels = Vec::with_capacity(n * 4);
        for i in 0..n {
            let s = &samples[i * 3..i * 3 + 3];
            let alpha = if s.iter().zip(key).all(|(&v, k)| u16::from(v) == k) {
                0
            } else {
                255
            };
            pixels.extend_from_slice(s);
            pixels.push(alpha);
        }
        return Raster::new(info.width, info.height, 4, pixels);
    }
    if info.depth == 8 {
        return Raster::new(info.width, info.height, spp as u8, samples.to_vec());
    }
    let pixels: Vec<u8> = samples.iter().copied().map(scale).collect();
    Raster::new(info.width, info.height, spp as u8, pixels)
}

fn crc32_parts(parts: &[&[u8]]) -> u32 {
    let mut c = 0xffff_ffffu32;
    for part in parts {
        for &b in *part {
            c ^= u32::from(b);
            for _ in 0..8 {
                let mask = (c & 1).wrapping_neg();
                c = (c >> 1) ^ (0xedb8_8320 & mask);
            }
        }
    }
    !c
}

fn push_chunk(out: &mut Vec<u8>, tag: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc32_parts(&[tag, data]).to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inflate::zlib_store;

    #[test]
    fn adam7_scatters_every_pixel() {
        let w = 8u32;
        let h = 8u32;
        let mut px = vec![0u8; 8 * 8 * 3];
        for y in 0..8u32 {
            for x in 0..8u32 {
                let i = ((y * 8 + x) * 3) as usize;
                px[i] = x as u8;
                px[i + 1] = y as u8;
                px[i + 2] = 9;
            }
        }
        let mut raw = Vec::new();
        for pass in 0..7 {
            let pw = pass_count(X0[pass], XS[pass], w);
            let ph = pass_count(Y0[pass], YS[pass], h);
            if pw == 0 || ph == 0 {
                continue;
            }
            for py in 0..ph {
                raw.push(0);
                for px_i in 0..pw {
                    let x = X0[pass] + px_i * XS[pass];
                    let y = Y0[pass] + py * YS[pass];
                    let i = ((y * w + x) * 3) as usize;
                    raw.extend_from_slice(&px[i..i + 3]);
                }
            }
        }
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&w.to_be_bytes());
        ihdr[4..8].copy_from_slice(&h.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = 2;
        ihdr[12] = 1;
        let compressed = zlib_store(&raw);
        let mut png = Vec::new();
        png.extend_from_slice(&SIG);
        push_chunk(&mut png, b"IHDR", &ihdr);
        push_chunk(&mut png, b"IDAT", &compressed);
        push_chunk(&mut png, b"IEND", &[]);
        let img = decode_png(&png).unwrap();
        assert_eq!(img.pixels, px);
    }
}
