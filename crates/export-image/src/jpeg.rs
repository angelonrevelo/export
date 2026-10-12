//! Baseline JPEG (ITU-T T.81), 8-bit Huffman, 4:2:0.
//!
//! Decode accepts SOF0 files with three components, 2×2 luma and 1×1 chroma,
//! including files this encoder did not write, as long as the scan uses
//! baseline spectral selection (Ss = 0, Se = 63, Ah = Al = 0) and the Huffman
//! tables carried in the file. Progressive (SOF2), arithmetic (SOF9/SOF10 and
//! the other arithmetic frames), and unknown markers are errors. They are not
//! decoded.
//!
//! [`encode_jpeg`] writes the standard Huffman tables. Shrink uses a second
//! baseline encode, with tables counted from the image, only when those
//! standard tables cannot fit the cap.

use crate::error::{Error, INVALID, TOO_LARGE, TRUNCATED, UNSUPPORTED};
use crate::raster::Raster;
use crate::MAX_PIXELS;

const ZIGZAG: [u8; 64] = [
    0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27, 20,
    13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51, 58, 59,
    52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
];

const QUANT_LUMA: [u8; 64] = [
    16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69, 56,
    14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81, 104, 113,
    92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
];

const QUANT_CHROMA: [u8; 64] = [
    17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99, 99,
    47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
];

const BITS_DC_LUMA: [u8; 16] = [0, 1, 5, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0, 0];
const VALS_DC_LUMA: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const BITS_DC_CHROMA: [u8; 16] = [0, 3, 1, 1, 1, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0];
const VALS_DC_CHROMA: [u8; 12] = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11];
const BITS_AC_LUMA: [u8; 16] = [0, 2, 1, 3, 3, 2, 4, 3, 5, 5, 4, 4, 0, 0, 1, 0x7d];
const VALS_AC_LUMA: [u8; 162] = [
    0x01, 0x02, 0x03, 0x00, 0x04, 0x11, 0x05, 0x12, 0x21, 0x31, 0x41, 0x06, 0x13, 0x51, 0x61, 0x07,
    0x22, 0x71, 0x14, 0x32, 0x81, 0x91, 0xa1, 0x08, 0x23, 0x42, 0xb1, 0xc1, 0x15, 0x52, 0xd1, 0xf0,
    0x24, 0x33, 0x62, 0x72, 0x82, 0x09, 0x0a, 0x16, 0x17, 0x18, 0x19, 0x1a, 0x25, 0x26, 0x27, 0x28,
    0x29, 0x2a, 0x34, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48, 0x49,
    0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68, 0x69,
    0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89,
    0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
    0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3, 0xc4, 0xc5,
    0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda, 0xe1, 0xe2,
    0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];
const BITS_AC_CHROMA: [u8; 16] = [0, 2, 1, 2, 4, 4, 3, 4, 7, 5, 4, 4, 0, 1, 2, 0x77];
const VALS_AC_CHROMA: [u8; 162] = [
    0x00, 0x01, 0x02, 0x03, 0x11, 0x04, 0x05, 0x21, 0x31, 0x06, 0x12, 0x41, 0x51, 0x07, 0x61, 0x71,
    0x13, 0x22, 0x32, 0x81, 0x08, 0x14, 0x42, 0x91, 0xa1, 0xb1, 0xc1, 0x09, 0x23, 0x33, 0x52, 0xf0,
    0x15, 0x62, 0x72, 0xd1, 0x0a, 0x16, 0x24, 0x34, 0xe1, 0x25, 0xf1, 0x17, 0x18, 0x19, 0x1a, 0x26,
    0x27, 0x28, 0x29, 0x2a, 0x35, 0x36, 0x37, 0x38, 0x39, 0x3a, 0x43, 0x44, 0x45, 0x46, 0x47, 0x48,
    0x49, 0x4a, 0x53, 0x54, 0x55, 0x56, 0x57, 0x58, 0x59, 0x5a, 0x63, 0x64, 0x65, 0x66, 0x67, 0x68,
    0x69, 0x6a, 0x73, 0x74, 0x75, 0x76, 0x77, 0x78, 0x79, 0x7a, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87,
    0x88, 0x89, 0x8a, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0xa2, 0xa3, 0xa4, 0xa5,
    0xa6, 0xa7, 0xa8, 0xa9, 0xaa, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7, 0xb8, 0xb9, 0xba, 0xc2, 0xc3,
    0xc4, 0xc5, 0xc6, 0xc7, 0xc8, 0xc9, 0xca, 0xd2, 0xd3, 0xd4, 0xd5, 0xd6, 0xd7, 0xd8, 0xd9, 0xda,
    0xe2, 0xe3, 0xe4, 0xe5, 0xe6, 0xe7, 0xe8, 0xe9, 0xea, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8,
    0xf9, 0xfa,
];

struct HuffEnc {
    code: [u16; 256],
    len: [u8; 256],
}

struct HuffDec {
    mincode: [i32; 18],
    maxcode: [i32; 18],
    valptr: [i32; 18],
    vals: [u8; 256],
    nvals: usize,
}

fn build_huff(bits: &[u8; 16], vals: &[u8]) -> (HuffEnc, HuffDec) {
    let mut enc = HuffEnc {
        code: [0; 256],
        len: [0; 256],
    };
    let mut dec = HuffDec {
        mincode: [0; 18],
        maxcode: [-1; 18],
        valptr: [0; 18],
        vals: [0; 256],
        nvals: vals.len(),
    };
    dec.vals[..vals.len()].copy_from_slice(vals);
    let mut code: u32 = 0;
    let mut k = 0usize;
    for len in 1..=16 {
        let count = bits[len - 1] as usize;
        if count > 0 {
            dec.valptr[len] = k as i32;
            dec.mincode[len] = code as i32;
        }
        for _ in 0..count {
            let sym = vals[k] as usize;
            enc.code[sym] = code as u16;
            enc.len[sym] = len as u8;
            k += 1;
            code += 1;
        }
        if count > 0 {
            dec.maxcode[len] = code as i32 - 1;
        }
        code <<= 1;
    }
    debug_assert_eq!(k, vals.len());
    (enc, dec)
}

