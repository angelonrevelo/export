use std::fmt;

use crate::bits::BitReader;
use crate::huff::{
    fixed_dist_lengths, fixed_lit_lengths, CL_ORDER, DIST_BASE, DIST_EXTRA, LEN_BASE, LEN_EXTRA,
};

/// Inflate refuses to grow the output past this many bytes.
pub const DEFAULT_MAX_OUT: usize = 64 * 1024 * 1024;

const MAX_DISTANCE: usize = 32_768;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// The block ended before its lengths, codes, or stored bytes.
    Truncated,
    /// The block type, Huffman tree, length check, or distance is invalid.
    BadInput,
    /// Output would grow past the caller's limit.
    TooLarge,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Error::Truncated => "truncated deflate stream",
            Error::BadInput => "invalid deflate stream",
            Error::TooLarge => "deflate output exceeded the limit",
        })
    }
}

impl std::error::Error for Error {}

pub fn inflate(input: &[u8]) -> Result<Vec<u8>, Error> {
    inflate_with_limit(input, DEFAULT_MAX_OUT)
}

pub fn inflate_with_limit(input: &[u8], max_out: usize) -> Result<Vec<u8>, Error> {
    let mut bits = BitReader::new(input);
    let mut out = Vec::new();
    loop {
        let bfinal = bits.pull(1)?;
        match bits.pull(2)? {
            0 => stored(&mut bits, &mut out, max_out)?,
            1 => {
                let lit = Decoder::from_lengths(&fixed_lit_lengths())?;
                let dist = Decoder::from_lengths(&fixed_dist_lengths())?;
                codes(&mut bits, &lit, &dist, &mut out, max_out)?;
            }
            2 => {
                let (lit_len, dist_len) = dynamic_lengths(&mut bits)?;
                let lit = Decoder::from_lengths(&lit_len)?;
                let dist = Decoder::from_lengths(&dist_len)?;
                codes(&mut bits, &lit, &dist, &mut out, max_out)?;
            }
            _ => return Err(Error::BadInput),
        }
        if bfinal == 1 {
            break;
        }
    }
    Ok(out)
}

fn stored(bits: &mut BitReader<'_>, out: &mut Vec<u8>, max_out: usize) -> Result<(), Error> {
    bits.align()?;
    let len = bits.pull(16)? as usize;
    let nlen = bits.pull(16)? as usize;
    if nlen != (!len & 0xffff) {
        return Err(Error::BadInput);
    }
    if out.len().checked_add(len).ok_or(Error::TooLarge)? > max_out {
        return Err(Error::TooLarge);
    }
    out.extend_from_slice(bits.take_bytes(len)?);
    Ok(())
}

fn dynamic_lengths(bits: &mut BitReader<'_>) -> Result<(Vec<u8>, Vec<u8>), Error> {
    let hlit = bits.pull(5)? as usize + 257;
    let hdist = bits.pull(5)? as usize + 1;
    let hclen = bits.pull(4)? as usize + 4;
    if hlit > 288 || hdist > 32 || hclen > 19 {
        return Err(Error::BadInput);
    }
    let mut cl_len = [0u8; 19];
    for slot in CL_ORDER.iter().take(hclen) {
        cl_len[*slot] = bits.pull(3)? as u8;
    }
    let cl = Decoder::from_lengths(&cl_len)?;
    let total = hlit + hdist;
    let mut lengths = vec![0u8; total];
    let mut i = 0;
    while i < total {
        match cl.decode(bits)? {
            sym @ 0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(Error::BadInput);
                }
                let rep = 3 + bits.pull(2)? as usize;
                if i + rep > total {
                    return Err(Error::BadInput);
                }
                let value = lengths[i - 1];
                for _ in 0..rep {
                    lengths[i] = value;
                    i += 1;
                }
            }
            17 => {
                let rep = 3 + bits.pull(3)? as usize;
                if i + rep > total {
                    return Err(Error::BadInput);
                }
                i += rep;
            }
            18 => {
                let rep = 11 + bits.pull(7)? as usize;
                if i + rep > total {
                    return Err(Error::BadInput);
                }
                i += rep;
            }
            _ => return Err(Error::BadInput),
        }
    }
    Ok((lengths[..hlit].to_vec(), lengths[hlit..].to_vec()))
}

