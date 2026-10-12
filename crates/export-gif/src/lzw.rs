//! GIF LZW. Codes are packed least-significant bit first.
//!
//! The first code is a clear code. The table grows until 12-bit codes, then
//! another clear code resets it. A code past the table, or a stream that ends
//! before the end code, is an error.

use crate::error::{Error, BAD_LZW, INVALID, TRUNCATED};

const MAX_CODE: u16 = 4095;
const LZ_MAX_CODE: u16 = 4095;

/// Compress indexed pixels. `min_code_size` is 2..=8, and every pixel must be
/// smaller than `1 << min_code_size`.
pub fn encode(min_code_size: u8, pixels: &[u8]) -> Result<Vec<u8>, Error> {
    if !(2..=8).contains(&min_code_size) {
        return Err(INVALID);
    }
    let clear = 1u16 << min_code_size;
    let eoi = clear + 1;
    for &px in pixels {
        if u16::from(px) >= clear {
            return Err(INVALID);
        }
    }

    let mut next_code = eoi + 1;
    let mut code_size = min_code_size + 1;
    let mut max_code1 = 1u16 << code_size;
    let mut dict = Table::new();
    let mut w = BitWriter::new();

    write_code(&mut w, clear, &mut code_size, next_code, &mut max_code1);
    if pixels.is_empty() {
        write_code(&mut w, eoi, &mut code_size, next_code, &mut max_code1);
        return Ok(w.finish());
    }

    let mut crnt = u16::from(pixels[0]);
    for &px in &pixels[1..] {
        if let Some(found) = dict.get(crnt, px) {
            crnt = found;
            continue;
        }
        write_code(&mut w, crnt, &mut code_size, next_code, &mut max_code1);
        if next_code >= LZ_MAX_CODE {
            write_code(&mut w, clear, &mut code_size, next_code, &mut max_code1);
            dict.reset();
            next_code = eoi + 1;
            code_size = min_code_size + 1;
            max_code1 = 1u16 << code_size;
        } else {
            dict.insert(crnt, px, next_code);
            next_code += 1;
        }
        crnt = u16::from(px);
    }
    write_code(&mut w, crnt, &mut code_size, next_code, &mut max_code1);
    write_code(&mut w, eoi, &mut code_size, next_code, &mut max_code1);
    Ok(w.finish())
}

/// Expand one image's LZW bytes into exactly `pixel_count` indices.
pub fn decode(min_code_size: u8, data: &[u8], pixel_count: usize) -> Result<Vec<u8>, Error> {
    if !(2..=8).contains(&min_code_size) {
        return Err(INVALID);
    }
    let clear = 1u16 << min_code_size;
    let eoi = clear + 1;
    let mut dec = Decoder {
        data,
        pos: 0,
        bit_buf: 0,
        bit_n: 0,
        code_size: min_code_size + 1,
        next_code: eoi + 1,
        max_code1: 1u16 << (min_code_size + 1),
        clear,
        eoi,
        min_code_size,
        prefix: [0; 4096],
        suffix: [0; 4096],
        defined: [false; 4096],
    };

    let mut out = Vec::with_capacity(pixel_count);
    let mut prev: Option<u16> = None;
    let mut rev = [0u8; 4096];

    loop {
        if out.len() > pixel_count {
            return Err(INVALID);
        }
        let code = dec.read_code()?;
        if code == dec.clear {
            dec.reset_table();
            prev = None;
            continue;
        }
        if code == dec.eoi {
            break;
        }

        let slot = dec.next_code.wrapping_sub(2);
        let kw = code == slot;
        if kw {
            if prev.is_none() || slot > MAX_CODE {
                return Err(BAD_LZW);
            }
        } else if code > MAX_CODE
            || (code >= dec.clear && (code > slot || !dec.defined[code as usize]))
        {
            return Err(BAD_LZW);
        }

        let first = emit(&dec, &mut out, &mut rev, code, kw, prev)?;
        if out.len() > pixel_count {
            return Err(INVALID);
        }
        if let Some(p) = prev {
            if slot <= MAX_CODE && !dec.defined[slot as usize] {
                dec.prefix[slot as usize] = p;
                dec.suffix[slot as usize] = first;
                dec.defined[slot as usize] = true;
            }
        }
        prev = Some(code);
    }

    if out.len() != pixel_count {
        return Err(TRUNCATED);
    }
    Ok(out)
}