fn luma_dc() -> &'static (HuffEnc, HuffDec) {
    use std::sync::OnceLock;
    static T: OnceLock<(HuffEnc, HuffDec)> = OnceLock::new();
    T.get_or_init(|| build_huff(&BITS_DC_LUMA, &VALS_DC_LUMA))
}
fn chroma_dc() -> &'static (HuffEnc, HuffDec) {
    use std::sync::OnceLock;
    static T: OnceLock<(HuffEnc, HuffDec)> = OnceLock::new();
    T.get_or_init(|| build_huff(&BITS_DC_CHROMA, &VALS_DC_CHROMA))
}
fn luma_ac() -> &'static (HuffEnc, HuffDec) {
    use std::sync::OnceLock;
    static T: OnceLock<(HuffEnc, HuffDec)> = OnceLock::new();
    T.get_or_init(|| build_huff(&BITS_AC_LUMA, &VALS_AC_LUMA))
}
fn chroma_ac() -> &'static (HuffEnc, HuffDec) {
    use std::sync::OnceLock;
    static T: OnceLock<(HuffEnc, HuffDec)> = OnceLock::new();
    T.get_or_init(|| build_huff(&BITS_AC_CHROMA, &VALS_AC_CHROMA))
}

fn scale_quant(base: &[u8; 64], quality: u8) -> [u8; 64] {
    let q = u32::from(quality.clamp(1, 100));
    let scale = if q < 50 { 5000 / q } else { 200 - q * 2 };
    let mut out = [0u8; 64];
    for i in 0..64 {
        let v = (u32::from(base[i]) * scale + 50) / 100;
        out[i] = v.clamp(1, 255) as u8;
    }
    out
}

