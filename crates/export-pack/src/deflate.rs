//! Raw deflate and zlib inflate. Stored blocks only. No compressor, no Huffman.
//!
//! A fixed or dynamic Huffman block returns [`Error::Unsupported`] before any
//! output byte is written. The zip writer still emits store.

use crate::error::{Error, Result};
use std::io::{Read, Write};

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

struct Out<W> {
    dst: W,
    total: u64,
    max: u64,
    pending: Vec<u8>,
}

impl<W: Write> Out<W> {
    fn new(dst: W, max: u64) -> Self {
        Self {
            dst,
            total: 0,
            max,
            pending: Vec::with_capacity(8192),
        }
    }

    fn push(&mut self, byte: u8) -> Result<()> {
        if self.total >= self.max {
            return Err(Error::Limit("deflate output limit"));
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

/// Inflate a raw deflate stream of exactly `comp_len` bytes.
///
/// Block type 1 (fixed Huffman) and block type 2 (dynamic Huffman) error
/// before this function writes to `dst`.
pub fn inflate_raw<R: Read, W: Write>(src: R, comp_len: u64, dst: W, max_out: u64) -> Result<u64> {
    let mut bits = Bits::new(src, comp_len);
    let mut out = Out::new(dst, max_out);
    loop {
        let bfinal = bits.get(1)?;
        match bits.get(2)? {
            0 => stored_block(&mut bits, &mut out)?,
            1 => return Err(Error::Unsupported("fixed huffman")),
            2 => return Err(Error::Unsupported("dynamic huffman")),
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
    use super::{inflate_raw, inflate_zlib, zlib_store, Error};
    use std::io::Write;

    /// `zlib.compress(b"hello")` from CPython 3.12.
    /// The first deflate byte is `0xCB` (`0b11001011`). Read LSB first, that is
    /// bfinal = 1 and btype = 01, a single fixed-Huffman block.
    const FIXED_HUFFMAN_HELLO: &[u8] = &[120, 156, 203, 72, 205, 201, 201, 7, 0, 6, 44, 2, 21];

    struct Counting(u64);

    impl Write for Counting {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0 += buf.len() as u64;
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stored_zlib_roundtrip() {
        let raw = b"hello hello hello";
        assert_eq!(inflate_zlib(&zlib_store(raw)).unwrap(), raw);
    }

    #[test]
    fn fixed_huffman_zlib_errors_without_emitting() {
        let err = inflate_zlib(FIXED_HUFFMAN_HELLO).unwrap_err();
        assert!(matches!(err, Error::Unsupported("fixed huffman")), "{err}");

        let raw = &FIXED_HUFFMAN_HELLO[2..FIXED_HUFFMAN_HELLO.len() - 4];
        let mut sink = Counting(0);
        let err = inflate_raw(raw, raw.len() as u64, &mut sink, u64::MAX).unwrap_err();
        assert!(matches!(err, Error::Unsupported("fixed huffman")), "{err}");
        assert_eq!(sink.0, 0);
    }
}
