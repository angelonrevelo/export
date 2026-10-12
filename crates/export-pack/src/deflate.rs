//! Raw deflate and zlib inflate. No compressor.
//!
//! Method 8 read is on because [`inflate_zlib`] matches the CPython zlib
//! vectors and a flipped block returns an error. The zip writer still emits store.

use crate::error::{Error, Result};
use std::io::{Read, Write};

const MAX_BITS: usize = 15;

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

struct Huff {
    count: [u16; MAX_BITS + 1],
    symbol: Vec<u16>,
}

fn build(lengths: &[u16]) -> Result<Huff> {
    let mut count = [0u16; MAX_BITS + 1];
    for &len in lengths {
        if len as usize > MAX_BITS {
            return Err(Error::Format("deflate code"));
        }
        count[len as usize] = count[len as usize].saturating_add(1);
    }
    if count[0] as usize == lengths.len() {
        return Ok(Huff {
            count,
            symbol: Vec::new(),
        });
    }
    let mut left = 1i32;
    for &n in count.iter().take(MAX_BITS + 1).skip(1) {
        left <<= 1;
        left -= i32::from(n);
        if left < 0 {
            return Err(Error::Format("deflate code"));
        }
    }
    let mut offs = [0usize; MAX_BITS + 1];
    for len in 1..MAX_BITS {
        offs[len + 1] = offs[len] + count[len] as usize;
    }
    let mut symbol = vec![0u16; lengths.len() - count[0] as usize];
    let mut next = offs;
    for (sym, &len) in lengths.iter().enumerate() {
        if len != 0 {
            let slot = next[len as usize];
            symbol[slot] = sym as u16;
            next[len as usize] = slot + 1;
        }
    }
    Ok(Huff { count, symbol })
}

fn fixed_lit() -> Result<Huff> {
    let mut lengths = vec![0u16; 288];
    for len in lengths.iter_mut().take(144) {
        *len = 8;
    }
    for len in lengths.iter_mut().take(256).skip(144) {
        *len = 9;
    }
    for len in lengths.iter_mut().take(280).skip(256) {
        *len = 7;
    }
    for len in lengths.iter_mut().skip(280) {
        *len = 8;
    }
    build(&lengths)
}

fn fixed_dist() -> Result<Huff> {
    build(&[5u16; 32])
}

struct Bits<R> {
    inner: R,
    left: u64,
    acc: u32,
    nbits: u8,
}

impl<R: Read> Bits<R> {
    fn new(inner: R, left: u64) -> Self {
        Self {
            inner,
            left,
            acc: 0,
            nbits: 0,
        }
    }

    fn take_byte(&mut self) -> Result<u8> {
        if self.left == 0 {
            return Err(Error::Format("truncated"));
        }
        let mut buf = [0u8; 1];
        self.inner.read_exact(&mut buf)?;
        self.left -= 1;
        Ok(buf[0])
    }

    fn get(&mut self, n: u8) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        while self.nbits < n {
            let byte = self.take_byte()?;
            self.acc |= u32::from(byte) << self.nbits;
            self.nbits += 8;
        }
        let value = self.acc & ((1u32 << n) - 1);
        self.acc >>= n;
        self.nbits -= n;
        Ok(value)
    }

    fn align(&mut self) {
        let extra = self.nbits % 8;
        if extra != 0 {
            self.acc >>= extra;
            self.nbits -= extra;
        }
    }

    fn read_aligned(&mut self, buf: &mut [u8]) -> Result<()> {
        self.align();
        for byte in buf {
            *byte = if self.nbits >= 8 {
                let value = (self.acc & 0xFF) as u8;
                self.acc >>= 8;
                self.nbits -= 8;
                value
            } else {
                self.take_byte()?
            };
        }
        Ok(())
    }

    fn drain(mut self) -> Result<()> {
        self.align();
        self.acc = 0;
        self.nbits = 0;
        while self.left > 0 {
            let _ = self.take_byte()?;
        }
        Ok(())
    }
}

fn decode_symbol<R: Read>(bits: &mut Bits<R>, huff: &Huff) -> Result<u16> {
    if huff.symbol.is_empty() {
        return Err(Error::Format("deflate code"));
    }
    let mut code = 0u32;
    let mut first = 0u32;
    let mut index = 0usize;
    for len in 1..=MAX_BITS {
        code |= bits.get(1)?;
        let count = u32::from(huff.count[len]);
        if code < first + count {
            let slot = index + (code - first) as usize;
            return huff
                .symbol
                .get(slot)
                .copied()
                .ok_or(Error::Format("deflate code"));
        }
        index += count as usize;
        first = (first + count) << 1;
        code <<= 1;
    }
    Err(Error::Format("deflate code"))
}

