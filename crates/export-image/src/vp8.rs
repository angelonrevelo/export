//! Lossy VP8, keyframe only.
//!
//! The subset this crate round-trips is a still keyframe: profile 0, one
//! coefficient partition, no segmentation, loop filter off, and every
//! macroblock intra 16×16 DC with chroma DC. Width and height are multiples
//! of 16. A different prediction mode, a filter, segmentation, or another
//! profile is an error. There is no alpha plane in this bitstream.

use crate::error::{Error, INVALID, TOO_LARGE, TRUNCATED, UNSUPPORTED};
use crate::raster::Raster;
use crate::vp8_tab::{AC_Q, COEF_PROBS, COEF_UPDATE_PROBS, DC_Q, NORM};
use crate::MAX_PIXELS;

/// Quantizer index used by [`encode_vp8`]. Not a per-image search.
pub(crate) const VP8_QINDEX: u8 = 8;

const BANDS: [usize; 16] = [0, 1, 2, 3, 6, 4, 5, 6, 6, 6, 6, 6, 6, 6, 6, 7];
const ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];
const MAX_TOKEN: i32 = 67 + ((1 << 11) - 1);

struct BoolWriter {
    low: u32,
    range: u32,
    count: i32,
    buf: Vec<u8>,
}

impl BoolWriter {
    fn new() -> Self {
        Self {
            low: 0,
            range: 255,
            count: -24,
            buf: Vec::new(),
        }
    }

    fn put(&mut self, bit: u32, prob: u32) {
        let mut count = self.count;
        let range = self.range;
        let mut low = self.low;
        let split = 1 + (((range - 1) * prob) >> 8);
        let mut next = split;
        if bit != 0 {
            low = low.wrapping_add(split);
            next = range - split;
        }
        let mut shift = i32::from(NORM[(next as u8) as usize]);
        next <<= shift;
        count += shift;
        if count >= 0 {
            let offset = shift - count;
            if low.wrapping_shl((offset - 1) as u32) & 0x8000_0000 != 0 {
                let mut x = self.buf.len() as i32 - 1;
                while x >= 0 && self.buf[x as usize] == 0xff {
                    self.buf[x as usize] = 0;
                    x -= 1;
                }
                self.buf[x as usize] = self.buf[x as usize].wrapping_add(1);
            }
            self.buf
                .push(((low >> (24 - offset as u32)) & 0xff) as u8);
            shift = count;
            low = (((low as u64) << offset) as u32) & 0x00ff_ffff;
            count -= 8;
        }
        low = low.wrapping_shl(shift as u32);
        self.count = count;
        self.low = low;
        self.range = next;
    }

    fn literal(&mut self, value: u32, bits: u32) {
        for i in (0..bits).rev() {
            self.put((value >> i) & 1, 128);
        }
    }

    fn sign(&mut self, negative: bool) {
        let split = (self.range + 1) >> 1;
        let mut low = self.low;
        let mut range = if negative {
            low = low.wrapping_add(split);
            self.range - split
        } else {
            split
        };
        range <<= 1;
        if low & 0x8000_0000 != 0 {
            let mut x = self.buf.len() as i32 - 1;
            while x >= 0 && self.buf[x as usize] == 0xff {
                self.buf[x as usize] = 0;
                x -= 1;
            }
            self.buf[x as usize] = self.buf[x as usize].wrapping_add(1);
        }
        low = low.wrapping_shl(1);
        self.count += 1;
        if self.count == 0 {
            self.count = -8;
            self.buf.push((low >> 24) as u8);
            low &= 0x00ff_ffff;
        }
        self.low = low;
        self.range = range;
    }

    fn finish(mut self) -> Vec<u8> {
        for _ in 0..32 {
            self.put(0, 128);
        }
        self.buf
    }
}

struct BoolReader<'a> {
    data: &'a [u8],
    pos: usize,
    value: u64,
    count: i32,
    range: u32,
}

impl<'a> BoolReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        let mut r = Self {
            data,
            pos: 0,
            value: 0,
            count: -8,
            range: 255,
        };
        r.fill();
        r
    }

    fn fill(&mut self) {
        let mut value = self.value;
        let mut count = self.count;
        let mut shift = 64 - 8 - (count + 8);
        let bytes_left = self.data.len().saturating_sub(self.pos);
        let bits_left = bytes_left.saturating_mul(8);
        let x = shift + 8 - bits_left as i32;
        let mut loop_end = 0i32;
        if x >= 0 {
            count += 0x4000_0000;
            loop_end = x;
        }
        if x < 0 || bits_left != 0 {
            while shift >= loop_end {
                count += 8;
                let byte = u64::from(self.data[self.pos]);
                value |= byte << shift;
                self.pos += 1;
                shift -= 8;
            }
        }
        self.value = value;
        self.count = count;
    }

    fn bit(&mut self, prob: u32) -> Result<u32, Error> {
        if self.count < 0 {
            self.fill();
        }
        let split = 1 + (((self.range - 1) * prob) >> 8);
        let bigsplit = (split as u64) << (64 - 8);
        let mut range = split;
        let bit = if self.value >= bigsplit {
            range = self.range - split;
            self.value -= bigsplit;
            1
        } else {
            0
        };
        let shift = i32::from(NORM[(range as u8) as usize]);
        range <<= shift;
        self.value <<= shift;
        self.count -= shift;
        self.range = range;
        Ok(bit)
    }

    fn literal(&mut self, bits: u32) -> Result<u32, Error> {
        let mut z = 0u32;
        for i in (0..bits).rev() {
            z |= self.bit(128)? << i;
        }
        Ok(z)
    }

    fn sign(&mut self) -> Result<bool, Error> {
        if self.count < 0 {
            self.fill();
        }
        let split = (self.range + 1) >> 1;
        let bigsplit = (split as u64) << (64 - 8);
        let negative = if self.value < bigsplit {
            self.range = split;
            false
        } else {
            self.range -= split;
            self.value -= bigsplit;
            true
        };
        self.range <<= 1;
        self.value <<= 1;
        self.count -= 1;
        Ok(negative)
    }

    fn delta(&mut self, bits: u32) -> Result<i32, Error> {
        if self.bit(128)? == 0 {
            return Ok(0);
        }
        let mag = self.literal(bits)? as i32;
        if self.bit(128)? == 1 {
            Ok(-mag)
        } else {
            Ok(mag)
        }
    }
}