/// True when a clear code appears after the leading clear.
#[cfg(test)]
fn has_midstream_clear(min_code_size: u8, data: &[u8]) -> Result<bool, Error> {
    if !(2..=8).contains(&min_code_size) {
        return Err(INVALID);
    }
    let clear = 1u16 << min_code_size;
    let eoi = clear + 1;
    let mut dec = Decoder {
        data,
        pos: 0,
        bit_buf: 0,
        bit_n: 0,
        code_size: min_code_size + 1,
        next_code: eoi + 1,
        max_code1: 1u16 << (min_code_size + 1),
        clear,
        eoi,
        min_code_size,
        prefix: [0; 4096],
        suffix: [0; 4096],
        defined: [false; 4096],
    };
    let mut clears = 0u32;
    loop {
        let code = match dec.read_code() {
            Ok(c) => c,
            Err(e) if e == TRUNCATED => return Ok(clears > 1),
            Err(e) => return Err(e),
        };
        if code == clear {
            clears += 1;
            dec.reset_table();
            if clears > 1 {
                return Ok(true);
            }
        } else if code == eoi {
            return Ok(false);
        }
    }
}

struct Decoder<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buf: u64,
    bit_n: u8,
    code_size: u8,
    next_code: u16,
    max_code1: u16,
    clear: u16,
    eoi: u16,
    min_code_size: u8,
    prefix: [u16; 4096],
    suffix: [u8; 4096],
    defined: [bool; 4096],
}

impl Decoder<'_> {
    fn reset_table(&mut self) {
        self.next_code = self.eoi + 1;
        self.code_size = self.min_code_size + 1;
        self.max_code1 = 1u16 << self.code_size;
        self.defined = [false; 4096];
    }

    fn read_code(&mut self) -> Result<u16, Error> {
        while self.bit_n < self.code_size {
            if self.pos >= self.data.len() {
                return Err(TRUNCATED);
            }
            self.bit_buf |= u64::from(self.data[self.pos]) << self.bit_n;
            self.bit_n += 8;
            self.pos += 1;
        }
        let mask = (1u64 << self.code_size) - 1;
        let code = (self.bit_buf & mask) as u16;
        self.bit_buf >>= self.code_size;
        self.bit_n -= self.code_size;
        if self.next_code < LZ_MAX_CODE + 2 {
            self.next_code += 1;
            if self.next_code > self.max_code1 && self.code_size < 12 {
                self.max_code1 <<= 1;
                self.code_size += 1;
            }
        }
        Ok(code)
    }
}

fn emit(
    dec: &Decoder<'_>,
    out: &mut Vec<u8>,
    rev: &mut [u8; 4096],
    code: u16,
    kw: bool,
    prev: Option<u16>,
) -> Result<u8, Error> {
    let mut n = 0usize;
    if kw {
        rev[0] = 0;
        n = 1;
    }
    let mut c = if kw { prev.ok_or(BAD_LZW)? } else { code };
    let mut guard = 0u16;
    while c >= dec.clear {
        if c > MAX_CODE || !dec.defined[c as usize] || n >= 4095 || guard > MAX_CODE {
            return Err(BAD_LZW);
        }
        rev[n] = dec.suffix[c as usize];
        n += 1;
        c = dec.prefix[c as usize];
        guard += 1;
    }
    let first = u8::try_from(c).map_err(|_| BAD_LZW)?;
    if n >= 4096 {
        return Err(BAD_LZW);
    }
    rev[n] = first;
    n += 1;
    if kw {
        rev[0] = first;
    }
    while n > 0 {
        n -= 1;
        out.push(rev[n]);
    }
    Ok(first)
}

fn write_code(
    w: &mut BitWriter,
    code: u16,
    code_size: &mut u8,
    next_code: u16,
    max_code1: &mut u16,
) {
    w.push(code, *code_size);
    if code <= MAX_CODE && next_code >= *max_code1 && *code_size < 12 {
        *code_size += 1;
        *max_code1 = 1u16 << *code_size;
    }
}