struct Out<W> {
    dst: W,
    window: Vec<u8>,
    pos: usize,
    filled: usize,
    total: u64,
    max: u64,
    pending: Vec<u8>,
}

impl<W: Write> Out<W> {
    fn new(dst: W, max: u64) -> Self {
        Self {
            dst,
            window: vec![0; 32_768],
            pos: 0,
            filled: 0,
            total: 0,
            max,
            pending: Vec::with_capacity(8192),
        }
    }

    fn push(&mut self, byte: u8) -> Result<()> {
        if self.total >= self.max {
            return Err(Error::Limit("deflate output limit"));
        }
        self.window[self.pos] = byte;
        self.pos = (self.pos + 1) % self.window.len();
        if self.filled < self.window.len() {
            self.filled += 1;
        }
        self.total += 1;
        self.pending.push(byte);
        if self.pending.len() >= 8192 {
            self.flush_pending()?;
        }
        Ok(())
    }

    fn flush_pending(&mut self) -> Result<()> {
        if !self.pending.is_empty() {
            self.dst.write_all(&self.pending)?;
            self.pending.clear();
        }
        Ok(())
    }

    fn copy_match(&mut self, dist: usize, len: usize) -> Result<()> {
        if dist == 0 || dist > self.filled || self.total < dist as u64 {
            return Err(Error::Format("deflate distance"));
        }
        for _ in 0..len {
            let idx = (self.pos + self.window.len() - dist) % self.window.len();
            let byte = self.window[idx];
            self.push(byte)?;
        }
        Ok(())
    }
}

fn stored_block<R: Read, W: Write>(bits: &mut Bits<R>, out: &mut Out<W>) -> Result<()> {
    let mut len_buf = [0u8; 4];
    bits.read_aligned(&mut len_buf)?;
    let len = u16::from_le_bytes([len_buf[0], len_buf[1]]);
    let nlen = u16::from_le_bytes([len_buf[2], len_buf[3]]);
    if nlen != !len {
        return Err(Error::Format("deflate stored"));
    }
    let mut left = usize::from(len);
    let mut buf = [0u8; 4096];
    while left > 0 {
        let n = left.min(buf.len());
        bits.read_aligned(&mut buf[..n])?;
        for &byte in &buf[..n] {
            out.push(byte)?;
        }
        left -= n;
    }
    Ok(())
}

fn codes<R: Read, W: Write>(
    bits: &mut Bits<R>,
    out: &mut Out<W>,
    lit: &Huff,
    dist: &Huff,
) -> Result<()> {
    loop {
        let sym = decode_symbol(bits, lit)?;
        match sym {
            0..=255 => out.push(sym as u8)?,
            256 => return Ok(()),
            257..=285 => {
                let idx = usize::from(sym - 257);
                let len = usize::from(LEN_BASE[idx]) + bits.get(LEN_EXTRA[idx])? as usize;
                let dsym = decode_symbol(bits, dist)?;
                if usize::from(dsym) >= DIST_BASE.len() {
                    return Err(Error::Format("deflate distance"));
                }
                let dist_v = usize::from(DIST_BASE[usize::from(dsym)])
                    + bits.get(DIST_EXTRA[usize::from(dsym)])? as usize;
                out.copy_match(dist_v, len)?;
            }
            _ => return Err(Error::Format("deflate code")),
        }
    }
}

fn dynamic_trees<R: Read>(bits: &mut Bits<R>) -> Result<(Huff, Huff)> {
    let nlit = bits.get(5)? as usize + 257;
    let ndist = bits.get(5)? as usize + 1;
    let nclen = bits.get(4)? as usize + 4;
    if nlit > 286 || ndist > 32 {
        return Err(Error::Format("deflate code"));
    }
    let mut cl_len = [0u16; 19];
    for i in 0..nclen {
        cl_len[CL_ORDER[i]] = bits.get(3)? as u16;
    }
    let cl = build(&cl_len)?;
    let mut lens = vec![0u16; nlit + ndist];
    let mut i = 0usize;
    while i < lens.len() {
        let sym = decode_symbol(bits, &cl)?;
        match sym {
            0..=15 => {
                lens[i] = sym;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(Error::Format("deflate code"));
                }
                let prev = lens[i - 1];
                let rep = 3 + bits.get(2)? as usize;
                for _ in 0..rep {
                    if i >= lens.len() {
                        return Err(Error::Format("deflate code"));
                    }
                    lens[i] = prev;
                    i += 1;
                }
            }
            17 => {
                let rep = 3 + bits.get(3)? as usize;
                for _ in 0..rep {
                    if i >= lens.len() {
                        return Err(Error::Format("deflate code"));
                    }
                    lens[i] = 0;
                    i += 1;
                }
            }
            18 => {
                let rep = 11 + bits.get(7)? as usize;
                for _ in 0..rep {
                    if i >= lens.len() {
                        return Err(Error::Format("deflate code"));
                    }
                    lens[i] = 0;
                    i += 1;
                }
            }
            _ => return Err(Error::Format("deflate code")),
        }
    }
    Ok((build(&lens[..nlit])?, build(&lens[nlit..])?))
}