fn node(table: &[u8], kind: usize, band: usize, ctx: usize, n: usize) -> u32 {
    u32::from(table[((kind * 8 + band) * 3 + ctx) * 11 + n])
}

fn write_abs(w: &mut BoolWriter, p: [u32; 11], abs: u32) {
    if abs == 1 {
        w.put(0, p[2]);
        return;
    }
    w.put(1, p[2]);
    if abs == 2 {
        w.put(0, p[3]);
        w.put(0, p[4]);
        return;
    }
    if abs == 3 || abs == 4 {
        w.put(0, p[3]);
        w.put(1, p[4]);
        w.put(u32::from(abs == 4), p[5]);
        return;
    }
    w.put(1, p[3]);
    if (5..=6).contains(&abs) {
        w.put(0, p[6]);
        w.put(0, p[7]);
        w.put(abs - 5, 159);
        return;
    }
    if (7..=10).contains(&abs) {
        w.put(0, p[6]);
        w.put(1, p[7]);
        let extra = abs - 7;
        w.put((extra >> 1) & 1, 165);
        w.put(extra & 1, 145);
        return;
    }
    w.put(1, p[6]);
    let (cat, base, probs): (u32, u32, &[u8]) = if abs < 19 {
        (0, 11, &[173, 148, 140])
    } else if abs < 35 {
        (1, 19, &[176, 155, 140, 135])
    } else if abs < 67 {
        (2, 35, &[180, 157, 141, 134, 130])
    } else {
        (3, 67, &[254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129])
    };
    let bit1 = cat >> 1;
    w.put(bit1, p[8]);
    w.put(cat & 1, p[9 + bit1 as usize]);
    let extra = abs - base;
    let nbits = probs.len() as u32;
    for i in 0..nbits {
        w.put((extra >> (nbits - 1 - i)) & 1, u32::from(probs[i as usize]));
    }
}

fn read_abs(r: &mut BoolReader<'_>, p: [u32; 11]) -> Result<u32, Error> {
    if r.bit(p[2])? == 0 {
        return Ok(1);
    }
    if r.bit(p[3])? == 0 {
        if r.bit(p[4])? == 0 {
            return Ok(2);
        }
        return Ok(3 + r.bit(p[5])?);
    }
    if r.bit(p[6])? == 0 {
        if r.bit(p[7])? == 0 {
            return Ok(5 + r.bit(159)?);
        }
        let mut v = 7 + 2 * r.bit(165)?;
        v += r.bit(145)?;
        return Ok(v);
    }
    let bit1 = r.bit(p[8])?;
    let bit0 = r.bit(p[9 + bit1 as usize])?;
    let cat = 2 * bit1 + bit0;
    let probs: &[u8] = match cat {
        0 => &[173, 148, 140],
        1 => &[176, 155, 140, 135],
        2 => &[180, 157, 141, 134, 130],
        _ => &[254, 254, 243, 230, 196, 177, 153, 140, 133, 130, 129],
    };
    let mut extra = 0u32;
    for &prob in probs {
        extra = (extra << 1) | r.bit(u32::from(prob))?;
    }
    Ok(extra + 3 + (8 << cat))
}

fn write_block(w: &mut BoolWriter, kind: usize, ctx: usize, start: usize, q: &[i32; 16]) {
    let mut n = start;
    let mut band = BANDS[n];
    let mut context = ctx;
    let later = |from: usize| (from..16).any(|i| q[ZIGZAG[i]] != 0);
    if !later(start) {
        w.put(0, node(&COEF_PROBS, kind, band, context, 0));
        return;
    }
    w.put(1, node(&COEF_PROBS, kind, band, context, 0));
    loop {
        n += 1;
        let coef = q[ZIGZAG[n - 1]];
        let p = [
            node(&COEF_PROBS, kind, band, context, 0),
            node(&COEF_PROBS, kind, band, context, 1),
            node(&COEF_PROBS, kind, band, context, 2),
            node(&COEF_PROBS, kind, band, context, 3),
            node(&COEF_PROBS, kind, band, context, 4),
            node(&COEF_PROBS, kind, band, context, 5),
            node(&COEF_PROBS, kind, band, context, 6),
            node(&COEF_PROBS, kind, band, context, 7),
            node(&COEF_PROBS, kind, band, context, 8),
            node(&COEF_PROBS, kind, band, context, 9),
            node(&COEF_PROBS, kind, band, context, 10),
        ];
        if coef == 0 {
            w.put(0, p[1]);
            if n == 16 {
                return;
            }
            band = BANDS[n];
            context = 0;
            continue;
        }
        w.put(1, p[1]);
        write_abs(w, p, coef.unsigned_abs());
        w.sign(coef < 0);
        if n == 16 {
            return;
        }
        context = if coef.unsigned_abs() == 1 { 1 } else { 2 };
        band = BANDS[n];
        if !later(n) {
            w.put(0, node(&COEF_PROBS, kind, band, context, 0));
            return;
        }
        w.put(1, node(&COEF_PROBS, kind, band, context, 0));
    }
}

