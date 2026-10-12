//! 8-bit PNG decode and a stored-zlib encoder for fixtures.
//! Interlaced images are refused.

use crate::crc::Crc32;
use crate::deflate::{inflate_zlib, zlib_store};
use crate::error::{Error, Result};

pub const MAX_PIXELS: u64 = 64_000_000;

pub struct RgbImage {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

pub fn encode_png_rgb(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_PIXELS || rgb.len() != pixels as usize * 3 {
        return Err(Error::Format("png image"));
    }
    let mut raw = Vec::with_capacity((u64::from(width) * 3 + 1) as usize * height as usize);
    let row = width as usize * 3;
    for y in 0..height as usize {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * row..(y + 1) * row]);
    }
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut png = Vec::new();
    png.extend_from_slice(b"\x89PNG\r\n\x1a\n");
    png.extend(chunk(b"IHDR", &ihdr));
    png.extend(chunk(b"IDAT", &zlib_store(&raw)));
    png.extend(chunk(b"IEND", &[]));
    Ok(png)
}

pub fn decode_png(data: &[u8]) -> Result<RgbImage> {
    if data.len() < 8 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        return Err(Error::Format("png signature"));
    }
    let mut pos = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color = 0u8;
    let mut depth = 0u8;
    let mut interlace = 0u8;
    let mut idat = Vec::new();
    let mut palette: Vec<u8> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut saw_ihdr = false;
    let mut saw_iend = false;
    while pos + 12 <= data.len() {
        let len =
            u32::from_be_bytes([data[pos], data[pos + 1], data[pos + 2], data[pos + 3]]) as usize;
        let tag = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        let start = pos + 8;
        let end = start.checked_add(len).ok_or(Error::Format("truncated"))?;
        if end + 4 > data.len() {
            return Err(Error::Format("truncated"));
        }
        let chunk_crc =
            u32::from_be_bytes([data[end], data[end + 1], data[end + 2], data[end + 3]]);
        if Crc32::checksum_of(&data[pos + 4..end]) != chunk_crc {
            return Err(Error::ChecksumMismatch);
        }
        let body = &data[start..end];
        match &tag {
            b"IHDR" => {
                if body.len() != 13 {
                    return Err(Error::Format("png image"));
                }
                width = u32::from_be_bytes([body[0], body[1], body[2], body[3]]);
                height = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
                depth = body[8];
                color = body[9];
                if body[10] != 0 || body[11] != 0 {
                    return Err(Error::Unsupported("png compression"));
                }
                interlace = body[12];
                saw_ihdr = true;
            }
            b"PLTE" => palette = body.to_vec(),
            b"tRNS" => trns = body.to_vec(),
            b"IDAT" => idat.extend_from_slice(body),
            b"IEND" => {
                saw_iend = true;
                break;
            }
            _ => {
                if tag[0] & 0x20 == 0 {
                    return Err(Error::Unsupported("png chunk"));
                }
            }
        }
        pos = end + 4;
    }
    if !saw_ihdr || !saw_iend {
        return Err(Error::Format("png image"));
    }
    if interlace != 0 {
        return Err(Error::Unsupported("png interlace"));
    }
    if depth != 8 {
        return Err(Error::Unsupported("png bit depth"));
    }
    let pixels = u64::from(width) * u64::from(height);
    if width == 0 || height == 0 || pixels > MAX_PIXELS {
        return Err(Error::Limit("png image"));
    }
    let channels = match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        6 => 4,
        _ => return Err(Error::Unsupported("png color")),
    };
    if color == 3 && (palette.is_empty() || palette.len() % 3 != 0) {
        return Err(Error::Format("png palette"));
    }
    let row_bytes = width as usize * channels;
    let expect = height as usize * (row_bytes + 1);
    let inflated = inflate_zlib(&idat)?;
    if inflated.len() != expect {
        return Err(Error::Format("png image"));
    }
    let mut raw = vec![0u8; height as usize * row_bytes];
    unfilter(&inflated, &mut raw, height as usize, row_bytes, channels)?;
    let rgb = to_rgb(&raw, width, height, color, &palette, &trns)?;
    Ok(RgbImage { width, height, rgb })
}

fn unfilter(src: &[u8], dst: &mut [u8], height: usize, row: usize, channels: usize) -> Result<()> {
    let bpp = channels.max(1);
    for y in 0..height {
        let filter = src[y * (row + 1)];
        let in_row = &src[y * (row + 1) + 1..y * (row + 1) + 1 + row];
        let (prev, cur) = dst.split_at_mut(y * row);
        let out = &mut cur[..row];
        let up = if y == 0 {
            None
        } else {
            Some(&prev[prev.len() - row..])
        };
        for x in 0..row {
            let left = if x >= bpp { out[x - bpp] } else { 0 };
            let above = up.map(|row| row[x]).unwrap_or(0);
            let above_left = if x >= bpp {
                up.map(|row| row[x - bpp]).unwrap_or(0)
            } else {
                0
            };
            let value = in_row[x];
            out[x] = match filter {
                0 => value,
                1 => value.wrapping_add(left),
                2 => value.wrapping_add(above),
                3 => value.wrapping_add(((u16::from(left) + u16::from(above)) / 2) as u8),
                4 => value.wrapping_add(paeth(left, above, above_left)),
                _ => return Err(Error::Format("png filter")),
            };
        }
    }
    Ok(())
}