/// Inflate a raw deflate stream of exactly `comp_len` bytes.
pub fn inflate_raw<R: Read, W: Write>(src: R, comp_len: u64, dst: W, max_out: u64) -> Result<u64> {
    let mut bits = Bits::new(src, comp_len);
    let mut out = Out::new(dst, max_out);
    let fixed_l = fixed_lit()?;
    let fixed_d = fixed_dist()?;
    loop {
        let bfinal = bits.get(1)?;
        match bits.get(2)? {
            0 => stored_block(&mut bits, &mut out)?,
            1 => codes(&mut bits, &mut out, &fixed_l, &fixed_d)?,
            2 => {
                let (lit, dist) = dynamic_trees(&mut bits)?;
                codes(&mut bits, &mut out, &lit, &dist)?;
            }
            _ => return Err(Error::Format("deflate block")),
        }
        if bfinal == 1 {
            break;
        }
    }
    out.flush_pending()?;
    let total = out.total;
    bits.drain()?;
    Ok(total)
}

/// Inflate a zlib wrapper (CMF/FLG, raw deflate, Adler-32).
pub fn inflate_zlib(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 6 {
        return Err(Error::Format("truncated"));
    }
    let cmf = data[0];
    let flg = data[1];
    if cmf & 0x0F != 8 {
        return Err(Error::Unsupported("zlib method"));
    }
    if (u16::from(cmf) * 256 + u16::from(flg)) % 31 != 0 {
        return Err(Error::Format("zlib header"));
    }
    if flg & 0x20 != 0 {
        return Err(Error::Unsupported("zlib dictionary"));
    }
    let raw_end = data.len() - 4;
    let mut out = Vec::new();
    inflate_raw(
        &data[2..raw_end],
        (raw_end - 2) as u64,
        &mut out,
        u64::MAX / 4,
    )?;
    let expect = u32::from_be_bytes([
        data[raw_end],
        data[raw_end + 1],
        data[raw_end + 2],
        data[raw_end + 3],
    ]);
    if adler32(&out) != expect {
        return Err(Error::ChecksumMismatch);
    }
    Ok(out)
}

pub fn adler32(data: &[u8]) -> u32 {
    let mut s1: u32 = 1;
    let mut s2: u32 = 0;
    for &byte in data {
        s1 = (s1 + u32::from(byte)) % 65521;
        s2 = (s2 + s1) % 65521;
    }
    (s2 << 16) | s1
}

/// Zlib wrapper around stored deflate blocks. Used to build PNG fixtures.
pub fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x78, 0x01];
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xFF, 0xFF]);
    } else {
        let mut i = 0usize;
        while i < data.len() {
            let n = (data.len() - i).min(65_535);
            let last = i + n == data.len();
            out.push(if last { 0x01 } else { 0x00 });
            let len = n as u16;
            out.extend_from_slice(&len.to_le_bytes());
            out.extend_from_slice(&(!len).to_le_bytes());
            out.extend_from_slice(&data[i..i + n]);
            i += n;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::inflate_zlib;

    /// zlib.compress(b"hello") from CPython 3.12.
    const HELLO: &[u8] = &[120, 156, 203, 72, 205, 201, 201, 7, 0, 6, 44, 2, 21];
    /// zlib.compress(b"hello hello hello") — dynamic codes and a match.
    const REPEAT: &[u8] = &[
        120, 156, 203, 72, 205, 201, 201, 87, 200, 64, 144, 0, 58, 46, 6, 125,
    ];

    #[test]
    fn known_zlib_vectors() {
        assert_eq!(inflate_zlib(HELLO).unwrap(), b"hello");
        assert_eq!(inflate_zlib(REPEAT).unwrap(), b"hello hello hello");
    }

    #[test]
    fn flipped_block_errors() {
        let mut bad = HELLO.to_vec();
        bad[4] ^= 0xFF;
        assert!(inflate_zlib(&bad).is_err());
    }
}