fn fdct(spatial: &[f32; 64], freq: &mut [f32; 64]) {
    let mut tmp = [0f32; 64];
    for y in 0..8 {
        for u in 0..8 {
            let mut s = 0f32;
            for x in 0..8 {
                let ang = (2.0 * x as f32 + 1.0) * u as f32 * std::f32::consts::PI / 16.0;
                s += spatial[y * 8 + x] * ang.cos();
            }
            let cu = if u == 0 {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            tmp[y * 8 + u] = s * cu * 0.5;
        }
    }
    for u in 0..8 {
        for v in 0..8 {
            let mut s = 0f32;
            for y in 0..8 {
                let ang = (2.0 * y as f32 + 1.0) * v as f32 * std::f32::consts::PI / 16.0;
                s += tmp[y * 8 + u] * ang.cos();
            }
            let cv = if v == 0 {
                std::f32::consts::FRAC_1_SQRT_2
            } else {
                1.0
            };
            freq[v * 8 + u] = s * cv * 0.5;
        }
    }
}

fn idct(block: &mut [f32; 64]) {
    let mut tmp = [0f32; 64];
    for y in 0..8 {
        for x in 0..8 {
            let mut s = 0f32;
            for u in 0..8 {
                let cu = if u == 0 {
                    std::f32::consts::FRAC_1_SQRT_2
                } else {
                    1.0
                };
                let ang = (2.0 * x as f32 + 1.0) * u as f32 * std::f32::consts::PI / 16.0;
                s += cu * block[y * 8 + u] * ang.cos();
            }
            tmp[y * 8 + x] = s * 0.5;
        }
    }
    for x in 0..8 {
        for y in 0..8 {
            let mut s = 0f32;
            for v in 0..8 {
                let cv = if v == 0 {
                    std::f32::consts::FRAC_1_SQRT_2
                } else {
                    1.0
                };
                let ang = (2.0 * y as f32 + 1.0) * v as f32 * std::f32::consts::PI / 16.0;
                s += cv * tmp[v * 8 + x] * ang.cos();
            }
            block[y * 8 + x] = s * 0.5;
        }
    }
}

fn ycbcr(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let r = f32::from(r);
    let g = f32::from(g);
    let b = f32::from(b);
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    let cb = -0.168736 * r - 0.331264 * g + 0.5 * b + 128.0;
    let cr = 0.5 * r - 0.418688 * g - 0.081312 * b + 128.0;
    (y, cb, cr)
}

fn rgb_from(y: f32, cb: f32, cr: f32) -> (u8, u8, u8) {
    let cb = cb - 128.0;
    let cr = cr - 128.0;
    let r = y + 1.402 * cr;
    let g = y - 0.344136 * cb - 0.714136 * cr;
    let b = y + 1.772 * cb;
    (clamp_u8(r), clamp_u8(g), clamp_u8(b))
}

fn clamp_u8(v: f32) -> u8 {
    let r = v.round();
    if r <= 0.0 {
        0
    } else if r >= 255.0 {
        255
    } else {
        r as u8
    }
}

fn amplitude(v: i32) -> (u8, u32) {
    if v == 0 {
        return (0, 0);
    }
    let abs = v.unsigned_abs();
    let cat = (32 - abs.leading_zeros()) as u8;
    let mag = if v < 0 {
        abs ^ ((1u32 << cat) - 1)
    } else {
        abs
    };
    (cat, mag)
}

struct Writer {
    out: Vec<u8>,
    acc: u64,
    n: u32,
    limit: usize,
    exceeded: bool,
}

impl Writer {
    fn put(&mut self, bits: u32, len: u8) {
        if self.exceeded || len == 0 {
            return;
        }
        let mask = if len == 32 {
            u32::MAX
        } else {
            (1u32 << len) - 1
        };
        self.acc = (self.acc << len) | u64::from(bits & mask);
        self.n += u32::from(len);
        while self.n >= 8 {
            self.n -= 8;
            let byte = (self.acc >> self.n) as u8;
            self.out.push(byte);
            if byte == 0xff {
                self.out.push(0x00);
            }
            if self.out.len() > self.limit {
                self.exceeded = true;
                return;
            }
        }
    }

    fn symbol(&mut self, enc: &HuffEnc, sym: u8) {
        let len = enc.len[sym as usize];
        self.put(u32::from(enc.code[sym as usize]), len);
    }

    fn finish(&mut self) {
        if self.n > 0 && !self.exceeded {
            let pad = (8 - self.n) as u8;
            let ones = (1u32 << pad) - 1;
            self.put(ones, pad);
        }
    }
}

fn encode_block(
    spatial: &[f32; 64],
    quant: &[u8; 64],
    dc_tab: &HuffEnc,
    ac_tab: &HuffEnc,
    prev_dc: &mut i32,
    w: &mut Writer,
) {
    let mut freq = [0f32; 64];
    fdct(spatial, &mut freq);
    let mut zz = [0i32; 64];
    for k in 0..64 {
        let nat = ZIGZAG[k] as usize;
        zz[k] = (freq[nat] / f32::from(quant[nat])).round() as i32;
    }
    let diff = zz[0] - *prev_dc;
    *prev_dc = zz[0];
    let (cat, mag) = amplitude(diff);
    w.symbol(dc_tab, cat);
    w.put(mag, cat);
    let mut k = 1;
    while k < 64 {
        let mut run = 0u8;
        while k < 64 && zz[k] == 0 {
            run += 1;
            k += 1;
        }
        if k == 64 {
            w.symbol(ac_tab, 0x00);
            break;
        }
        while run >= 16 {
            w.symbol(ac_tab, 0xf0);
            run -= 16;
        }
        let (cat, mag) = amplitude(zz[k]);
        w.symbol(ac_tab, (run << 4) | cat);
        w.put(mag, cat);
        k += 1;
    }
}

pub(crate) enum Limited {
    Bytes(Vec<u8>),
    Exceeded,
}

pub fn encode_jpeg(rgb: &Raster, quality: u8) -> Result<Vec<u8>, Error> {
    match encode_jpeg_limited(rgb, quality, usize::MAX)? {
        Limited::Bytes(buf) => Ok(buf),
        Limited::Exceeded => Err(INVALID),
    }
}

pub(crate) fn encode_jpeg_limited(
    rgb: &Raster,
    quality: u8,
    limit: usize,
) -> Result<Limited, Error> {
    if rgb.channels != 3 || quality == 0 {
        return Err(INVALID);
    }
    let qy = scale_quant(&QUANT_LUMA, quality);
    let qc = scale_quant(&QUANT_CHROMA, quality);
    let (dc_y, _) = luma_dc();
    let (dc_c, _) = chroma_dc();
    let (ac_y, _) = luma_ac();
    let (ac_c, _) = chroma_ac();

    let mut out = Vec::with_capacity(1024);
    out.extend_from_slice(&[0xff, 0xd8]);
    out.extend_from_slice(&[0xff, 0xe0, 0x00, 0x10]);
    out.extend_from_slice(b"JFIF\0");
    out.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
    write_dqt(&mut out, 0, &qy);
    write_dqt(&mut out, 1, &qc);
    out.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
    out.extend_from_slice(&(rgb.height as u16).to_be_bytes());
    out.extend_from_slice(&(rgb.width as u16).to_be_bytes());
    out.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
    write_dht(&mut out, 0x00, &BITS_DC_LUMA, &VALS_DC_LUMA);
    write_dht(&mut out, 0x10, &BITS_AC_LUMA, &VALS_AC_LUMA);
    write_dht(&mut out, 0x01, &BITS_DC_CHROMA, &VALS_DC_CHROMA);
    write_dht(&mut out, 0x11, &BITS_AC_CHROMA, &VALS_AC_CHROMA);
    out.extend_from_slice(&[
        0xff, 0xda, 0x00, 0x0c, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00,
    ]);
    if out.len() >= limit {
        return Ok(Limited::Exceeded);
    }
    let mut w = Writer {
        out: Vec::new(),
        acc: 0,
        n: 0,
        limit: limit - out.len(),
        exceeded: false,
    };
    let mcus_x = rgb.width.div_ceil(16);
    let mcus_y = rgb.height.div_ceil(16);
    let mut dc = [0i32; 3];
    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            for by in 0..2 {
                for bx in 0..2 {
                    let mut block = [0f32; 64];
                    let x0 = mx * 16 + bx * 8;
                    let y0 = my * 16 + by * 8;
                    for ly in 0..8 {
                        for lx in 0..8 {
                            let (r, g, b) = sample(rgb, x0 + lx, y0 + ly);
                            let (y, _, _) = ycbcr(r, g, b);
                            block[(ly * 8 + lx) as usize] = y - 128.0;
                        }
                    }
                    encode_block(&block, &qy, dc_y, ac_y, &mut dc[0], &mut w);
                }
            }
            let mut cb_b = [0f32; 64];
            let mut cr_b = [0f32; 64];
            let x0 = mx * 16;
            let y0 = my * 16;
            for cy in 0..8u32 {
                for cx in 0..8u32 {
                    let mut cb = 0f32;
                    let mut cr = 0f32;
                    for dy in 0..2u32 {
                        for dx in 0..2u32 {
                            let (r, g, b) = sample(rgb, x0 + cx * 2 + dx, y0 + cy * 2 + dy);
                            let (_, cbb, crr) = ycbcr(r, g, b);
                            cb += cbb;
                            cr += crr;
                        }
                    }
                    cb_b[(cy * 8 + cx) as usize] = cb / 4.0 - 128.0;
                    cr_b[(cy * 8 + cx) as usize] = cr / 4.0 - 128.0;
                }
            }
            encode_block(&cb_b, &qc, dc_c, ac_c, &mut dc[1], &mut w);
            encode_block(&cr_b, &qc, dc_c, ac_c, &mut dc[2], &mut w);
            if w.exceeded {
                return Ok(Limited::Exceeded);
            }
        }
    }
    w.finish();
    if w.exceeded {
        return Ok(Limited::Exceeded);
    }
    out.extend_from_slice(&w.out);
    out.extend_from_slice(&[0xff, 0xd9]);
    if out.len() > limit {
        return Ok(Limited::Exceeded);
    }
    Ok(Limited::Bytes(out))
}