fn paeth(left: u8, above: u8, above_left: u8) -> u8 {
    let a = i16::from(left);
    let b = i16::from(above);
    let c = i16::from(above_left);
    let p = a + b - c;
    let pa = (p - a).abs();
    let pb = (p - b).abs();
    let pc = (p - c).abs();
    if pa <= pb && pa <= pc {
        left
    } else if pb <= pc {
        above
    } else {
        above_left
    }
}

fn to_rgb(
    raw: &[u8],
    width: u32,
    height: u32,
    color: u8,
    palette: &[u8],
    trns: &[u8],
) -> Result<Vec<u8>> {
    let n = width as usize * height as usize;
    let mut rgb = vec![0u8; n * 3];
    match color {
        0 => {
            for i in 0..n {
                let g = raw[i];
                rgb[i * 3] = g;
                rgb[i * 3 + 1] = g;
                rgb[i * 3 + 2] = g;
            }
        }
        2 => rgb.copy_from_slice(raw),
        3 => {
            for i in 0..n {
                let idx = usize::from(raw[i]);
                if idx * 3 + 2 >= palette.len() {
                    return Err(Error::Format("png palette"));
                }
                let alpha = trns.get(idx).copied().unwrap_or(255);
                rgb[i * 3] = blend(palette[idx * 3], alpha);
                rgb[i * 3 + 1] = blend(palette[idx * 3 + 1], alpha);
                rgb[i * 3 + 2] = blend(palette[idx * 3 + 2], alpha);
            }
        }
        4 => {
            for i in 0..n {
                let g = raw[i * 2];
                let alpha = raw[i * 2 + 1];
                let v = blend(g, alpha);
                rgb[i * 3] = v;
                rgb[i * 3 + 1] = v;
                rgb[i * 3 + 2] = v;
            }
        }
        6 => {
            for i in 0..n {
                let alpha = raw[i * 4 + 3];
                rgb[i * 3] = blend(raw[i * 4], alpha);
                rgb[i * 3 + 1] = blend(raw[i * 4 + 1], alpha);
                rgb[i * 3 + 2] = blend(raw[i * 4 + 2], alpha);
            }
        }
        _ => return Err(Error::Unsupported("png color")),
    }
    Ok(rgb)
}

fn blend(color: u8, alpha: u8) -> u8 {
    if alpha == 255 {
        color
    } else {
        ((u16::from(color) * u16::from(alpha) + 255 * u16::from(255 - alpha)) / 255) as u8
    }
}

fn chunk(tag: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(tag);
    out.extend_from_slice(data);
    let sum = Crc32::checksum_of(&out[4..]);
    out.extend_from_slice(&sum.to_be_bytes());
    out
}

pub fn jpeg_size(data: &[u8]) -> Result<(u32, u32, bool)> {
    if data.len() < 4 || data[0] != 0xFF || data[1] != 0xD8 {
        return Err(Error::Format("jpeg"));
    }
    let mut i = 2usize;
    while i + 3 < data.len() {
        if data[i] != 0xFF {
            return Err(Error::Format("jpeg"));
        }
        while i < data.len() && data[i] == 0xFF {
            i += 1;
        }
        if i >= data.len() {
            break;
        }
        let marker = data[i];
        i += 1;
        if marker == 0xD9 || marker == 0xDA {
            break;
        }
        if marker == 0x01 || (0xD0..=0xD7).contains(&marker) {
            continue;
        }
        if i + 1 >= data.len() {
            return Err(Error::Format("truncated"));
        }
        let len = u16::from_be_bytes([data[i], data[i + 1]]) as usize;
        if len < 2 || i + len > data.len() {
            return Err(Error::Format("truncated"));
        }
        if matches!(marker, 0xC0..=0xC2) {
            if len < 8 {
                return Err(Error::Format("jpeg"));
            }
            let height = u16::from_be_bytes([data[i + 3], data[i + 4]]);
            let width = u16::from_be_bytes([data[i + 5], data[i + 6]]);
            let components = data[i + 7];
            if width == 0 || height == 0 {
                return Err(Error::Format("jpeg"));
            }
            let pixels = u64::from(width) * u64::from(height);
            if pixels > MAX_PIXELS {
                return Err(Error::Limit("jpeg"));
            }
            return Ok((u32::from(width), u32::from(height), components == 1));
        }
        i += len;
    }
    Err(Error::Format("jpeg"))
}

pub fn is_png(data: &[u8]) -> bool {
    data.len() >= 8 && &data[..8] == b"\x89PNG\r\n\x1a\n"
}

pub fn is_jpeg(data: &[u8]) -> bool {
    data.len() >= 3 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF
}