fn read_block(
    r: &mut BoolReader<'_>,
    table: &[u8],
    kind: usize,
    ctx: usize,
    start: usize,
    q: &mut [i32; 16],
) -> Result<bool, Error> {
    let mut n = start;
    let mut band = BANDS[n];
    let mut context = ctx;
    if r.bit(node(table, kind, band, context, 0))? == 0 {
        return Ok(false);
    }
    loop {
        n += 1;
        let p = [
            node(table, kind, band, context, 0),
            node(table, kind, band, context, 1),
            node(table, kind, band, context, 2),
            node(table, kind, band, context, 3),
            node(table, kind, band, context, 4),
            node(table, kind, band, context, 5),
            node(table, kind, band, context, 6),
            node(table, kind, band, context, 7),
            node(table, kind, band, context, 8),
            node(table, kind, band, context, 9),
            node(table, kind, band, context, 10),
        ];
        if r.bit(p[1])? == 0 {
            if n == 16 {
                return Ok(true);
            }
            band = BANDS[n];
            context = 0;
            continue;
        }
        let abs = read_abs(r, p)?;
        let value = if r.sign()? { -(abs as i32) } else { abs as i32 };
        q[ZIGZAG[n - 1]] = value;
        if n == 16 {
            return Ok(true);
        }
        context = if abs == 1 { 1 } else { 2 };
        band = BANDS[n];
        if r.bit(node(table, kind, band, context, 0))? == 0 {
            return Ok(true);
        }
    }
}

fn fdct4x4(src: &[i32], stride: usize, out: &mut [i32; 16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let o = i * stride;
        let a1 = (src[o] + src[o + 3]) * 8;
        let b1 = (src[o + 1] + src[o + 2]) * 8;
        let c1 = (src[o + 1] - src[o + 2]) * 8;
        let d1 = (src[o] - src[o + 3]) * 8;
        tmp[i * 4] = a1 + b1;
        tmp[i * 4 + 2] = a1 - b1;
        tmp[i * 4 + 1] = (c1 * 2217 + d1 * 5352 + 14500) >> 12;
        tmp[i * 4 + 3] = (d1 * 2217 - c1 * 5352 + 7500) >> 12;
    }
    for i in 0..4 {
        let a1 = tmp[i] + tmp[12 + i];
        let b1 = tmp[4 + i] + tmp[8 + i];
        let c1 = tmp[4 + i] - tmp[8 + i];
        let d1 = tmp[i] - tmp[12 + i];
        out[i] = (a1 + b1 + 7) >> 4;
        out[8 + i] = (a1 - b1 + 7) >> 4;
        out[4 + i] = ((c1 * 2217 + d1 * 5352 + 12000) >> 16) + i32::from(d1 != 0);
        out[12 + i] = (d1 * 2217 - c1 * 5352 + 51000) >> 16;
    }
}

fn fwalsh(input: &[i32; 16], out: &mut [i32; 16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let ip = i * 4;
        let a1 = (input[ip] + input[ip + 2]) * 4;
        let d1 = (input[ip + 1] + input[ip + 3]) * 4;
        let c1 = (input[ip + 1] - input[ip + 3]) * 4;
        let b1 = (input[ip] - input[ip + 2]) * 4;
        tmp[ip] = a1 + d1 + i32::from(a1 != 0);
        tmp[ip + 1] = b1 + c1;
        tmp[ip + 2] = b1 - c1;
        tmp[ip + 3] = a1 - d1;
    }
    for i in 0..4 {
        let a1 = tmp[i] + tmp[8 + i];
        let d1 = tmp[4 + i] + tmp[12 + i];
        let c1 = tmp[4 + i] - tmp[12 + i];
        let b1 = tmp[i] - tmp[8 + i];
        let mut a2 = a1 + d1;
        let mut b2 = b1 + c1;
        let mut c2 = b1 - c1;
        let mut d2 = a1 - d1;
        if a2 < 0 {
            a2 += 1;
        }
        if b2 < 0 {
            b2 += 1;
        }
        if c2 < 0 {
            c2 += 1;
        }
        if d2 < 0 {
            d2 += 1;
        }
        out[i] = (a2 + 3) >> 3;
        out[4 + i] = (b2 + 3) >> 3;
        out[8 + i] = (c2 + 3) >> 3;
        out[12 + i] = (d2 + 3) >> 3;
    }
}

fn iwalsh(input: &[i32; 16], output: &mut [i32; 16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a1 = input[i] + input[12 + i];
        let b1 = input[4 + i] + input[8 + i];
        let c1 = input[4 + i] - input[8 + i];
        let d1 = input[i] - input[12 + i];
        tmp[i] = a1 + b1;
        tmp[4 + i] = c1 + d1;
        tmp[8 + i] = a1 - b1;
        tmp[12 + i] = d1 - c1;
    }
    for i in 0..4 {
        let row = i * 4;
        let a1 = tmp[row] + tmp[row + 3];
        let b1 = tmp[row + 1] + tmp[row + 2];
        let c1 = tmp[row + 1] - tmp[row + 2];
        let d1 = tmp[row] - tmp[row + 3];
        output[row] = (a1 + b1 + 3) >> 3;
        output[row + 1] = (c1 + d1 + 3) >> 3;
        output[row + 2] = (a1 - b1 + 3) >> 3;
        output[row + 3] = (d1 - c1 + 3) >> 3;
    }
}