fn sample(rgb: &Raster, x: u32, y: u32) -> (u8, u8, u8) {
    let x = x.min(rgb.width - 1);
    let y = y.min(rgb.height - 1);
    let i = ((y as usize * rgb.width as usize) + x as usize) * 3;
    (rgb.pixels[i], rgb.pixels[i + 1], rgb.pixels[i + 2])
}

fn write_dqt(out: &mut Vec<u8>, id: u8, natural: &[u8; 64]) {
    out.extend_from_slice(&[0xff, 0xdb, 0x00, 0x43, id]);
    for k in 0..64 {
        out.push(natural[ZIGZAG[k] as usize]);
    }
}

fn write_dht(out: &mut Vec<u8>, class_id: u8, bits: &[u8; 16], vals: &[u8]) {
    let len = (2 + 1 + 16 + vals.len()) as u16;
    out.extend_from_slice(&[0xff, 0xc4]);
    out.extend_from_slice(&len.to_be_bytes());
    out.push(class_id);
    out.extend_from_slice(bits);
    out.extend_from_slice(vals);
}

struct Reader<'a> {
    data: &'a [u8],
    i: usize,
    acc: u32,
    n: u32,
}

impl Reader<'_> {
    fn pull(&mut self) -> Result<Option<u8>, Error> {
        if self.i >= self.data.len() {
            return Ok(None);
        }
        let b = self.data[self.i];
        if b == 0xff {
            if self.i + 1 >= self.data.len() {
                return Err(TRUNCATED);
            }
            let next = self.data[self.i + 1];
            if next == 0x00 {
                self.i += 2;
                return Ok(Some(0xff));
            }
            return Ok(None);
        }
        self.i += 1;
        Ok(Some(b))
    }

    fn ensure(&mut self) -> Result<(), Error> {
        while self.n <= 24 {
            match self.pull()? {
                Some(b) => {
                    self.acc = (self.acc << 8) | u32::from(b);
                    self.n += 8;
                }
                None => break,
            }
        }
        Ok(())
    }

    fn bit(&mut self) -> Result<u32, Error> {
        if self.n == 0 {
            self.ensure()?;
            if self.n == 0 {
                return Err(TRUNCATED);
            }
        }
        self.n -= 1;
        Ok((self.acc >> self.n) & 1)
    }

    fn bits(&mut self, n: u8) -> Result<u32, Error> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | self.bit()?;
        }
        Ok(v)
    }

    fn symbol(&mut self, dec: &HuffDec) -> Result<u8, Error> {
        let mut code = 0i32;
        for len in 1..=16 {
            code = (code << 1) | self.bit()? as i32;
            if code <= dec.maxcode[len] {
                let idx = dec.valptr[len] + code - dec.mincode[len];
                if idx < 0 || idx as usize >= dec.nvals {
                    return Err(INVALID);
                }
                return Ok(dec.vals[idx as usize]);
            }
        }
        Err(INVALID)
    }
}

fn extend(v: u32, cat: u8) -> i32 {
    if cat == 0 {
        return 0;
    }
    let vt = 1u32 << (cat - 1);
    if v < vt {
        v as i32 - ((1i32 << cat) - 1)
    } else {
        v as i32
    }
}

fn decode_block(
    r: &mut Reader<'_>,
    dc_tab: &HuffDec,
    ac_tab: &HuffDec,
    quant: &[u8; 64],
    prev: &mut i32,
) -> Result<[u8; 64], Error> {
    let mut zz = [0i32; 64];
    let cat = r.symbol(dc_tab)?;
    let diff = extend(r.bits(cat)?, cat);
    let dc = *prev + diff;
    *prev = dc;
    zz[0] = dc;
    let mut k = 1usize;
    while k < 64 {
        let s = r.symbol(ac_tab)?;
        if s == 0 {
            break;
        }
        if s == 0xf0 {
            k += 16;
            continue;
        }
        let run = (s >> 4) as usize;
        let cat = s & 0x0f;
        k += run;
        if k >= 64 || cat == 0 {
            return Err(INVALID);
        }
        zz[k] = extend(r.bits(cat)?, cat);
        k += 1;
    }
    let mut freq = [0f32; 64];
    for k in 0..64 {
        let nat = ZIGZAG[k] as usize;
        freq[nat] = zz[k] as f32 * f32::from(quant[nat]);
    }
    idct(&mut freq);
    let mut out = [0u8; 64];
    for i in 0..64 {
        out[i] = clamp_u8(freq[i] + 128.0);
    }
    Ok(out)
}

