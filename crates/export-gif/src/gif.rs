//! GIF87a / GIF89a reader and writer.
//!
//! Frames keep their indexed pixels, the color table that applies to them
//! (global or local), and the graphic-control delay in centiseconds.

use crate::error::{Error, INVALID, NOT_GIF, TOO_LARGE, TRUNCATED, UNSUPPORTED_VIDEO};
use crate::lzw;
use crate::sniff::{self, Format};
use crate::MAX_PIXELS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Gif87a,
    Gif89a,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub left: u16,
    pub top: u16,
    pub width: u16,
    pub height: u16,
    /// Hundredths of a second. Zero when the frame has no graphic control.
    pub delay_cs: u16,
    pub disposal: u8,
    pub transparent: Option<u8>,
    pub local_table: Option<Vec<[u8; 3]>>,
    pub indices: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Gif {
    pub version: Version,
    pub width: u16,
    pub height: u16,
    pub background: u8,
    pub aspect: u8,
    pub global_table: Option<Vec<[u8; 3]>>,
    /// Netscape loop count when the application extension is present.
    pub loop_count: Option<u16>,
    pub frames: Vec<Frame>,
}

/// Decode a GIF. Video magics return [`UNSUPPORTED_VIDEO`] and are not parsed.
pub fn decode_gif(bytes: &[u8]) -> Result<Gif, Error> {
    match sniff::sniff(bytes) {
        Format::Gif87a | Format::Gif89a => parse(bytes),
        Format::Video => Err(UNSUPPORTED_VIDEO),
        Format::Unknown => Err(NOT_GIF),
    }
}

/// Encode a GIF. This does not start from a video file.
pub fn encode_gif(gif: &Gif) -> Result<Vec<u8>, Error> {
    validate(gif)?;
    let mut out = Vec::new();
    match gif.version {
        Version::Gif87a => out.extend_from_slice(b"GIF87a"),
        Version::Gif89a => out.extend_from_slice(b"GIF89a"),
    }
    out.extend_from_slice(&gif.width.to_le_bytes());
    out.extend_from_slice(&gif.height.to_le_bytes());
    let mut packed = 0u8;
    if let Some(table) = &gif.global_table {
        let n = size_field(table.len())?;
        let reso = n.min(7);
        packed = 0x80 | (reso << 4) | n;
    }
    out.push(packed);
    out.push(gif.background);
    out.push(gif.aspect);
    if let Some(table) = &gif.global_table {
        write_table(&mut out, table);
    }
    if gif.version == Version::Gif87a {
        if gif.loop_count.is_some() {
            return Err(INVALID);
        }
        for frame in &gif.frames {
            if frame.delay_cs != 0 || frame.disposal != 0 || frame.transparent.is_some() {
                return Err(INVALID);
            }
        }
    } else if let Some(loop_count) = gif.loop_count {
        out.extend_from_slice(&[0x21, 0xff, 0x0b]);
        out.extend_from_slice(b"NETSCAPE2.0");
        out.extend_from_slice(&[0x03, 0x01]);
        out.extend_from_slice(&loop_count.to_le_bytes());
        out.push(0x00);
    }
    for frame in &gif.frames {
        if gif.version == Version::Gif89a {
            let mut gp = (frame.disposal & 0x07) << 2;
            let trans = frame.transparent.unwrap_or(0);
            if frame.transparent.is_some() {
                gp |= 1;
            }
            out.extend_from_slice(&[0x21, 0xf9, 0x04, gp]);
            out.extend_from_slice(&frame.delay_cs.to_le_bytes());
            out.push(trans);
            out.push(0x00);
        }
        out.push(0x2c);
        out.extend_from_slice(&frame.left.to_le_bytes());
        out.extend_from_slice(&frame.top.to_le_bytes());
        out.extend_from_slice(&frame.width.to_le_bytes());
        out.extend_from_slice(&frame.height.to_le_bytes());
        let table = frame
            .local_table
            .as_ref()
            .or(gif.global_table.as_ref())
            .ok_or(INVALID)?;
        let mut ip = 0u8;
        if frame.local_table.is_some() {
            ip = 0x80 | size_field(table.len())?;
        }
        out.push(ip);
        if let Some(local) = &frame.local_table {
            write_table(&mut out, local);
        }
        let min_code = min_code_size(table.len())?;
        for &px in &frame.indices {
            if usize::from(px) >= table.len() {
                return Err(INVALID);
            }
        }
        let compressed = lzw::encode(min_code, &frame.indices)?;
        out.push(min_code);
        write_subblocks(&mut out, &compressed);
    }
    out.push(0x3b);
    Ok(out)
}