fn idct_add(plane: &mut [u8], stride: usize, at: usize, coeffs: &[i32; 16]) {
    let mut tmp = [0i32; 16];
    for i in 0..4 {
        let a1 = coeffs[i] + coeffs[8 + i];
        let b1 = coeffs[i] - coeffs[8 + i];
        let c1 = ((coeffs[4 + i] * 35468) >> 16)
            - (coeffs[12 + i] + ((coeffs[12 + i] * 20091) >> 16));
        let d1 = (coeffs[4 + i] + ((coeffs[4 + i] * 20091) >> 16))
            + ((coeffs[12 + i] * 35468) >> 16);
        tmp[i] = a1 + d1;
        tmp[12 + i] = a1 - d1;
        tmp[4 + i] = b1 + c1;
        tmp[8 + i] = b1 - c1;
    }
    for row in 0..4 {
        let c0 = tmp[row * 4];
        let c1 = tmp[row * 4 + 1];
        let c2 = tmp[row * 4 + 2];
        let c3 = tmp[row * 4 + 3];
        let a1 = c0 + c2;
        let b1 = c0 - c2;
        let d1 = (c1 + ((c1 * 20091) >> 16)) + ((c3 * 35468) >> 16);
        let e1 = ((c1 * 35468) >> 16) - (c3 + ((c3 * 20091) >> 16));
        let base = at + row * stride;
        plane[base] = clip8(i32::from(plane[base]) + ((a1 + d1 + 4) >> 3));
        plane[base + 3] = clip8(i32::from(plane[base + 3]) + ((a1 - d1 + 4) >> 3));
        plane[base + 1] = clip8(i32::from(plane[base + 1]) + ((b1 + e1 + 4) >> 3));
        plane[base + 2] = clip8(i32::from(plane[base + 2]) + ((b1 - e1 + 4) >> 3));
    }
}

fn clip8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

fn quantize(c: i32, step: i32) -> Result<i32, Error> {
    if c == 0 {
        return Ok(0);
    }
    let q = if c > 0 {
        (c + step / 2) / step
    } else {
        -((c.abs() + step / 2) / step)
    };
    if q.abs() > MAX_TOKEN {
        return Err(UNSUPPORTED);
    }
    Ok(q)
}

struct Factors {
    y2_dc: i32,
    y2_ac: i32,
    y1_ac: i32,
    uv_dc: i32,
    uv_ac: i32,
}

fn factors(q: i32, d: [i32; 5]) -> Factors {
    let idx = |v: i32| v.clamp(0, 127) as usize;
    let mut y2_ac = AC_Q[idx(q + d[2])] * 155 / 100;
    if y2_ac < 8 {
        y2_ac = 8;
    }
    let mut uv_dc = DC_Q[idx(q + d[3])];
    if uv_dc > 132 {
        uv_dc = 132;
    }
    Factors {
        y2_dc: DC_Q[idx(q + d[1])] * 2,
        y2_ac,
        y1_ac: AC_Q[idx(q)],
        uv_dc,
        uv_ac: AC_Q[idx(q + d[4])],
    }
}

fn dc_pred(plane: &[u8], stride: usize, x: usize, y: usize, n: usize) -> u8 {
    if x == 0 && y == 0 {
        return 128;
    }
    let shift = n.trailing_zeros();
    if y == 0 {
        let sum = (0..n)
            .map(|row| i32::from(plane[(y + row) * stride + x - 1]))
            .sum::<i32>();
        return ((sum + (1 << (shift - 1))) >> shift) as u8;
    }
    if x == 0 {
        let sum = (0..n)
            .map(|col| i32::from(plane[(y - 1) * stride + x + col]))
            .sum::<i32>();
        return ((sum + (1 << (shift - 1))) >> shift) as u8;
    }
    let left = (0..n)
        .map(|row| i32::from(plane[(y + row) * stride + x - 1]))
        .sum::<i32>();
    let above = (0..n)
        .map(|col| i32::from(plane[(y - 1) * stride + x + col]))
        .sum::<i32>();
    let wide = shift + 1;
    ((left + above + (1 << (wide - 1))) >> wide) as u8
}

fn paint(plane: &mut [u8], stride: usize, x: usize, y: usize, n: usize, value: u8) {
    for row in 0..n {
        for col in 0..n {
            plane[(y + row) * stride + x + col] = value;
        }
    }
}

fn rgb_to_y(r: i32, g: i32, b: i32) -> u8 {
    let luma = 16839 * r + 33059 * g + 6420 * b;
    ((luma + 32768 + (16 << 16)) >> 16) as u8
}

fn clip_uv(uv: i32) -> u8 {
    let v = (uv + 32768 + (128 << 18)) >> 18;
    if v & !0xff == 0 {
        v as u8
    } else if v < 0 {
        0
    } else {
        255
    }
}

fn rgb_to_u(r: i32, g: i32, b: i32) -> u8 {
    clip_uv(-9719 * r - 19081 * g + 28800 * b)
}

fn rgb_to_v(r: i32, g: i32, b: i32) -> u8 {
    clip_uv(28800 * r - 24116 * g - 4684 * b)
}