pub fn decode_jpeg(bytes: &[u8]) -> Result<Raster, Error> {
    if bytes.len() < 4 || bytes[0] != 0xff || bytes[1] != 0xd8 {
        return Err(INVALID);
    }
    let mut i = 2usize;
    let mut quant = [[0u8; 64]; 4];
    let mut quant_set = [false; 4];
    let mut huff_dc: [Option<HuffDec>; 4] = [None, None, None, None];
    let mut huff_ac: [Option<HuffDec>; 4] = [None, None, None, None];
    let mut width = 0u32;
    let mut height = 0u32;
    let mut comps: Vec<Comp> = Vec::new();
    loop {
        let marker = read_marker(bytes, &mut i)?;
        if marker == 0xd9 {
            return Err(INVALID);
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        // Progressive, extended, lossless, and arithmetic frames are rejected
        // before the segment length is read. They are not decoded.
        if unsupported_frame(marker) {
            return Err(UNSUPPORTED);
        }
        if i + 2 > bytes.len() {
            return Err(TRUNCATED);
        }
        let seglen = u16::from_be_bytes([bytes[i], bytes[i + 1]]) as usize;
        if seglen < 2 || i + seglen > bytes.len() {
            return Err(TRUNCATED);
        }
        let payload = &bytes[i + 2..i + seglen];
        i += seglen;
        match marker {
            0xc0 => {
                if payload.len() < 6 {
                    return Err(INVALID);
                }
                if payload[0] != 8 {
                    return Err(UNSUPPORTED);
                }
                height = u16::from_be_bytes([payload[1], payload[2]]) as u32;
                width = u16::from_be_bytes([payload[3], payload[4]]) as u32;
                let n = payload[5] as usize;
                if n != 3 || payload.len() < 6 + n * 3 || width == 0 || height == 0 {
                    return Err(if n == 3 { INVALID } else { UNSUPPORTED });
                }
                let pixels = (width as u64).checked_mul(height as u64).ok_or(TOO_LARGE)?;
                if pixels > MAX_PIXELS {
                    return Err(TOO_LARGE);
                }
                comps.clear();
                for c in 0..n {
                    let p = 6 + c * 3;
                    let hv = payload[p + 1];
                    comps.push(Comp {
                        id: payload[p],
                        h: hv >> 4,
                        v: hv & 0x0f,
                        tq: payload[p + 2],
                        dc: 0,
                        ac: 0,
                    });
                }
                if comps[0].h != 2
                    || comps[0].v != 2
                    || comps[1].h != 1
                    || comps[1].v != 1
                    || comps[2].h != 1
                    || comps[2].v != 1
                {
                    return Err(UNSUPPORTED);
                }
            }
            0xdb => {
                let mut p = 0;
                while p < payload.len() {
                    let info = payload[p];
                    p += 1;
                    if info >> 4 != 0 {
                        return Err(UNSUPPORTED);
                    }
                    let id = (info & 0x0f) as usize;
                    if id > 3 || p + 64 > payload.len() {
                        return Err(INVALID);
                    }
                    for k in 0..64 {
                        quant[id][ZIGZAG[k] as usize] = payload[p + k];
                    }
                    quant_set[id] = true;
                    p += 64;
                }
            }
            0xc4 => {
                let mut p = 0;
                while p < payload.len() {
                    let class_id = payload[p];
                    p += 1;
                    if p + 16 > payload.len() {
                        return Err(TRUNCATED);
                    }
                    let mut bits = [0u8; 16];
                    bits.copy_from_slice(&payload[p..p + 16]);
                    p += 16;
                    let n: usize = bits.iter().map(|&c| c as usize).sum();
                    if p + n > payload.len() {
                        return Err(TRUNCATED);
                    }
                    let vals = &payload[p..p + n];
                    p += n;
                    let (_, dec) = build_huff(&bits, vals);
                    let id = (class_id & 0x0f) as usize;
                    if id > 3 {
                        return Err(INVALID);
                    }
                    if class_id >> 4 == 0 {
                        huff_dc[id] = Some(dec);
                    } else {
                        huff_ac[id] = Some(dec);
                    }
                }
            }
            0xda => {
                bind_scan(&mut comps, payload)?;
                break;
            }
            0xdd => {
                if payload != [0, 0] {
                    return Err(UNSUPPORTED);
                }
            }
            0xe0..=0xef | 0xfe => {}
            _ => return Err(UNSUPPORTED),
        }
    }
    if comps.len() != 3 || width == 0 {
        return Err(INVALID);
    }
    // `i` currently points at the first entropy byte: the SOS header was consumed
    // in the match arm via `break` after `i += seglen`. Re-read: the loop adds
    // seglen before match, then SOS breaks. So `i` is the start of entropy. Good.
    let scan = &bytes[i..];
    let mut reader = Reader {
        data: scan,
        i: 0,
        acc: 0,
        n: 0,
    };
    let mut pixels = vec![0u8; width as usize * height as usize * 3];
    let mcus_x = width.div_ceil(16);
    let mcus_y = height.div_ceil(16);
    let mut prev = [0i32; 3];
    for comp in &comps {
        let tq = usize::from(comp.tq);
        if tq > 3 || !quant_set[tq] {
            return Err(INVALID);
        }
    }
    let qy = &quant[usize::from(comps[0].tq)];
    let qc1 = &quant[usize::from(comps[1].tq)];
    let qc2 = &quant[usize::from(comps[2].tq)];
    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            let mut yplane = [0u8; 256];
            for by in 0..2u32 {
                for bx in 0..2u32 {
                    let block = decode_block(
                        &mut reader,
                        table(&huff_dc, usize::from(comps[0].dc))?,
                        table(&huff_ac, usize::from(comps[0].ac))?,
                        qy,
                        &mut prev[0],
                    )?;
                    for ly in 0..8 {
                        for lx in 0..8 {
                            yplane[((by * 8 + ly) * 16 + bx * 8 + lx) as usize] =
                                block[(ly * 8 + lx) as usize];
                        }
                    }
                }
            }
            let cb = decode_block(
                &mut reader,
                table(&huff_dc, usize::from(comps[1].dc))?,
                table(&huff_ac, usize::from(comps[1].ac))?,
                qc1,
                &mut prev[1],
            )?;
            let cr = decode_block(
                &mut reader,
                table(&huff_dc, usize::from(comps[2].dc))?,
                table(&huff_ac, usize::from(comps[2].ac))?,
                qc2,
                &mut prev[2],
            )?;
            let x0 = mx * 16;
            let y0 = my * 16;
            for py in 0..16u32 {
                for px in 0..16u32 {
                    let x = x0 + px;
                    let y = y0 + py;
                    if x >= width || y >= height {
                        continue;
                    }
                    let ys = f32::from(yplane[(py * 16 + px) as usize]);
                    let ci = ((py / 2) * 8 + px / 2) as usize;
                    let (r, g, b) = rgb_from(ys, f32::from(cb[ci]), f32::from(cr[ci]));
                    let o = ((y as usize * width as usize) + x as usize) * 3;
                    pixels[o] = r;
                    pixels[o + 1] = g;
                    pixels[o + 2] = b;
                }
            }
        }
    }
    Raster::new(width, height, 3, pixels)
}

