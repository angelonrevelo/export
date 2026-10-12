//! VP8L, the lossless WebP bitstream.
//!
//! The reader accepts the transforms, prefix codes, LZ77 distances, and color
//! cache described by the WebP lossless specification. The writer emits one
//! prefix-code group and literal pixels only: no predictor, color transform,
//! palette, color cache, or meta-Huffman image. A stream that is not a still
//! VP8L image is rejected by the container parser, not here.

use crate::error::{Error, INVALID, TOO_LARGE, TRUNCATED, UNSUPPORTED};
use crate::raster::Raster;
use crate::MAX_PIXELS;

const DIST_MAP: [(i8, i8); 120] = [
    (0, 1), (1, 0), (1, 1), (-1, 1), (0, 2), (2, 0), (1, 2),
    (-1, 2), (2, 1), (-2, 1), (2, 2), (-2, 2), (0, 3), (3, 0),
    (1, 3), (-1, 3), (3, 1), (-3, 1), (2, 3), (-2, 3), (3, 2),
    (-3, 2), (0, 4), (4, 0), (1, 4), (-1, 4), (4, 1), (-4, 1),
    (3, 3), (-3, 3), (2, 4), (-2, 4), (4, 2), (-4, 2), (0, 5),
    (3, 4), (-3, 4), (4, 3), (-4, 3), (5, 0), (1, 5), (-1, 5),
    (5, 1), (-5, 1), (2, 5), (-2, 5), (5, 2), (-5, 2), (4, 4),
    (-4, 4), (3, 5), (-3, 5), (5, 3), (-5, 3), (0, 6), (6, 0),
    (1, 6), (-1, 6), (6, 1), (-6, 1), (2, 6), (-2, 6), (6, 2),
    (-6, 2), (4, 5), (-4, 5), (5, 4), (-5, 4), (3, 6), (-3, 6),
    (6, 3), (-6, 3), (0, 7), (7, 0), (1, 7), (-1, 7), (5, 5),
    (-5, 5), (7, 1), (-7, 1), (4, 6), (-4, 6), (6, 4), (-6, 4),
    (2, 7), (-2, 7), (7, 2), (-7, 2), (3, 7), (-3, 7), (7, 3),
    (-7, 3), (5, 6), (-5, 6), (6, 5), (-6, 5), (8, 0), (4, 7),
    (-4, 7), (7, 4), (-7, 4), (8, 1), (8, 2), (6, 6), (-6, 6),
    (8, 3), (5, 7), (-5, 7), (7, 5), (-7, 5), (8, 4), (6, 7),
    (-6, 7), (7, 6), (-7, 6), (8, 5), (7, 7), (-7, 7), (8, 6),
    (8, 7),
];