struct BitWriter {
    buf: Vec<u8>,
    acc: u64,
    bits: u8,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            buf: Vec::new(),
            acc: 0,
            bits: 0,
        }
    }

    fn push(&mut self, code: u16, n: u8) {
        self.acc |= u64::from(code) << self.bits;
        self.bits += n;
        while self.bits >= 8 {
            self.buf.push(self.acc as u8);
            self.acc >>= 8;
            self.bits -= 8;
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.bits > 0 {
            self.buf.push(self.acc as u8);
        }
        self.buf
    }
}

/// Open-addressed `(prefix, byte) -> code` map. Serial. No heap beyond the table.
struct Table {
    keys: Vec<u32>,
    codes: Vec<u16>,
    used: usize,
}

impl Table {
    fn new() -> Self {
        Self {
            keys: vec![u32::MAX; 8192],
            codes: vec![0; 8192],
            used: 0,
        }
    }

    fn reset(&mut self) {
        self.keys.fill(u32::MAX);
        self.used = 0;
    }

    fn get(&self, prefix: u16, byte: u8) -> Option<u16> {
        let key = key(prefix, byte);
        let mask = self.keys.len() - 1;
        let mut i = (key as usize) & mask;
        loop {
            let slot = self.keys[i];
            if slot == u32::MAX {
                return None;
            }
            if slot == key {
                return Some(self.codes[i]);
            }
            i = (i + 1) & mask;
        }
    }

    fn insert(&mut self, prefix: u16, byte: u8, code: u16) {
        if self.used * 2 >= self.keys.len() {
            self.grow();
        }
        let key = key(prefix, byte);
        let mask = self.keys.len() - 1;
        let mut i = (key as usize) & mask;
        loop {
            if self.keys[i] == u32::MAX {
                self.keys[i] = key;
                self.codes[i] = code;
                self.used += 1;
                return;
            }
            i = (i + 1) & mask;
        }
    }

    fn grow(&mut self) {
        let next = self.keys.len() * 2;
        let old_keys = std::mem::replace(&mut self.keys, vec![u32::MAX; next]);
        let old_codes = std::mem::replace(&mut self.codes, vec![0; next]);
        self.used = 0;
        for (k, c) in old_keys.into_iter().zip(old_codes) {
            if k != u32::MAX {
                let prefix = (k >> 8) as u16;
                let byte = k as u8;
                self.insert(prefix, byte, c);
            }
        }
    }
}

fn key(prefix: u16, byte: u8) -> u32 {
    (u32::from(prefix) << 8) | u32::from(byte)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_pixel_bits() {
        assert_eq!(encode(2, &[0]).unwrap(), vec![0x44, 0x01]);
    }

    #[test]
    fn two_pixels_bits() {
        assert_eq!(encode(2, &[0, 1]).unwrap(), vec![0x44, 0x0a]);
    }

    #[test]
    fn kwkwk_round_trip() {
        let pixels = [0u8, 0, 0];
        let bytes = encode(2, &pixels).unwrap();
        assert_eq!(decode(2, &bytes, pixels.len()).unwrap(), pixels);
    }

    #[test]
    fn clear_then_bad_code() {
        let err = decode(2, &[0x34], 1).unwrap_err();
        assert_eq!(err, BAD_LZW);
    }

    #[test]
    fn truncated_bit_stream() {
        let err = decode(2, &[0x04], 1).unwrap_err();
        assert_eq!(err, TRUNCATED);
    }

    #[test]
    fn hand_built_clear_round_trip() {
        let pixels = decode(2, &[0x44, 0x88, 0x02], 3).unwrap();
        assert_eq!(pixels, vec![0, 1, 0]);
    }

    #[test]
    fn incompressible_emits_clear() {
        let mut state = 0x1234_5678u32;
        let mut pixels = Vec::with_capacity(12_000);
        for _ in 0..12_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            pixels.push((state >> 24) as u8);
        }
        let bytes = encode(8, &pixels).unwrap();
        assert!(has_midstream_clear(8, &bytes).unwrap());
        assert_eq!(decode(8, &bytes, pixels.len()).unwrap(), pixels);
    }
}