fn mul_hi(v: i32, coeff: i32) -> i32 {
    (v * coeff) >> 8
}

fn clip_rgb(v: i32) -> u8 {
    const MASK: i32 = (256 << 6) - 1;
    if v & !MASK == 0 {
        (v >> 6) as u8
    } else if v < 0 {
        0
    } else {
        255
    }
}

fn yuv_to_rgb(y: u8, u: u8, v: u8) -> [u8; 3] {
    let y = i32::from(y);
    let u = i32::from(u);
    let v = i32::from(v);
    [
        clip_rgb(mul_hi(y, 19077) + mul_hi(v, 26149) - 14234),
        clip_rgb(mul_hi(y, 19077) - mul_hi(u, 6419) - mul_hi(v, 13320) + 8708),
        clip_rgb(mul_hi(y, 19077) + mul_hi(u, 33050) - 17685),
    ]
}

struct Planes {
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    yw: usize,
    uw: usize,
}

fn sample_rgb(rgb: &Raster) -> Planes {
    let w = rgb.width as usize;
    let h = rgb.height as usize;
    let mut y = vec![0u8; w * h];
    let uw = w / 2;
    let uh = h / 2;
    let mut u = vec![0u8; uw * uh];
    let mut v = vec![0u8; uw * uh];
    for row in 0..h {
        for col in 0..w {
            let p = (row * w + col) * 3;
            y[row * w + col] = rgb_to_y(
                i32::from(rgb.pixels[p]),
                i32::from(rgb.pixels[p + 1]),
                i32::from(rgb.pixels[p + 2]),
            );
        }
    }
    for row in 0..uh {
        for col in 0..uw {
            let mut su = 0i32;
            let mut sv = 0i32;
            for dy in 0..2 {
                for dx in 0..2 {
                    let p = ((row * 2 + dy) * w + col * 2 + dx) * 3;
                    let r = i32::from(rgb.pixels[p]);
                    let g = i32::from(rgb.pixels[p + 1]);
                    let b = i32::from(rgb.pixels[p + 2]);
                    su += i32::from(rgb_to_u(r, g, b));
                    sv += i32::from(rgb_to_v(r, g, b));
                }
            }
            u[row * uw + col] = ((su + 2) >> 2) as u8;
            v[row * uw + col] = ((sv + 2) >> 2) as u8;
        }
    }
    Planes {
        y,
        u,
        v,
        yw: w,
        uw,
    }
}

fn residual_block(src: &[u8], rec: &[u8], stride: usize, x: usize, y: usize) -> [i32; 16] {
    let mut block = [0i32; 16];
    for row in 0..4 {
        for col in 0..4 {
            let i = (y + row) * stride + x + col;
            block[row * 4 + col] = i32::from(src[i]) - i32::from(rec[i]);
        }
    }
    block
}

#[allow(clippy::too_many_arguments)]
fn encode_mb(
    header: &mut BoolWriter,
    tokens: &mut BoolWriter,
    src: &Planes,
    rec: &mut Planes,
    mx: usize,
    my: usize,
    left: &mut [u8; 9],
    above: &mut [u8; 9],
    fq: &Factors,
) -> Result<(), Error> {
    // DC_PRED is 1,0,0 on the keyframe luma tree. Chroma DC is a single 0.
    header.put(1, 145);
    header.put(0, 156);
    header.put(0, 163);
    header.put(0, 142);

    let x = mx * 16;
    let y = my * 16;
    let ux = mx * 8;
    let uy = my * 8;
    let ydc = dc_pred(&rec.y, rec.yw, x, y, 16);
    paint(&mut rec.y, rec.yw, x, y, 16, ydc);
    let udc = dc_pred(&rec.u, rec.uw, ux, uy, 8);
    let vdc = dc_pred(&rec.v, rec.uw, ux, uy, 8);
    paint(&mut rec.u, rec.uw, ux, uy, 8, udc);
    paint(&mut rec.v, rec.uw, ux, uy, 8, vdc);

    let mut dcs = [0i32; 16];
    let mut acs = [[0i32; 16]; 16];
    for i in 0..16 {
        let bx = x + (i % 4) * 4;
        let by = y + (i / 4) * 4;
        let pix = residual_block(&src.y, &rec.y, rec.yw, bx, by);
        let mut freq = [0i32; 16];
        fdct4x4(&pix, 4, &mut freq);
        dcs[i] = freq[0];
        for (k, coef) in freq.iter().enumerate().skip(1) {
            acs[i][k] = quantize(*coef, fq.y1_ac)?;
        }
    }
    let mut wht = [0i32; 16];
    fwalsh(&dcs, &mut wht);
    let mut y2 = [0i32; 16];
    for i in 0..16 {
        let step = if i == 0 { fq.y2_dc } else { fq.y2_ac };
        y2[i] = quantize(wht[i], step)?;
    }
    let mut deq = [0i32; 16];
    for i in 0..16 {
        let step = if i == 0 { fq.y2_dc } else { fq.y2_ac };
        deq[i] = y2[i] * step;
    }
    let mut back = [0i32; 16];
    iwalsh(&deq, &mut back);

    let y2_ctx = usize::from(left[8] + above[8]);
    let y2_nz = write_flag(tokens, 1, y2_ctx, 0, &y2);
    left[8] = u8::from(y2_nz);
    above[8] = u8::from(y2_nz);

    for i in 0..16 {
        let a = i % 4;
        let l = i / 4;
        let ctx = usize::from(left[l] + above[a]);
        let nz = write_flag(tokens, 0, ctx, 1, &acs[i]);
        left[l] = u8::from(nz);
        above[a] = u8::from(nz);
        let mut coeffs = [0i32; 16];
        coeffs[0] = back[i];
        for (k, q) in acs[i].iter().enumerate().skip(1) {
            coeffs[k] = q * fq.y1_ac;
        }
        let bx = x + (i % 4) * 4;
        let by = y + (i / 4) * 4;
        idct_add(&mut rec.y, rec.yw, by * rec.yw + bx, &coeffs);
    }

    encode_chroma(tokens, &src.u, &mut rec.u, rec.uw, ux, uy, left, above, fq, 4)?;
    encode_chroma(tokens, &src.v, &mut rec.v, rec.uw, ux, uy, left, above, fq, 6)?;
    Ok(())
}