/// Video-to-GIF is refused. The bytes are not decoded and ffmpeg is not called.
pub fn video_to_gif(_bytes: &[u8]) -> Result<Gif, Error> {
    Err(UNSUPPORTED_VIDEO)
}

fn validate(gif: &Gif) -> Result<(), Error> {
    let _ = pixel_count(gif.width, gif.height)?;
    if let Some(table) = &gif.global_table {
        check_table(table)?;
    }
    if gif.frames.is_empty() {
        return Err(INVALID);
    }
    let mut total = 0u64;
    for frame in &gif.frames {
        if frame.disposal > 7 {
            return Err(INVALID);
        }
        let n = pixel_count(frame.width, frame.height)?;
        let right = u32::from(frame.left)
            .checked_add(u32::from(frame.width))
            .ok_or(INVALID)?;
        let bottom = u32::from(frame.top)
            .checked_add(u32::from(frame.height))
            .ok_or(INVALID)?;
        if right > u32::from(gif.width) || bottom > u32::from(gif.height) {
            return Err(INVALID);
        }
        if frame.indices.len() != n as usize {
            return Err(INVALID);
        }
        total = total.checked_add(u64::from(n)).ok_or(TOO_LARGE)?;
        if total > MAX_PIXELS {
            return Err(TOO_LARGE);
        }
        if let Some(table) = &frame.local_table {
            check_table(table)?;
        } else if gif.global_table.is_none() {
            return Err(INVALID);
        }
    }
    Ok(())
}

fn parse(bytes: &[u8]) -> Result<Gif, Error> {
    let mut r = Reader { b: bytes, i: 0 };
    let magic = r.bytes(6)?;
    let version = if magic == b"GIF87a" {
        Version::Gif87a
    } else if magic == b"GIF89a" {
        Version::Gif89a
    } else {
        return Err(NOT_GIF);
    };
    let width = r.u16()?;
    let height = r.u16()?;
    let _ = pixel_count(width, height)?;
    let packed = r.u8()?;
    let background = r.u8()?;
    let aspect = r.u8()?;
    let global_table = if packed & 0x80 != 0 {
        Some(read_table(&mut r, packed & 0x07)?)
    } else {
        None
    };

    let mut loop_count = None;
    let mut frames = Vec::new();
    let mut pending: Option<(u16, u8, Option<u8>)> = None;
    let mut total = 0u64;

    loop {
        let b = r.u8()?;
        if b == 0x3b {
            break;
        } else if b == 0x21 {
            let label = r.u8()?;
            if label == 0xf9 {
                let size = r.u8()?;
                if size != 4 {
                    return Err(INVALID);
                }
                let gp = r.u8()?;
                let delay = r.u16()?;
                let trans = r.u8()?;
                let term = r.u8()?;
                if term != 0 {
                    return Err(INVALID);
                }
                let transparent = if gp & 1 != 0 { Some(trans) } else { None };
                pending = Some((delay, (gp >> 2) & 0x07, transparent));
            } else if label == 0xff {
                let blocks = read_subblocks(&mut r)?;
                if blocks.len() >= 14 && &blocks[..11] == b"NETSCAPE2.0" && blocks[11] == 1 {
                    loop_count = Some(u16::from_le_bytes([blocks[12], blocks[13]]));
                }
            } else {
                let _ = read_subblocks(&mut r)?;
            }
        } else if b == 0x2c {
            let left = r.u16()?;
            let top = r.u16()?;
            let fw = r.u16()?;
            let fh = r.u16()?;
            let n = pixel_count(fw, fh)?;
            let right = u32::from(left).checked_add(u32::from(fw)).ok_or(INVALID)?;
            let bottom = u32::from(top).checked_add(u32::from(fh)).ok_or(INVALID)?;
            if right > u32::from(width) || bottom > u32::from(height) {
                return Err(INVALID);
            }
            let ip = r.u8()?;
            let local_table = if ip & 0x80 != 0 {
                Some(read_table(&mut r, ip & 0x07)?)
            } else {
                None
            };
            if local_table.is_none() && global_table.is_none() {
                return Err(INVALID);
            }
            let interlaced = ip & 0x40 != 0;
            let min_code = r.u8()?;
            let data = read_subblocks(&mut r)?;
            let cap = (n as usize).saturating_mul(2).saturating_add(64);
            if data.len() > cap {
                return Err(TOO_LARGE);
            }
            let raw = lzw::decode(min_code, &data, n as usize)?;
            let indices = if interlaced {
                deinterlace(&raw, fw, fh)?
            } else {
                raw
            };
            total = total.checked_add(u64::from(n)).ok_or(TOO_LARGE)?;
            if total > MAX_PIXELS {
                return Err(TOO_LARGE);
            }
            let (delay_cs, disposal, transparent) = pending.take().unwrap_or((0, 0, None));
            frames.push(Frame {
                left,
                top,
                width: fw,
                height: fh,
                delay_cs,
                disposal,
                transparent,
                local_table,
                indices,
            });
        } else {
            return Err(INVALID);
        }
    }
    if frames.is_empty() {
        return Err(INVALID);
    }
    Ok(Gif {
        version,
        width,
        height,
        background,
        aspect,
        global_table,
        loop_count,
        frames,
    })
}