fn codes(
    bits: &mut BitReader<'_>,
    lit: &Decoder,
    dist: &Decoder,
    out: &mut Vec<u8>,
    max_out: usize,
) -> Result<(), Error> {
    loop {
        match lit.decode(bits)? {
            sym if sym < 256 => {
                if out.len() >= max_out {
                    return Err(Error::TooLarge);
                }
                out.push(sym as u8);
            }
            256 => return Ok(()),
            sym @ 257..=285 => {
                let idx = (sym - 257) as usize;
                let add = bits.pull(u32::from(LEN_EXTRA[idx]))? as usize;
                let length = LEN_BASE[idx] as usize + add;
                let dsym = dist.decode(bits)? as usize;
                if dsym >= DIST_BASE.len() {
                    return Err(Error::BadInput);
                }
                let dadd = bits.pull(u32::from(DIST_EXTRA[dsym]))? as usize;
                let distance = DIST_BASE[dsym] as usize + dadd;
                copy_match(out, distance, length, max_out)?;
            }
            _ => return Err(Error::BadInput),
        }
    }
}

fn copy_match(out: &mut Vec<u8>, dist: usize, len: usize, max_out: usize) -> Result<(), Error> {
    if dist == 0 || dist > out.len() || dist > MAX_DISTANCE {
        return Err(Error::BadInput);
    }
    let new_len = out.len().checked_add(len).ok_or(Error::TooLarge)?;
    if new_len > max_out {
        return Err(Error::TooLarge);
    }
    if dist == 1 {
        let byte = out[out.len() - 1];
        out.resize(new_len, byte);
        return Ok(());
    }
    let start = out.len() - dist;
    if len <= dist {
        out.extend_from_within(start..start + len);
    } else {
        for _ in 0..len {
            let byte = out[out.len() - dist];
            out.push(byte);
        }
    }
    Ok(())
}

struct Decoder {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Decoder {
    fn from_lengths(lengths: &[u8]) -> Result<Self, Error> {
        let mut counts = [0u16; 16];
        let mut left = 1i32;
        for &len in lengths {
            if len > 15 {
                return Err(Error::BadInput);
            }
            if len > 0 {
                counts[len as usize] =
                    counts[len as usize].checked_add(1).ok_or(Error::BadInput)?;
            }
        }
        for count in counts.iter().skip(1) {
            left <<= 1;
            left -= i32::from(*count);
            if left < 0 {
                return Err(Error::BadInput);
            }
        }
        let mut symbols = vec![0u16; lengths.len()];
        let mut offs = [0usize; 16];
        let mut sum = 0usize;
        for len in 1..16 {
            offs[len] = sum;
            sum += usize::from(counts[len]);
        }
        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let slot = offs[len as usize];
            symbols[slot] = sym as u16;
            offs[len as usize] = slot + 1;
        }
        symbols.truncate(sum);
        Ok(Self { counts, symbols })
    }

    fn decode(&self, bits: &mut BitReader<'_>) -> Result<u16, Error> {
        let mut code = 0u32;
        let mut first = 0u32;
        let mut index = 0usize;
        for len in 1..=15 {
            code = (code << 1) | bits.pull(1)?;
            let count = u32::from(self.counts[len]);
            if code.saturating_sub(first) < count {
                let pos = index + (code - first) as usize;
                return self.symbols.get(pos).copied().ok_or(Error::BadInput);
            }
            index += count as usize;
            first = first.wrapping_add(count) << 1;
        }
        Err(Error::BadInput)
    }
}