fn write_flag(w: &mut BoolWriter, kind: usize, ctx: usize, start: usize, q: &[i32; 16]) -> bool {
    let any = (start..16).any(|i| q[ZIGZAG[i]] != 0);
    write_block(w, kind, ctx, start, q);
    any
}

#[allow(clippy::too_many_arguments)]
fn encode_chroma(
    tokens: &mut BoolWriter,
    src: &[u8],
    rec: &mut [u8],
    stride: usize,
    x: usize,
    y: usize,
    left: &mut [u8; 9],
    above: &mut [u8; 9],
    fq: &Factors,
    base: usize,
) -> Result<(), Error> {
    for i in 0..4 {
        let bx = x + (i % 2) * 4;
        let by = y + (i / 2) * 4;
        let pix = residual_block(src, rec, stride, bx, by);
        let mut freq = [0i32; 16];
        fdct4x4(&pix, 4, &mut freq);
        let mut q = [0i32; 16];
        for k in 0..16 {
            let step = if k == 0 { fq.uv_dc } else { fq.uv_ac };
            q[k] = quantize(freq[k], step)?;
        }
        let above_i = base + (i & 1);
        let left_i = base + usize::from(i >= 2);
        let ctx = usize::from(left[left_i] + above[above_i]);
        let nz = write_flag(tokens, 2, ctx, 0, &q);
        left[left_i] = u8::from(nz);
        above[above_i] = u8::from(nz);
        let mut coeffs = [0i32; 16];
        for k in 0..16 {
            let step = if k == 0 { fq.uv_dc } else { fq.uv_ac };
            coeffs[k] = q[k] * step;
        }
        idct_add(rec, stride, by * stride + bx, &coeffs);
    }
    Ok(())
}

/// VP8 frame, without the RIFF container.
pub(crate) fn encode_vp8(rgb: &Raster) -> Result<Vec<u8>, Error> {
    if rgb.channels != 3 {
        return Err(UNSUPPORTED);
    }
    if rgb.width % 16 != 0 || rgb.height % 16 != 0 || rgb.width > 16383 || rgb.height > 16383 {
        return Err(UNSUPPORTED);
    }
    let mb_cols = (rgb.width / 16) as usize;
    let mb_rows = (rgb.height / 16) as usize;
    let src = sample_rgb(rgb);
    let mut rec = Planes {
        y: vec![0u8; src.y.len()],
        u: vec![0u8; src.u.len()],
        v: vec![0u8; src.v.len()],
        yw: src.yw,
        uw: src.uw,
    };
    let fq = factors(i32::from(VP8_QINDEX), [0; 5]);
    let mut header = BoolWriter::new();
    header.put(0, 128);
    header.put(0, 128);
    header.put(0, 128);
    header.put(0, 128);
    header.literal(0, 6);
    header.literal(0, 3);
    header.put(0, 128);
    header.literal(0, 2);
    header.literal(u32::from(VP8_QINDEX), 7);
    for _ in 0..5 {
        header.put(0, 128);
    }
    header.put(1, 128);
    for &prob in &COEF_UPDATE_PROBS {
        header.put(0, u32::from(prob));
    }
    header.put(0, 128);

    let mut tokens = BoolWriter::new();
    let mut above = vec![[0u8; 9]; mb_cols];
    for my in 0..mb_rows {
        let mut left = [0u8; 9];
        for (mx, above_mb) in above.iter_mut().enumerate() {
            encode_mb(
                &mut header,
                &mut tokens,
                &src,
                &mut rec,
                mx,
                my,
                &mut left,
                above_mb,
                &fq,
            )?;
        }
    }
    let part0 = header.finish();
    let part1 = tokens.finish();
    if part0.len() >= (1 << 19) {
        return Err(TOO_LARGE);
    }
    let mut out = Vec::with_capacity(10 + part0.len() + part1.len());
    let tag = (part0.len() << 5) | (1 << 4);
    out.push((tag & 0xff) as u8);
    out.push(((tag >> 8) & 0xff) as u8);
    out.push(((tag >> 16) & 0xff) as u8);
    out.extend_from_slice(&[0x9d, 0x01, 0x2a]);
    out.extend_from_slice(&(rgb.width as u16).to_le_bytes());
    out.extend_from_slice(&(rgb.height as u16).to_le_bytes());
    out.extend_from_slice(&part0);
    out.extend_from_slice(&part1);
    Ok(out)
}