fn deinterlace(raw: &[u8], width: u16, height: u16) -> Result<Vec<u8>, Error> {
    let w = width as usize;
    let h = height as usize;
    if raw.len() != w * h {
        return Err(INVALID);
    }
    let mut out = vec![0u8; raw.len()];
    let passes: [(usize, usize); 4] = [(0, 8), (4, 8), (2, 4), (1, 2)];
    let mut src = 0usize;
    for (start, step) in passes {
        let mut y = start;
        while y < h {
            let row = y * w;
            let next = src + w;
            if next > raw.len() {
                return Err(TRUNCATED);
            }
            out[row..row + w].copy_from_slice(&raw[src..next]);
            src = next;
            y += step;
        }
    }
    if src != raw.len() {
        return Err(INVALID);
    }
    Ok(out)
}

fn pixel_count(width: u16, height: u16) -> Result<u32, Error> {
    if width == 0 || height == 0 {
        return Err(INVALID);
    }
    let n = u32::from(width)
        .checked_mul(u32::from(height))
        .ok_or(TOO_LARGE)?;
    if u64::from(n) > MAX_PIXELS {
        return Err(TOO_LARGE);
    }
    Ok(n)
}

fn check_table(table: &[[u8; 3]]) -> Result<(), Error> {
    check_table_len(table.len())
}

fn check_table_len(n: usize) -> Result<(), Error> {
    if !(2..=256).contains(&n) || !n.is_power_of_two() {
        return Err(INVALID);
    }
    Ok(())
}

fn size_field(len: usize) -> Result<u8, Error> {
    check_table_len(len)?;
    Ok((len.trailing_zeros() as u8) - 1)
}

fn min_code_size(len: usize) -> Result<u8, Error> {
    check_table_len(len)?;
    let mut bits = 2u8;
    while (1usize << bits) < len && bits < 8 {
        bits += 1;
    }
    Ok(bits)
}

fn write_table(out: &mut Vec<u8>, table: &[[u8; 3]]) {
    for rgb in table {
        out.extend_from_slice(rgb);
    }
}

fn read_table(r: &mut Reader<'_>, size_field: u8) -> Result<Vec<[u8; 3]>, Error> {
    let n = 1usize << (u32::from(size_field) + 1);
    let mut table = Vec::with_capacity(n);
    for _ in 0..n {
        let rgb = r.bytes(3)?;
        table.push([rgb[0], rgb[1], rgb[2]]);
    }
    Ok(table)
}

fn write_subblocks(out: &mut Vec<u8>, data: &[u8]) {
    let mut i = 0;
    while i < data.len() {
        let n = (data.len() - i).min(255);
        out.push(n as u8);
        out.extend_from_slice(&data[i..i + n]);
        i += n;
    }
    out.push(0);
}

fn read_subblocks(r: &mut Reader<'_>) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    loop {
        let n = r.u8()? as usize;
        if n == 0 {
            break;
        }
        if out.len().saturating_add(n) > 32 * 1024 * 1024 {
            return Err(TOO_LARGE);
        }
        out.extend_from_slice(r.bytes(n)?);
    }
    Ok(out)
}

struct Reader<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Reader<'a> {
    fn u8(&mut self) -> Result<u8, Error> {
        if self.i >= self.b.len() {
            return Err(TRUNCATED);
        }
        let v = self.b[self.i];
        self.i += 1;
        Ok(v)
    }

    fn u16(&mut self) -> Result<u16, Error> {
        let lo = self.u8()?;
        let hi = self.u8()?;
        Ok(u16::from_le_bytes([lo, hi]))
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.i.checked_add(n).ok_or(TOO_LARGE)?;
        if end > self.b.len() {
            return Err(TRUNCATED);
        }
        let s = &self.b[self.i..end];
        self.i = end;
        Ok(s)
    }
}