struct Comp {
    id: u8,
    h: u8,
    v: u8,
    tq: u8,
    dc: u8,
    ac: u8,
}

fn unsupported_frame(marker: u8) -> bool {
    matches!(
        marker,
        0xc1 | 0xc2 | 0xc3 | 0xc5 | 0xc6 | 0xc7 | 0xc9 | 0xca | 0xcb | 0xcd | 0xce | 0xcf
    )
}

/// Record the Huffman table selectors from SOS. Component order must match SOF.
/// Spectral selection and successive approximation are rejected.
fn bind_scan(comps: &mut [Comp], payload: &[u8]) -> Result<(), Error> {
    if payload.is_empty() {
        return Err(INVALID);
    }
    let ns = usize::from(payload[0]);
    if ns != comps.len() || payload.len() < 1 + ns * 2 + 3 {
        return Err(INVALID);
    }
    for s in 0..ns {
        let id = payload[1 + s * 2];
        let sel = payload[2 + s * 2];
        if id != comps[s].id {
            return Err(UNSUPPORTED);
        }
        let dc = sel >> 4;
        let ac = sel & 0x0f;
        if dc > 3 || ac > 3 {
            return Err(INVALID);
        }
        comps[s].dc = dc;
        comps[s].ac = ac;
    }
    let spec = &payload[1 + ns * 2..];
    if spec[0] != 0 || spec[1] != 63 || spec[2] != 0 {
        return Err(UNSUPPORTED);
    }
    Ok(())
}

fn table(tables: &[Option<HuffDec>; 4], id: usize) -> Result<&HuffDec, Error> {
    tables[id].as_ref().ok_or(INVALID)
}

fn read_marker(data: &[u8], i: &mut usize) -> Result<u8, Error> {
    if *i >= data.len() || data[*i] != 0xff {
        return Err(INVALID);
    }
    while *i < data.len() && data[*i] == 0xff {
        *i += 1;
    }
    if *i >= data.len() {
        return Err(TRUNCATED);
    }
    let m = data[*i];
    *i += 1;
    Ok(m)
}

struct OptHuff {
    bits: [u8; 16],
    vals: Vec<u8>,
    enc: HuffEnc,
}

/// Huffman table from symbol counts, following the JPEG procedure in T.81 K.2
/// and libjpeg `jpeg_gen_optimal_table`: a pseudo-symbol keeps a real symbol
/// off the all-ones code, then codes longer than 16 bits are shortened.
fn optimal_table(counts: &[u32; 257]) -> Result<OptHuff, Error> {
    let mut freq = [0i64; 257];
    for i in 0..256 {
        freq[i] = i64::from(counts[i]);
    }
    freq[256] = 1;
    let mut codesize = [0i32; 257];
    let mut others = [-1i32; 257];
    loop {
        let mut c1 = -1i32;
        let mut best = 1_000_000_000i64;
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 && f <= best {
                best = f;
                c1 = i as i32;
            }
        }
        let mut c2 = -1i32;
        best = 1_000_000_000i64;
        for (i, &f) in freq.iter().enumerate() {
            if f != 0 && f <= best && i as i32 != c1 {
                best = f;
                c2 = i as i32;
            }
        }
        if c2 < 0 {
            break;
        }
        freq[c1 as usize] += freq[c2 as usize];
        freq[c2 as usize] = 0;
        codesize[c1 as usize] += 1;
        let mut walk = c1;
        while others[walk as usize] >= 0 {
            walk = others[walk as usize];
            codesize[walk as usize] += 1;
        }
        others[walk as usize] = c2;
        codesize[c2 as usize] += 1;
        walk = c2;
        while others[walk as usize] >= 0 {
            walk = others[walk as usize];
            codesize[walk as usize] += 1;
        }
    }

    let mut bits_n = [0i32; 33];
    for &n in &codesize {
        if n > 0 {
            if n > 32 {
                return Err(INVALID);
            }
            bits_n[n as usize] += 1;
        }
    }
    let mut i = 32i32;
    while i > 16 {
        let len = i as usize;
        while bits_n[len] > 0 {
            if bits_n[len] < 2 {
                return Err(INVALID);
            }
            let mut j = i - 2;
            while j > 0 && bits_n[j as usize] == 0 {
                j -= 1;
            }
            if j <= 0 {
                return Err(INVALID);
            }
            bits_n[len] -= 2;
            bits_n[len - 1] += 1;
            bits_n[(j + 1) as usize] += 2;
            bits_n[j as usize] -= 1;
        }
        i -= 1;
    }
    while i > 0 && bits_n[i as usize] == 0 {
        i -= 1;
    }
    if i <= 0 || bits_n[i as usize] == 0 {
        return Err(INVALID);
    }
    bits_n[i as usize] -= 1;

    let mut bits = [0u8; 16];
    for len in 1..=16 {
        let n = bits_n[len];
        if !(0..=255).contains(&n) {
            return Err(INVALID);
        }
        bits[len - 1] = n as u8;
    }
    let mut vals = Vec::new();
    for len in 1..=32 {
        for (sym, &n) in codesize[..256].iter().enumerate() {
            if n == len {
                vals.push(sym as u8);
            }
        }
    }
    let sum: usize = bits.iter().map(|&c| c as usize).sum();
    if vals.len() != sum || vals.is_empty() {
        return Err(INVALID);
    }
    let (enc, _) = build_huff(&bits, &vals);
    Ok(OptHuff { bits, vals, enc })
}

fn zigzag_quant(spatial: &[f32; 64], quant: &[u8; 64]) -> [i32; 64] {
    let mut freq = [0f32; 64];
    fdct(spatial, &mut freq);
    let mut zz = [0i32; 64];
    for k in 0..64 {
        let nat = ZIGZAG[k] as usize;
        zz[k] = (freq[nat] / f32::from(quant[nat])).round() as i32;
    }
    zz
}