fn check_dims(width: u32, height: u32) -> Result<(usize, usize), Error> {
    if width == 0 || height == 0 || width > 16383 || height > 16383 {
        return Err(INVALID);
    }
    if width % 16 != 0 || height % 16 != 0 {
        return Err(UNSUPPORTED);
    }
    let pixels = u64::from(width) * u64::from(height);
    if pixels > MAX_PIXELS {
        return Err(TOO_LARGE);
    }
    Ok(((width / 16) as usize, (height / 16) as usize))
}

/// Decode one VP8 keyframe from the subset [`encode_vp8`] writes.
pub(crate) fn decode_vp8(payload: &[u8]) -> Result<Raster, Error> {
    if payload.len() < 10 {
        return Err(TRUNCATED);
    }
    let tag = u32::from(payload[0]) | (u32::from(payload[1]) << 8) | (u32::from(payload[2]) << 16);
    if tag & 1 != 0 || ((tag >> 1) & 7) != 0 {
        return Err(UNSUPPORTED);
    }
    if payload[3..6] != [0x9d, 0x01, 0x2a] {
        return Err(INVALID);
    }
    let width = u32::from(u16::from_le_bytes([payload[6], payload[7]]) & 0x3fff);
    let height = u32::from(u16::from_le_bytes([payload[8], payload[9]]) & 0x3fff);
    if payload[7] >> 6 != 0 || payload[9] >> 6 != 0 {
        return Err(UNSUPPORTED);
    }
    let (mb_cols, mb_rows) = check_dims(width, height)?;
    let part0 = (tag >> 5) as usize;
    if payload.len() < 10 + part0 {
        return Err(TRUNCATED);
    }
    let mut header = BoolReader::new(&payload[10..10 + part0]);
    if header.literal(2)? != 0 {
        return Err(UNSUPPORTED);
    }
    if header.bit(128)? != 0 {
        return Err(UNSUPPORTED);
    }
    let _filter_type = header.bit(128)?;
    let level = header.literal(6)?;
    let _sharp = header.literal(3)?;
    if level != 0 || header.bit(128)? != 0 {
        return Err(UNSUPPORTED);
    }
    if header.literal(2)? != 0 {
        return Err(UNSUPPORTED);
    }
    let q = header.literal(7)? as i32;
    let mut delta = [0i32; 5];
    for slot in &mut delta {
        *slot = header.delta(4)?;
    }
    let _refresh = header.bit(128)?;
    let mut probs = COEF_PROBS;
    for i in 0..probs.len() {
        if header.bit(u32::from(COEF_UPDATE_PROBS[i]))? == 1 {
            probs[i] = header.literal(8)? as u8;
        }
    }
    let skip = header.bit(128)? == 1;
    let skip_prob = if skip { header.literal(8)? } else { 0 };
    let fq = factors(q, delta);

    let yw = mb_cols * 16;
    let uw = mb_cols * 8;
    let mut rec = Planes {
        y: vec![0u8; yw * mb_rows * 16],
        u: vec![0u8; uw * mb_rows * 8],
        v: vec![0u8; uw * mb_rows * 8],
        yw,
        uw,
    };
    let mut tokens = BoolReader::new(&payload[10 + part0..]);
    let mut above = vec![[0u8; 9]; mb_cols];
    for my in 0..mb_rows {
        let mut left = [0u8; 9];
        for (mx, above_mb) in above.iter_mut().enumerate() {
            let mb_skip = skip && header.bit(skip_prob)? == 1;
            if mb_skip {
                left = [0; 9];
                *above_mb = [0; 9];
            }
            decode_mb(
                &mut header,
                &mut tokens,
                &mut rec,
                mx,
                my,
                &mut left,
                above_mb,
                &probs,
                &fq,
                mb_skip,
            )?;
        }
    }
    let mut pixels = Vec::with_capacity(yw * mb_rows * 16 * 3);
    for row in 0..mb_rows * 16 {
        for col in 0..yw {
            let y = rec.y[row * yw + col];
            let u = rec.u[(row / 2) * uw + col / 2];
            let v = rec.v[(row / 2) * uw + col / 2];
            pixels.extend_from_slice(&yuv_to_rgb(y, u, v));
        }
    }
    Raster::new(width, height, 3, pixels)
}