const CODE_LENGTH_ORDER: [usize; 19] = [
    17, 18, 0, 1, 2, 3, 4, 5, 16, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,
];

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn read(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        if n > 24 {
            return Err(INVALID);
        }
        let end = self.pos + n as usize;
        if end > self.data.len().saturating_mul(8) {
            return Err(TRUNCATED);
        }
        let mut value = 0u32;
        for i in 0..n {
            let at = self.pos + i as usize;
            let bit = u32::from((self.data[at / 8] >> (at % 8)) & 1);
            value |= bit << i;
        }
        self.pos = end;
        Ok(value)
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_mul(8).saturating_sub(self.pos)
    }

    /// Next eight bits, first bit in the high position. Past-the-end bits read as zero.
    fn peek8(&self) -> u32 {
        let byte = self.pos / 8;
        let shift = self.pos % 8;
        let b0 = u32::from(self.data.get(byte).copied().unwrap_or(0));
        let b1 = u32::from(self.data.get(byte + 1).copied().unwrap_or(0));
        let raw = ((b0 | (b1 << 8)) >> shift) & 0xff;
        raw.reverse_bits() >> 24
    }

    fn skip(&mut self, n: u32) -> Result<(), Error> {
        let end = self.pos + n as usize;
        if end > self.data.len().saturating_mul(8) {
            return Err(TRUNCATED);
        }
        self.pos = end;
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct LutHit {
    symbol: u16,
    nbits: u8,
}

struct Huff {
    single: Option<u16>,
    mincode: [i32; 16],
    maxcode: [i32; 16],
    valptr: [i32; 16],
    symbols: Vec<u16>,
    /// Codes of 1..=8 bits. `nbits == 0` means the 8-bit window is not a whole code.
    lut: [LutHit; 256],
}

fn empty_lut() -> [LutHit; 256] {
    [LutHit { symbol: 0, nbits: 0 }; 256]
}

fn fill_lut(
    mincode: &[i32; 16],
    maxcode: &[i32; 16],
    valptr: &[i32; 16],
    symbols: &[u16],
) -> [LutHit; 256] {
    let mut lut = empty_lut();
    for window in 0..256u32 {
        let mut code = 0i32;
        for len in 1..=8 {
            let bit = (window >> (8 - len)) & 1;
            code = (code << 1) | bit as i32;
            if maxcode[len] >= 0 && code <= maxcode[len] {
                let idx = valptr[len] + code - mincode[len];
                if idx >= 0 && (idx as usize) < symbols.len() {
                    lut[window as usize] = LutHit {
                        symbol: symbols[idx as usize],
                        nbits: len as u8,
                    };
                }
                break;
            }
        }
    }
    lut
}

impl Huff {
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, Error> {
        if let Some(symbol) = self.single {
            return Ok(symbol);
        }
        if self.symbols.is_empty() {
            return Err(INVALID);
        }
        let hit = self.lut[bits.peek8() as usize];
        if hit.nbits != 0 && bits.remaining() >= usize::from(hit.nbits) {
            bits.skip(u32::from(hit.nbits))?;
            return Ok(hit.symbol);
        }
        let mut code = 0i32;
        for len in 1..16 {
            code = (code << 1) | bits.read(1)? as i32;
            if self.maxcode[len] >= 0 && code <= self.maxcode[len] {
                let idx = self.valptr[len] + code - self.mincode[len];
                if idx < 0 || idx as usize >= self.symbols.len() {
                    return Err(INVALID);
                }
                return Ok(self.symbols[idx as usize]);
            }
        }
        Err(INVALID)
    }
}

fn build_huff(lengths: &[u8]) -> Result<Huff, Error> {
    let mut counts = [0u16; 16];
    let mut used = 0usize;
    let mut only = 0u16;
    for (sym, &len) in lengths.iter().enumerate() {
        if len == 0 {
            continue;
        }
        if len > 15 {
            return Err(INVALID);
        }
        counts[len as usize] += 1;
        used += 1;
        only = sym as u16;
    }
    if used == 0 {
        return Ok(Huff {
            single: None,
            mincode: [0; 16],
            maxcode: [-1; 16],
            valptr: [0; 16],
            symbols: Vec::new(),
            lut: empty_lut(),
        });
    }
    if used == 1 {
        return Ok(Huff {
            single: Some(only),
            mincode: [0; 16],
            maxcode: [-1; 16],
            valptr: [0; 16],
            symbols: Vec::new(),
            lut: empty_lut(),
        });
    }
    let mut kraft = 0u32;
    for (len, count) in counts.iter().enumerate().skip(1) {
        kraft += u32::from(*count) << (15 - len);
    }
    if kraft != 1 << 15 {
        return Err(INVALID);
    }
    let mut next = [0u32; 16];
    let mut code = 0u32;
    for len in 1..16 {
        code = (code + u32::from(counts[len - 1])) << 1;
        next[len] = code;
    }
    let mut symbols = vec![0u16; used];
    let mut mincode = [0i32; 16];
    let mut maxcode = [-1i32; 16];
    let mut valptr = [0i32; 16];
    let mut cursor = [0usize; 16];
    let mut base = 0usize;
    for len in 1..16 {
        valptr[len] = base as i32;
        cursor[len] = base;
        base += counts[len] as usize;
    }
    for (sym, &len) in lengths.iter().enumerate() {
        if len == 0 {
            continue;
        }
        let len = len as usize;
        let assigned = next[len];
        if maxcode[len] < 0 {
            mincode[len] = assigned as i32;
        }
        maxcode[len] = assigned as i32;
        symbols[cursor[len]] = sym as u16;
        cursor[len] += 1;
        next[len] = assigned + 1;
    }
    let lut = fill_lut(&mincode, &maxcode, &valptr, &symbols);
    Ok(Huff {
        single: None,
        mincode,
        maxcode,
        valptr,
        symbols,
        lut,
    })
}

fn read_huff(bits: &mut Bits<'_>, alphabet: usize) -> Result<Huff, Error> {
    let mut lengths = vec![0u8; alphabet];
    if bits.read(1)? == 1 {
        let nsym = bits.read(1)? + 1;
        let wide = bits.read(1)?;
        let first = bits.read(1 + 7 * wide)? as usize;
        if first >= alphabet {
            return Err(INVALID);
        }
        lengths[first] = 1;
        if nsym == 2 {
            let second = bits.read(8)? as usize;
            if second >= alphabet {
                return Err(INVALID);
            }
            lengths[second] = 1;
        }
    } else {
        let nlengths = 4 + bits.read(4)? as usize;
        if nlengths > CODE_LENGTH_ORDER.len() {
            return Err(INVALID);
        }
        let mut code_lengths = [0u8; 19];
        for i in 0..nlengths {
            code_lengths[CODE_LENGTH_ORDER[i]] = bits.read(3)? as u8;
        }
        let coder = build_huff(&code_lengths)?;
        // This counts code-length tokens, not expanded symbols. Repeats are
        // one token. Symbols past the last token stay length 0.
        let mut tokens = if bits.read(1)? == 0 {
            alphabet
        } else {
            let nbits = 2 + 2 * bits.read(3)?;
            let tokens = 2 + bits.read(nbits)? as usize;
            if tokens > alphabet {
                return Err(INVALID);
            }
            tokens
        };
        let mut symbol = 0usize;
        let mut prev = 8u8;
        while symbol < alphabet && tokens > 0 {
            tokens -= 1;
            let code = usize::from(coder.decode(bits)?);
            if code < 16 {
                lengths[symbol] = code as u8;
                if code != 0 {
                    prev = code as u8;
                }
                symbol += 1;
                continue;
            }
            let (extra, offset, value) = match code {
                16 => (2u32, 3usize, prev),
                17 => (3, 3, 0),
                18 => (7, 11, 0),
                _ => return Err(INVALID),
            };
            let repeat = offset + bits.read(extra)? as usize;
            if symbol + repeat > alphabet {
                return Err(INVALID);
            }
            for _ in 0..repeat {
                lengths[symbol] = value;
                symbol += 1;
            }
        }
    }
    build_huff(&lengths)
}

fn prefix_value(bits: &mut Bits<'_>, prefix: u32) -> Result<u32, Error> {
    if prefix < 4 {
        return Ok(prefix + 1);
    }
    let extra = (prefix - 2) >> 1;
    if extra > 18 {
        return Err(INVALID);
    }
    let offset = (2 + (prefix & 1)) << extra;
    Ok(offset + bits.read(extra)? + 1)
}

fn cache_insert(cache: &mut [u32], bits: u32, color: u32) {
    if cache.is_empty() {
        return;
    }
    let shift = 32 - bits;
    let hash = color.wrapping_mul(0x1e35_a7bd) >> shift;
    cache[hash as usize] = color;
}

struct Group {
    green: Huff,
    red: Huff,
    blue: Huff,
    alpha: Huff,
    dist: Huff,
}

fn read_group(bits: &mut Bits<'_>, cache_size: usize) -> Result<Group, Error> {
    Ok(Group {
        green: read_huff(bits, 256 + 24 + cache_size)?,
        red: read_huff(bits, 256)?,
        blue: read_huff(bits, 256)?,
        alpha: read_huff(bits, 256)?,
        dist: read_huff(bits, 40)?,
    })
}

struct Meta {
    bits: u32,
    width: u32,
    codes: Vec<u16>,
}

fn decode_pixels(
    bits: &mut Bits<'_>,
    width: u32,
    height: u32,
    meta_ok: bool,
) -> Result<Vec<u32>, Error> {
    let total = (width as u64)
        .checked_mul(height as u64)
        .ok_or(TOO_LARGE)?;
    if total == 0 || total > MAX_PIXELS {
        return Err(TOO_LARGE);
    }
    let total = total as usize;
    let cache_bits = if bits.read(1)? == 1 {
        let n = bits.read(4)?;
        if !(1..=11).contains(&n) {
            return Err(INVALID);
        }
        n
    } else {
        0
    };
    let cache_size = if cache_bits == 0 {
        0
    } else {
        1usize << cache_bits
    };
    let mut cache = vec![0u32; cache_size];
    let meta = if meta_ok && bits.read(1)? == 1 {
        let prefix_bits = bits.read(3)? + 2;
        let mw = div_up(width, 1 << prefix_bits);
        let mh = div_up(height, 1 << prefix_bits);
        let image = decode_pixels(bits, mw, mh, false)?;
        let mut max_code = 0u16;
        let mut codes = Vec::with_capacity(image.len());
        for pixel in image {
            let code = ((pixel >> 8) & 0xffff) as u16;
            max_code = max_code.max(code);
            codes.push(code);
        }
        let groups = u32::from(max_code) + 1;
        if groups > 64 {
            return Err(UNSUPPORTED);
        }
        Some((
            Meta {
                bits: prefix_bits,
                width: mw,
                codes,
            },
            groups as usize,
        ))
    } else {
        None
    };
    let group_count = meta.as_ref().map(|(_, n)| *n).unwrap_or(1);
    let mut groups = Vec::with_capacity(group_count);
    for _ in 0..group_count {
        groups.push(read_group(bits, cache_size)?);
    }
    let mut pixels = vec![0u32; total];
    let mut written = 0usize;
    while written < total {
        let x = (written % width as usize) as u32;
        let y = (written / width as usize) as u32;
        let group_index = if let Some((meta, _)) = &meta {
            let mx = x >> meta.bits;
            let my = y >> meta.bits;
            let code = meta.codes[(my * meta.width + mx) as usize] as usize;
            if code >= groups.len() {
                return Err(INVALID);
            }
            code
        } else {
            0
        };
        let group = &groups[group_index];
        let symbol = group.green.decode(bits)?;
        if symbol < 256 {
            let red = group.red.decode(bits)?;
            let blue = group.blue.decode(bits)?;
            let alpha = group.alpha.decode(bits)?;
            let color = argb(
                u32::from(alpha),
                u32::from(red),
                u32::from(symbol),
                u32::from(blue),
            );
            pixels[written] = color;
            cache_insert(&mut cache, cache_bits, color);
            written += 1;
        } else if symbol < 280 {
            let length = prefix_value(bits, u32::from(symbol - 256))? as usize;
            if length == 0 || length > 4096 || written + length > total {
                return Err(INVALID);
            }
            let dist_symbol = group.dist.decode(bits)?;
            let distance_code = prefix_value(bits, u32::from(dist_symbol))?;
            let distance = map_distance(distance_code, width)?;
            if distance == 0 || distance > written {
                return Err(INVALID);
            }
            for _ in 0..length {
                let color = pixels[written - distance];
                pixels[written] = color;
                cache_insert(&mut cache, cache_bits, color);
                written += 1;
            }
        } else {
            let index = usize::from(symbol - 280);
            if index >= cache.len() {
                return Err(INVALID);
            }
            let color = cache[index];
            pixels[written] = color;
            cache_insert(&mut cache, cache_bits, color);
            written += 1;
        }
    }
    Ok(pixels)
}

fn map_distance(code: u32, width: u32) -> Result<usize, Error> {
    if code == 0 {
        return Err(INVALID);
    }
    if code > 120 {
        return Ok((code - 120) as usize);
    }
    let (xi, yi) = DIST_MAP[(code - 1) as usize];
    let dist = i64::from(xi) + i64::from(yi) * i64::from(width);
    if dist < 1 {
        Ok(1)
    } else {
        Ok(dist as usize)
    }
}

fn argb(a: u32, r: u32, g: u32, b: u32) -> u32 {
    (a << 24) | (r << 16) | (g << 8) | b
}

fn chan(pixel: u32, shift: u32) -> u8 {
    (pixel >> shift) as u8
}

fn div_up(value: u32, denom: u32) -> u32 {
    value.div_ceil(denom)
}

enum Transform {
    Predictor {
        bits: u32,
        cols: u32,
        modes: Vec<u8>,
    },
    Color {
        bits: u32,
        cols: u32,
        red_to_blue: Vec<u8>,
        green_to_blue: Vec<u8>,
        green_to_red: Vec<u8>,
    },
    SubGreen,
    Index {
        colors: Vec<u32>,
        width_bits: u32,
        out_width: u32,
    },
}

fn decode_sub(bits: &mut Bits<'_>, width: u32, height: u32) -> Result<Vec<u32>, Error> {
    decode_pixels(bits, width, height, false)
}

fn read_transforms(bits: &mut Bits<'_>, width: &mut u32, height: u32) -> Result<Vec<Transform>, Error> {
    let mut seen = [false; 4];
    let mut out = Vec::new();
    while bits.read(1)? == 1 {
        let kind = bits.read(2)? as usize;
        if kind > 3 || seen[kind] {
            return Err(INVALID);
        }
        seen[kind] = true;
        match kind {
            0 => {
                let size_bits = bits.read(3)? + 2;
                let cols = div_up(*width, 1 << size_bits);
                let rows = div_up(height, 1 << size_bits);
                let image = decode_sub(bits, cols, rows)?;
                let modes = image.iter().map(|p| chan(*p, 8)).collect();
                out.push(Transform::Predictor {
                    bits: size_bits,
                    cols,
                    modes,
                });
            }
            1 => {
                let size_bits = bits.read(3)? + 2;
                let cols = div_up(*width, 1 << size_bits);
                let rows = div_up(height, 1 << size_bits);
                let image = decode_sub(bits, cols, rows)?;
                out.push(Transform::Color {
                    bits: size_bits,
                    cols,
                    green_to_red: image.iter().map(|p| chan(*p, 0)).collect(),
                    green_to_blue: image.iter().map(|p| chan(*p, 8)).collect(),
                    red_to_blue: image.iter().map(|p| chan(*p, 16)).collect(),
                });
            }
            2 => out.push(Transform::SubGreen),
            _ => {
                let table_len = bits.read(8)? as usize + 1;
                let mut colors = decode_sub(bits, table_len as u32, 1)?;
                for i in 1..colors.len() {
                    let prev = colors[i - 1];
                    let cur = colors[i];
                    colors[i] = argb(
                        u32::from(chan(cur, 24).wrapping_add(chan(prev, 24))),
                        u32::from(chan(cur, 16).wrapping_add(chan(prev, 16))),
                        u32::from(chan(cur, 8).wrapping_add(chan(prev, 8))),
                        u32::from(chan(cur, 0).wrapping_add(chan(prev, 0))),
                    );
                }
                let width_bits = if table_len <= 2 {
                    3
                } else if table_len <= 4 {
                    2
                } else if table_len <= 16 {
                    1
                } else {
                    0
                };
                let out_width = *width;
                *width = div_up(*width, 1 << width_bits);
                out.push(Transform::Index {
                    colors,
                    width_bits,
                    out_width,
                });
            }
        }
    }
    Ok(out)
}

fn apply_transforms(
    mut pixels: Vec<u32>,
    mut width: u32,
    height: u32,
    transforms: &[Transform],
) -> Result<(Vec<u32>, u32), Error> {
    for transform in transforms.iter().rev() {
        match transform {
            Transform::SubGreen => {
                for pixel in &mut pixels {
                    let g = chan(*pixel, 8);
                    let r = chan(*pixel, 16).wrapping_add(g);
                    let b = chan(*pixel, 0).wrapping_add(g);
                    *pixel = argb(u32::from(chan(*pixel, 24)), u32::from(r), u32::from(g), u32::from(b));
                }
            }
            Transform::Predictor { bits, cols, modes } => {
                let mut next = pixels.clone();
                for y in 0..height {
                    for x in 0..width {
                        let i = (y * width + x) as usize;
                        let mode = modes[((y >> bits) * *cols + (x >> bits)) as usize];
                        let pred = predict(mode, x, y, width, &next)?;
                        next[i] = add_argb(pixels[i], pred);
                    }
                }
                pixels = next;
            }
            Transform::Color {
                bits,
                cols,
                red_to_blue,
                green_to_blue,
                green_to_red,
            } => {
                for y in 0..height {
                    for x in 0..width {
                        let i = (y * width + x) as usize;
                        let block = ((y >> bits) * *cols + (x >> bits)) as usize;
                        let g = chan(pixels[i], 8);
                        let mut r = i32::from(chan(pixels[i], 16));
                        let mut b = i32::from(chan(pixels[i], 0));
                        r += color_delta(green_to_red[block], g);
                        b += color_delta(green_to_blue[block], g);
                        b += color_delta(red_to_blue[block], r as u8);
                        pixels[i] = argb(
                            u32::from(chan(pixels[i], 24)),
                            (r as u32) & 0xff,
                            u32::from(g),
                            (b as u32) & 0xff,
                        );
                    }
                }
            }
            Transform::Index {
                colors,
                width_bits,
                out_width,
            } => {
                let full = *out_width;
                let mut expanded = vec![0u32; full as usize * height as usize];
                let bundle = 1u32 << width_bits;
                for y in 0..height {
                    for x in 0..full {
                        let packed_x = x >> width_bits;
                        let slot = x & (bundle - 1);
                        let src = (y * width + packed_x) as usize;
                        if src >= pixels.len() {
                            return Err(INVALID);
                        }
                        let index = unpack_index(chan(pixels[src], 8), *width_bits, slot);
                        let color = colors.get(index as usize).copied().unwrap_or(0);
                        expanded[(y * full + x) as usize] = color;
                    }
                }
                pixels = expanded;
                width = full;
            }
        }
    }
    Ok((pixels, width))
}

fn unpack_index(green: u8, width_bits: u32, slot: u32) -> u8 {
    if width_bits == 0 {
        return green;
    }
    let bits = 8 >> width_bits;
    let shift = slot * bits;
    (green >> shift) & ((1u8 << bits) - 1)
}

fn color_delta(t: u8, c: u8) -> i32 {
    (i32::from(t as i8) * i32::from(c as i8)) >> 5
}

fn add_argb(residual: u32, pred: u32) -> u32 {
    argb(
        u32::from(chan(residual, 24).wrapping_add(chan(pred, 24))),
        u32::from(chan(residual, 16).wrapping_add(chan(pred, 16))),
        u32::from(chan(residual, 8).wrapping_add(chan(pred, 8))),
        u32::from(chan(residual, 0).wrapping_add(chan(pred, 0))),
    )
}

fn predict(mode: u8, x: u32, y: u32, width: u32, pixels: &[u32]) -> Result<u32, Error> {
    if mode > 13 {
        return Err(INVALID);
    }
    if x == 0 && y == 0 {
        return Ok(0xff00_0000);
    }
    if y == 0 {
        return Ok(pixels[(y * width + x - 1) as usize]);
    }
    if x == 0 {
        return Ok(pixels[((y - 1) * width) as usize]);
    }
    let left = pixels[(y * width + x - 1) as usize];
    let top = pixels[((y - 1) * width + x) as usize];
    let tl = pixels[((y - 1) * width + x - 1) as usize];
    let tr = if x + 1 == width {
        pixels[(y * width) as usize]
    } else {
        pixels[((y - 1) * width + x + 1) as usize]
    };
    let pix = match mode {
        0 => 0xff00_0000,
        1 => left,
        2 => top,
        3 => tr,
        4 => tl,
        5 => avg2(avg2(left, tr), top),
        6 => avg2(left, tl),
        7 => avg2(left, top),
        8 => avg2(tl, top),
        9 => avg2(top, tr),
        10 => avg2(avg2(left, tl), avg2(top, tr)),
        11 => select_pix(left, top, tl),
        12 => clamp_full(left, top, tl),
        _ => clamp_half(avg2(left, top), tl),
    };
    Ok(pix)
}

fn avg2(a: u32, b: u32) -> u32 {
    argb(
        u32::from((u16::from(chan(a, 24)) + u16::from(chan(b, 24))) / 2),
        u32::from((u16::from(chan(a, 16)) + u16::from(chan(b, 16))) / 2),
        u32::from((u16::from(chan(a, 8)) + u16::from(chan(b, 8))) / 2),
        u32::from((u16::from(chan(a, 0)) + u16::from(chan(b, 0))) / 2),
    )
}

fn select_pix(left: u32, top: u32, tl: u32) -> u32 {
    let pa = i32::from(chan(left, 24)) + i32::from(chan(top, 24)) - i32::from(chan(tl, 24));
    let pr = i32::from(chan(left, 16)) + i32::from(chan(top, 16)) - i32::from(chan(tl, 16));
    let pg = i32::from(chan(left, 8)) + i32::from(chan(top, 8)) - i32::from(chan(tl, 8));
    let pb = i32::from(chan(left, 0)) + i32::from(chan(top, 0)) - i32::from(chan(tl, 0));
    let dl = (pa - i32::from(chan(left, 24))).abs()
        + (pr - i32::from(chan(left, 16))).abs()
        + (pg - i32::from(chan(left, 8))).abs()
        + (pb - i32::from(chan(left, 0))).abs();
    let dt = (pa - i32::from(chan(top, 24))).abs()
        + (pr - i32::from(chan(top, 16))).abs()
        + (pg - i32::from(chan(top, 8))).abs()
        + (pb - i32::from(chan(top, 0))).abs();
    if dl < dt {
        left
    } else {
        top
    }
}

fn clamp_full(left: u32, top: u32, tl: u32) -> u32 {
    argb(
        u32::from(clamp_i(i32::from(chan(left, 24)) + i32::from(chan(top, 24)) - i32::from(chan(tl, 24)))),
        u32::from(clamp_i(i32::from(chan(left, 16)) + i32::from(chan(top, 16)) - i32::from(chan(tl, 16)))),
        u32::from(clamp_i(i32::from(chan(left, 8)) + i32::from(chan(top, 8)) - i32::from(chan(tl, 8)))),
        u32::from(clamp_i(i32::from(chan(left, 0)) + i32::from(chan(top, 0)) - i32::from(chan(tl, 0)))),
    )
}

fn clamp_half(avg: u32, tl: u32) -> u32 {
    fn half(a: u8, b: u8) -> u8 {
        let a = i32::from(a);
        let b = i32::from(b);
        clamp_i(a + (a - b) / 2)
    }
    argb(
        u32::from(half(chan(avg, 24), chan(tl, 24))),
        u32::from(half(chan(avg, 16), chan(tl, 16))),
        u32::from(half(chan(avg, 8), chan(tl, 8))),
        u32::from(half(chan(avg, 0), chan(tl, 0))),
    )
}

fn clamp_i(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

pub(crate) fn decode_vp8l(payload: &[u8]) -> Result<Raster, Error> {
    if payload.is_empty() {
        return Err(TRUNCATED);
    }
    let mut bits = Bits::new(payload);
    if bits.read(8)? != 0x2f {
        return Err(INVALID);
    }
    let mut width = bits.read(14)? + 1;
    let height = bits.read(14)? + 1;
    let _alpha_hint = bits.read(1)?;
    if bits.read(3)? != 0 {
        return Err(UNSUPPORTED);
    }
    let pixels_full = u64::from(width) * u64::from(height);
    if pixels_full > MAX_PIXELS {
        return Err(TOO_LARGE);
    }
    let transforms = read_transforms(&mut bits, &mut width, height)?;
    let coded = decode_pixels(&mut bits, width, height, true)?;
    let (pixels, width) = apply_transforms(coded, width, height, &transforms)?;
    raster_from_argb(width, height, &pixels)
}

fn raster_from_argb(width: u32, height: u32, pixels: &[u32]) -> Result<Raster, Error> {
    let opaque = pixels.iter().all(|p| chan(*p, 24) == 255);
    if opaque {
        let mut out = Vec::with_capacity(pixels.len() * 3);
        for p in pixels {
            out.push(chan(*p, 16));
            out.push(chan(*p, 8));
            out.push(chan(*p, 0));
        }
        Raster::new(width, height, 3, out)
    } else {
        let mut out = Vec::with_capacity(pixels.len() * 4);
        for p in pixels {
            out.push(chan(*p, 16));
            out.push(chan(*p, 8));
            out.push(chan(*p, 0));
            out.push(chan(*p, 24));
        }
        Raster::new(width, height, 4, out)
    }
}

struct Writer {
    out: Vec<u8>,
    acc: u8,
    n: u8,
}

impl Writer {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            n: 0,
        }
    }

    fn bit(&mut self, bit: u32) {
        if bit & 1 == 1 {
            self.acc |= 1 << self.n;
        }
        self.n += 1;
        if self.n == 8 {
            self.out.push(self.acc);
            self.acc = 0;
            self.n = 0;
        }
    }

    fn bits(&mut self, value: u32, n: u32) {
        for i in 0..n {
            self.bit((value >> i) & 1);
        }
    }

    fn code(&mut self, value: u32, len: u32) {
        for i in (0..len).rev() {
            self.bit((value >> i) & 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc);
        }
        self.out
    }
}