fn luma_spatial(rgb: &Raster, x0: u32, y0: u32) -> [f32; 64] {
    let mut block = [0f32; 64];
    for ly in 0..8u32 {
        for lx in 0..8u32 {
            let (r, g, b) = sample(rgb, x0 + lx, y0 + ly);
            let (y, _, _) = ycbcr(r, g, b);
            block[(ly * 8 + lx) as usize] = y - 128.0;
        }
    }
    block
}

fn chroma_spatial(rgb: &Raster, x0: u32, y0: u32) -> ([f32; 64], [f32; 64]) {
    let mut cb_b = [0f32; 64];
    let mut cr_b = [0f32; 64];
    for cy in 0..8u32 {
        for cx in 0..8u32 {
            let mut cb = 0f32;
            let mut cr = 0f32;
            for dy in 0..2u32 {
                for dx in 0..2u32 {
                    let (r, g, b) = sample(rgb, x0 + cx * 2 + dx, y0 + cy * 2 + dy);
                    let (_, cbb, crr) = ycbcr(r, g, b);
                    cb += cbb;
                    cr += crr;
                }
            }
            cb_b[(cy * 8 + cx) as usize] = cb / 4.0 - 128.0;
            cr_b[(cy * 8 + cx) as usize] = cr / 4.0 - 128.0;
        }
    }
    (cb_b, cr_b)
}

fn bump(freq: &mut [u32; 257], sym: u8) {
    let i = usize::from(sym);
    freq[i] = freq[i].saturating_add(1);
}

fn note_symbols(
    zz: &[i32; 64],
    prev_dc: &mut i32,
    dc_freq: &mut [u32; 257],
    ac_freq: &mut [u32; 257],
) -> Result<(), Error> {
    let diff = zz[0] - *prev_dc;
    *prev_dc = zz[0];
    let (cat, _) = amplitude(diff);
    bump(dc_freq, cat);
    let mut k = 1;
    while k < 64 {
        let mut run = 0u8;
        while k < 64 && zz[k] == 0 {
            run += 1;
            k += 1;
        }
        if k == 64 {
            bump(ac_freq, 0x00);
            break;
        }
        while run >= 16 {
            bump(ac_freq, 0xf0);
            run -= 16;
        }
        let (cat, _) = amplitude(zz[k]);
        if cat > 15 {
            return Err(INVALID);
        }
        bump(ac_freq, (run << 4) | cat);
        k += 1;
    }
    Ok(())
}

fn write_symbols(
    zz: &[i32; 64],
    dc_tab: &HuffEnc,
    ac_tab: &HuffEnc,
    prev_dc: &mut i32,
    w: &mut Writer,
) -> Result<(), Error> {
    let diff = zz[0] - *prev_dc;
    *prev_dc = zz[0];
    let (cat, mag) = amplitude(diff);
    if dc_tab.len[usize::from(cat)] == 0 {
        return Err(INVALID);
    }
    w.symbol(dc_tab, cat);
    w.put(mag, cat);
    let mut k = 1;
    while k < 64 {
        let mut run = 0u8;
        while k < 64 && zz[k] == 0 {
            run += 1;
            k += 1;
        }
        if k == 64 {
            if ac_tab.len[0] == 0 {
                return Err(INVALID);
            }
            w.symbol(ac_tab, 0x00);
            break;
        }
        while run >= 16 {
            if ac_tab.len[0xf0] == 0 {
                return Err(INVALID);
            }
            w.symbol(ac_tab, 0xf0);
            run -= 16;
        }
        let (cat, mag) = amplitude(zz[k]);
        if cat > 15 {
            return Err(INVALID);
        }
        let sym = (run << 4) | cat;
        if ac_tab.len[usize::from(sym)] == 0 {
            return Err(INVALID);
        }
        w.symbol(ac_tab, sym);
        w.put(mag, cat);
        k += 1;
        if w.exceeded {
            return Ok(());
        }
    }
    Ok(())
}