#[allow(clippy::too_many_arguments)]
fn decode_mb(
    header: &mut BoolReader<'_>,
    tokens: &mut BoolReader<'_>,
    rec: &mut Planes,
    mx: usize,
    my: usize,
    left: &mut [u8; 9],
    above: &mut [u8; 9],
    probs: &[u8],
    fq: &Factors,
    skipped: bool,
) -> Result<(), Error> {
    if header.bit(145)? == 0 {
        return Err(UNSUPPORTED);
    }
    if header.bit(156)? != 0 || header.bit(163)? != 0 {
        return Err(UNSUPPORTED);
    }
    if header.bit(142)? != 0 {
        return Err(UNSUPPORTED);
    }
    let x = mx * 16;
    let y = my * 16;
    let ydc = dc_pred(&rec.y, rec.yw, x, y, 16);
    paint(&mut rec.y, rec.yw, x, y, 16, ydc);
    let ux = mx * 8;
    let uy = my * 8;
    let udc = dc_pred(&rec.u, rec.uw, ux, uy, 8);
    let vdc = dc_pred(&rec.v, rec.uw, ux, uy, 8);
    paint(&mut rec.u, rec.uw, ux, uy, 8, udc);
    paint(&mut rec.v, rec.uw, ux, uy, 8, vdc);
    if skipped {
        return Ok(());
    }

    let mut y2 = [0i32; 16];
    let y2_ctx = usize::from(left[8] + above[8]);
    let y2_nz = read_block(tokens, probs, 1, y2_ctx, 0, &mut y2)?;
    left[8] = u8::from(y2_nz);
    above[8] = u8::from(y2_nz);
    let mut deq = [0i32; 16];
    for i in 0..16 {
        let step = if i == 0 { fq.y2_dc } else { fq.y2_ac };
        deq[i] = y2[i] * step;
    }
    let mut back = [0i32; 16];
    iwalsh(&deq, &mut back);

    for (i, dc) in back.iter().enumerate() {
        let a = i % 4;
        let l = i / 4;
        let ctx = usize::from(left[l] + above[a]);
        let mut q = [0i32; 16];
        let nz = read_block(tokens, probs, 0, ctx, 1, &mut q)?;
        left[l] = u8::from(nz);
        above[a] = u8::from(nz);
        let mut coeffs = [0i32; 16];
        coeffs[0] = *dc;
        for (k, quant) in q.iter().enumerate().skip(1) {
            coeffs[k] = quant * fq.y1_ac;
        }
        let bx = x + (i % 4) * 4;
        let by = y + (i / 4) * 4;
        idct_add(&mut rec.y, rec.yw, by * rec.yw + bx, &coeffs);
    }
    decode_chroma(tokens, &mut rec.u, rec.uw, ux, uy, left, above, probs, fq, 4)?;
    decode_chroma(tokens, &mut rec.v, rec.uw, ux, uy, left, above, probs, fq, 6)?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn decode_chroma(
    tokens: &mut BoolReader<'_>,
    plane: &mut [u8],
    stride: usize,
    x: usize,
    y: usize,
    left: &mut [u8; 9],
    above: &mut [u8; 9],
    probs: &[u8],
    fq: &Factors,
    base: usize,
) -> Result<(), Error> {
    for i in 0..4 {
        let above_i = base + (i & 1);
        let left_i = base + usize::from(i >= 2);
        let ctx = usize::from(left[left_i] + above[above_i]);
        let mut q = [0i32; 16];
        let nz = read_block(tokens, probs, 2, ctx, 0, &mut q)?;
        left[left_i] = u8::from(nz);
        above[above_i] = u8::from(nz);
        let mut coeffs = [0i32; 16];
        for k in 0..16 {
            let step = if k == 0 { fq.uv_dc } else { fq.uv_ac };
            coeffs[k] = q[k] * step;
        }
        let bx = x + (i % 2) * 4;
        let by = y + (i / 2) * 4;
        idct_add(plane, stride, by * stride + bx, &coeffs);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bool_coder_round_trips_mixed_bits() {
        let mut w = BoolWriter::new();
        let mut bits = Vec::new();
        let mut state = 0x1234_5678u32;
        for i in 0..200 {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            let bit = state & 1;
            let prob = 1 + ((state >> 8) % 255);
            if i % 17 == 0 {
                w.sign(bit == 1);
            } else {
                w.put(bit, prob);
            }
            bits.push((bit, prob, i % 17 == 0));
        }
        let buf = w.finish();
        let mut r = BoolReader::new(&buf);
        for (bit, prob, sign) in bits {
            let got = if sign { u32::from(r.sign().unwrap()) } else { r.bit(prob).unwrap() };
            assert_eq!(got, bit);
        }
    }

    fn solid(w: u32, h: u32, rgb: [u8; 3]) -> Raster {
        let mut pixels = Vec::new();
        for _ in 0..w * h {
            pixels.extend_from_slice(&rgb);
        }
        Raster::new(w, h, 3, pixels).unwrap()
    }

    fn max_abs(a: &[u8], b: &[u8]) -> i32 {
        a.iter()
            .zip(b)
            .map(|(p, q)| (*p as i32 - *q as i32).abs())
            .max()
            .unwrap()
    }

    #[test]
    fn flat_color_matches_the_bt601_reconstruction() {
        let src = solid(16, 16, [220, 40, 60]);
        let expect = yuv_to_rgb(rgb_to_y(220, 40, 60), rgb_to_u(220, 40, 60), rgb_to_v(220, 40, 60));
        let back = decode_vp8(&encode_vp8(&src).unwrap()).unwrap();
        assert!(back.pixels.chunks(3).all(|p| p == expect));
        assert_ne!(expect, [220, 40, 60]);
    }

    #[test]
    fn two_macroblocks_keep_both_colors() {
        let mut pixels = Vec::new();
        for _y in 0..16 {
            for x in 0..32 {
                let rgb = if x < 16 { [32, 32, 32] } else { [200, 200, 200] };
                pixels.extend_from_slice(&rgb);
            }
        }
        let src = Raster::new(32, 16, 3, pixels).unwrap();
        let back = decode_vp8(&encode_vp8(&src).unwrap()).unwrap();
        assert!(max_abs(&src.pixels[..16 * 3], &back.pixels[..16 * 3]) <= 2);
        let right = 16 * 3;
        assert!(max_abs(&src.pixels[right..right + 12], &back.pixels[right..right + 12]) <= 2);
    }

    #[test]
    fn odd_size_and_non_rgb_are_rejected() {
        let odd = solid(8, 8, [1, 2, 3]);
        assert_eq!(encode_vp8(&odd).unwrap_err(), UNSUPPORTED);
        let rgba = Raster::new(16, 16, 4, vec![0; 16 * 16 * 4]).unwrap();
        assert_eq!(encode_vp8(&rgba).unwrap_err(), UNSUPPORTED);
    }
}