fn write_flat_channel(w: &mut Writer) {
    // Normal code whose code-length tree is the single symbol "length 8".
    // Every symbol then has length 8 and its canonical code is the symbol.
    w.bit(0);
    w.bits(8, 4);
    for &symbol in CODE_LENGTH_ORDER.iter().take(12) {
        let len = if symbol == 8 { 1 } else { 0 };
        w.bits(len, 3);
    }
    w.bit(0);
}

fn write_green_codes(w: &mut Writer) {
    // Symbols 0..255 have length 8. The 24 length codes stay unused.
    w.bit(0);
    w.bits(8, 4);
    for &sym in CODE_LENGTH_ORDER.iter().take(12) {
        let len = if sym == 8 || sym == 18 { 1 } else { 0 };
        w.bits(len, 3);
    }
    w.bit(0);
    for _ in 0..256 {
        w.code(0, 1);
    }
    w.code(1, 1);
    w.bits(13, 7);
}

fn write_simple(w: &mut Writer, symbol: u32) {
    w.bit(1);
    w.bit(0);
    if symbol < 2 {
        w.bit(0);
        w.bits(symbol, 1);
    } else {
        w.bit(1);
        w.bits(symbol, 8);
    }
}

/// Lossless VP8L payload, not including the RIFF container.
pub(crate) fn encode_vp8l(rgb: &Raster) -> Result<Vec<u8>, Error> {
    if rgb.channels != 3 && rgb.channels != 4 {
        return Err(UNSUPPORTED);
    }
    if rgb.width > 16384 || rgb.height > 16384 {
        return Err(TOO_LARGE);
    }
    let mut w = Writer::new();
    w.bits(0x2f, 8);
    w.bits(rgb.width - 1, 14);
    w.bits(rgb.height - 1, 14);
    let opaque = rgb.channels == 3 || rgb.pixels.chunks(4).all(|p| p[3] == 255);
    w.bit(if opaque { 0 } else { 1 });
    w.bits(0, 3);
    w.bit(0);
    w.bit(0);
    w.bit(0);
    write_green_codes(&mut w);
    write_flat_channel(&mut w);
    write_flat_channel(&mut w);
    if opaque {
        write_simple(&mut w, 255);
    } else {
        write_flat_channel(&mut w);
    }
    write_simple(&mut w, 0);
    let step = rgb.channels as usize;
    for px in rgb.pixels.chunks(step) {
        let g = u32::from(px[1]);
        let r = u32::from(px[0]);
        let b = u32::from(px[2]);
        w.code(g, 8);
        w.code(r, 8);
        w.code(b, 8);
        if !opaque {
            w.code(u32::from(px[3]), 8);
        }
    }
    Ok(w.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distance_map_has_the_near_neighborhood() {
        assert_eq!(DIST_MAP.len(), 120);
        assert_eq!(DIST_MAP[0], (0, 1));
        assert_eq!(DIST_MAP[119], (8, 7));
    }

    #[test]
    fn literal_round_trip_is_exact() {
        let pixels = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 1, 2, 3];
        let src = Raster::new(2, 2, 3, pixels.clone()).unwrap();
        let bytes = encode_vp8l(&src).unwrap();
        let back = decode_vp8l(&bytes).unwrap();
        assert_eq!(back.pixels, pixels);
    }

    #[test]
    fn codes_longer_than_eight_bits_skip_the_lookup() {
        let mut lengths = vec![0u8; 257];
        lengths[0] = 1;
        for length in lengths.iter_mut().skip(1) {
            *length = 9;
        }
        let huff = build_huff(&lengths).unwrap();
        let mut bits = Bits::new(&[0x00]);
        assert_eq!(huff.decode(&mut bits).unwrap(), 0);
        assert_eq!(bits.pos, 1);
        // Canonical code 256 is nine bits: a one, then eight zeros.
        let mut bits = Bits::new(&[0x01, 0x00]);
        assert_eq!(huff.decode(&mut bits).unwrap(), 1);
        assert_eq!(bits.pos, 9);
    }
}