/// Baseline JPEG with Huffman tables built from this image. Same quantization,
/// sampling, and SOF0 frame as [`encode_jpeg_limited`].
pub(crate) fn encode_jpeg_optimized_limited(
    rgb: &Raster,
    quality: u8,
    limit: usize,
) -> Result<Limited, Error> {
    if rgb.channels != 3 || quality == 0 {
        return Err(INVALID);
    }
    let qy = scale_quant(&QUANT_LUMA, quality);
    let qc = scale_quant(&QUANT_CHROMA, quality);
    let mut dc_y_f = [0u32; 257];
    let mut ac_y_f = [0u32; 257];
    let mut dc_c_f = [0u32; 257];
    let mut ac_c_f = [0u32; 257];
    let mut prev = [0i32; 3];
    let mcus_x = rgb.width.div_ceil(16);
    let mcus_y = rgb.height.div_ceil(16);
    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            for by in 0..2 {
                for bx in 0..2 {
                    let spatial = luma_spatial(rgb, mx * 16 + bx * 8, my * 16 + by * 8);
                    let zz = zigzag_quant(&spatial, &qy);
                    note_symbols(&zz, &mut prev[0], &mut dc_y_f, &mut ac_y_f)?;
                }
            }
            let (cb, cr) = chroma_spatial(rgb, mx * 16, my * 16);
            let cb_zz = zigzag_quant(&cb, &qc);
            let cr_zz = zigzag_quant(&cr, &qc);
            note_symbols(&cb_zz, &mut prev[1], &mut dc_c_f, &mut ac_c_f)?;
            note_symbols(&cr_zz, &mut prev[2], &mut dc_c_f, &mut ac_c_f)?;
        }
    }
    let dc_y = optimal_table(&dc_y_f)?;
    let ac_y = optimal_table(&ac_y_f)?;
    let dc_c = optimal_table(&dc_c_f)?;
    let ac_c = optimal_table(&ac_c_f)?;

    let mut out = Vec::with_capacity(1024);
    out.extend_from_slice(&[0xff, 0xd8]);
    out.extend_from_slice(&[0xff, 0xe0, 0x00, 0x10]);
    out.extend_from_slice(b"JFIF\0");
    out.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00]);
    write_dqt(&mut out, 0, &qy);
    write_dqt(&mut out, 1, &qc);
    out.extend_from_slice(&[0xff, 0xc0, 0x00, 0x11, 0x08]);
    out.extend_from_slice(&(rgb.height as u16).to_be_bytes());
    out.extend_from_slice(&(rgb.width as u16).to_be_bytes());
    out.extend_from_slice(&[0x03, 0x01, 0x22, 0x00, 0x02, 0x11, 0x01, 0x03, 0x11, 0x01]);
    write_dht(&mut out, 0x00, &dc_y.bits, &dc_y.vals);
    write_dht(&mut out, 0x10, &ac_y.bits, &ac_y.vals);
    write_dht(&mut out, 0x01, &dc_c.bits, &dc_c.vals);
    write_dht(&mut out, 0x11, &ac_c.bits, &ac_c.vals);
    out.extend_from_slice(&[
        0xff, 0xda, 0x00, 0x0c, 0x03, 0x01, 0x00, 0x02, 0x11, 0x03, 0x11, 0x00, 0x3f, 0x00,
    ]);
    if out.len() >= limit {
        return Ok(Limited::Exceeded);
    }
    let mut w = Writer {
        out: Vec::new(),
        acc: 0,
        n: 0,
        limit: limit - out.len(),
        exceeded: false,
    };
    prev = [0; 3];
    for my in 0..mcus_y {
        for mx in 0..mcus_x {
            for by in 0..2 {
                for bx in 0..2 {
                    let spatial = luma_spatial(rgb, mx * 16 + bx * 8, my * 16 + by * 8);
                    let zz = zigzag_quant(&spatial, &qy);
                    write_symbols(&zz, &dc_y.enc, &ac_y.enc, &mut prev[0], &mut w)?;
                }
            }
            let (cb, cr) = chroma_spatial(rgb, mx * 16, my * 16);
            let cb_zz = zigzag_quant(&cb, &qc);
            let cr_zz = zigzag_quant(&cr, &qc);
            write_symbols(&cb_zz, &dc_c.enc, &ac_c.enc, &mut prev[1], &mut w)?;
            write_symbols(&cr_zz, &dc_c.enc, &ac_c.enc, &mut prev[2], &mut w)?;
            if w.exceeded {
                return Ok(Limited::Exceeded);
            }
        }
    }
    w.finish();
    if w.exceeded {
        return Ok(Limited::Exceeded);
    }
    out.extend_from_slice(&w.out);
    out.extend_from_slice(&[0xff, 0xd9]);
    if out.len() > limit {
        return Ok(Limited::Exceeded);
    }
    Ok(Limited::Bytes(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_dc_luma_symbol_zero_is_two_bits() {
        let (enc, _) = luma_dc();
        assert_eq!(enc.len[0], 2);
        assert_eq!(enc.code[0], 0);
    }

    #[test]
    fn huffman_value_counts_match_bits() {
        let sum = |b: &[u8; 16]| b.iter().map(|&c| c as usize).sum::<usize>();
        assert_eq!(sum(&BITS_DC_LUMA), VALS_DC_LUMA.len());
        assert_eq!(sum(&BITS_DC_CHROMA), VALS_DC_CHROMA.len());
        assert_eq!(sum(&BITS_AC_LUMA), VALS_AC_LUMA.len());
        assert_eq!(sum(&BITS_AC_CHROMA), VALS_AC_CHROMA.len());
    }

    #[test]
    fn progressive_arithmetic_and_unknown_markers_error() {
        for marker in [0xc2u8, 0xc9, 0xca, 0x02] {
            let bytes = [
                0xff, 0xd8, 0xff, marker, 0x00, 0x0b, 0x08, 0x00, 0x08, 0x00, 0x08, 0x01, 0x01,
                0x11, 0x00,
            ];
            let err = decode_jpeg(&bytes).unwrap_err();
            assert_eq!(
                err.code,
                crate::error::ErrorCode::Unsupported,
                "marker {marker:#x}"
            );
        }
        // APP0 is a known marker and is skipped. The next frame is arithmetic.
        let bytes = [
            0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00, 0xff, 0xca, 0x00, 0x03, 0x00,
        ];
        assert_eq!(
            decode_jpeg(&bytes).unwrap_err().code,
            crate::error::ErrorCode::Unsupported
        );
    }

    #[test]
    fn optimal_table_keeps_the_all_ones_code_free() {
        let mut counts = [0u32; 257];
        counts[0] = 3;
        counts[1] = 1;
        let table = optimal_table(&counts).unwrap();
        assert_eq!(table.bits[0], 1);
        assert_eq!(table.bits[1], 1);
        assert_eq!(table.vals, vec![0, 1]);
        assert_eq!(table.enc.len[0], 1);
        assert_eq!(table.enc.code[0], 0);
        assert_eq!(table.enc.len[1], 2);
        assert_eq!(table.enc.code[1], 0b10);
    }

    #[test]
    fn counted_huffman_is_smaller_and_decodes() {
        let mut pixels = Vec::with_capacity(32 * 32 * 3);
        for y in 0..32u32 {
            for x in 0..32u32 {
                pixels.push((x * 8) as u8);
                pixels.push((y * 5) as u8);
                pixels.push(((x * 3 + y) % 64) as u8);
            }
        }
        let image = Raster::new(32, 32, 3, pixels).unwrap();
        let standard = encode_jpeg(&image, 20).unwrap();
        match encode_jpeg_optimized_limited(&image, 20, usize::MAX).unwrap() {
            Limited::Bytes(optimized) => {
                assert!(
                    optimized.len() < standard.len(),
                    "counted {} standard {}",
                    optimized.len(),
                    standard.len()
                );
                assert_eq!(&optimized[..2], &[0xff, 0xd8]);
                let back = decode_jpeg(&optimized).unwrap();
                assert_eq!((back.width, back.height, back.channels), (32, 32, 3));
            }
            Limited::Exceeded => panic!("unlimited encode exceeded"),
        }
    }
}
