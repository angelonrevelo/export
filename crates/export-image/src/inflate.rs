//! Deflate (RFC 1951) and the zlib wrapper (RFC 1950).
//!
//! The inflater owns stored, fixed-Huffman, and dynamic-Huffman blocks. The
//! compressor used by the PNG encoder is LZ77 plus the fixed Huffman codes.
//! Both are checked against raw zlib streams, not only against each other.

use crate::error::{Error, INVALID, TRUNCATED};

const MAX_BITS: usize = 15;
const MAX_MATCH: usize = 258;
const WINDOW: usize = 32_768;

pub fn adler32(data: &[u8]) -> u32 {
    let mut a: u32 = 1;
    let mut b: u32 = 0;
    for (i, &byte) in data.iter().enumerate() {
        a += byte as u32;
        b += a;
        // 5552 keeps the sums inside a range where one modulo is enough.
        if i % 5552 == 5551 {
            a %= 65521;
            b %= 65521;
        }
    }
    a %= 65521;
    b %= 65521;
    (b << 16) | a
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    fn bit(&mut self) -> Result<u32, Error> {
        let byte_i = self.pos / 8;
        if byte_i >= self.data.len() {
            return Err(TRUNCATED);
        }
        let shift = self.pos % 8;
        let bit = (self.data[byte_i] >> shift) & 1;
        self.pos += 1;
        Ok(bit as u32)
    }

    fn bits(&mut self, n: u8) -> Result<u32, Error> {
        let mut v = 0u32;
        for i in 0..n {
            v |= self.bit()? << i;
        }
        Ok(v)
    }

    fn align(&mut self) {
        let rem = self.pos % 8;
        if rem != 0 {
            self.pos += 8 - rem;
        }
    }

    fn u16(&mut self) -> Result<u16, Error> {
        self.align();
        let i = self.pos / 8;
        if i + 2 > self.data.len() {
            return Err(TRUNCATED);
        }
        let v = u16::from_le_bytes([self.data[i], self.data[i + 1]]);
        self.pos += 16;
        Ok(v)
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        self.align();
        let i = self.pos / 8;
        let end = i.checked_add(n).ok_or(TRUNCATED)?;
        if end > self.data.len() {
            return Err(TRUNCATED);
        }
        self.pos += n * 8;
        Ok(&self.data[i..end])
    }
}

struct Huff {
    counts: [u16; MAX_BITS + 1],
    symbols: Vec<u16>,
    first: [u32; MAX_BITS + 1],
    limit: [u32; MAX_BITS + 1],
    index: [u16; MAX_BITS + 1],
    max_len: u8,
}

impl Huff {
    fn build(lengths: &[u16]) -> Result<Self, Error> {
        let mut counts = [0u16; MAX_BITS + 1];
        let mut max_len = 0u8;
        for &len in lengths {
            if len > MAX_BITS as u16 {
                return Err(INVALID);
            }
            if len > 0 {
                counts[len as usize] += 1;
                max_len = max_len.max(len as u8);
            }
        }
        let mut left = 1i32;
        for &count in &counts[1..=MAX_BITS] {
            left <<= 1;
            left -= i32::from(count);
            if left < 0 {
                return Err(INVALID);
            }
        }
        let mut next = [0u32; MAX_BITS + 1];
        let mut code = 0u32;
        for len in 1..=MAX_BITS {
            code = (code + counts[len - 1] as u32) << 1;
            next[len] = code;
        }
        let mut offs = [0u16; MAX_BITS + 1];
        let mut total = 0u16;
        for len in 1..=MAX_BITS {
            offs[len] = total;
            total += counts[len];
        }
        let mut symbols = vec![0u16; total as usize];
        let mut first = [0u32; MAX_BITS + 1];
        let mut limit = [0u32; MAX_BITS + 1];
        let mut filled = [0u16; MAX_BITS + 1];
        for (sym, &len) in lengths.iter().enumerate() {
            if len == 0 {
                continue;
            }
            let assigned = next[len as usize];
            next[len as usize] = assigned + 1;
            let slot = (offs[len as usize] + filled[len as usize]) as usize;
            symbols[slot] = sym as u16;
            if filled[len as usize] == 0 {
                first[len as usize] = assigned;
            }
            limit[len as usize] = assigned;
            filled[len as usize] += 1;
        }
        Ok(Self {
            counts,
            symbols,
            first,
            limit,
            index: offs,
            max_len,
        })
    }

    fn symbol(&self, bits: &mut Bits<'_>) -> Result<u16, Error> {
        if self.max_len == 0 {
            return Err(INVALID);
        }
        let mut code = 0u32;
        for len in 1..=self.max_len {
            code = (code << 1) | bits.bit()?;
            let l = len as usize;
            if self.counts[l] == 0 {
                continue;
            }
            if code <= self.limit[l] {
                let i = self.index[l] as u32 + code - self.first[l];
                return Ok(self.symbols[i as usize]);
            }
        }
        Err(INVALID)
    }
}

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115,
    131, 163, 195, 227, 258,
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

fn fixed_pair() -> &'static (Huff, Huff) {
    use std::sync::OnceLock;
    static TREES: OnceLock<(Huff, Huff)> = OnceLock::new();
    TREES.get_or_init(|| {
        let mut lens = [0u16; 288];
        for (sym, len) in lens.iter_mut().enumerate() {
            *len = match sym {
                0..=143 => 8,
                144..=255 => 9,
                256..=279 => 7,
                _ => 8,
            };
        }
        let lit = Huff::build(&lens).expect("fixed lit tree");
        let dist = Huff::build(&[5u16; 32]).expect("fixed dist tree");
        (lit, dist)
    })
}

fn inflate_codes(
    bits: &mut Bits<'_>,
    lit: &Huff,
    dist: &Huff,
    out: &mut Vec<u8>,
    limit: usize,
) -> Result<(), Error> {
    loop {
        let sym = lit.symbol(bits)?;
        match sym {
            0..=255 => {
                if out.len() >= limit {
                    return Err(TOO_BIG);
                }
                out.push(sym as u8);
            }
            256 => return Ok(()),
            257..=285 => {
                let idx = (sym - 257) as usize;
                let extra = bits.bits(LEN_EXTRA[idx])?;
                let len = LEN_BASE[idx] as usize + extra as usize;
                let dsym = dist.symbol(bits)? as usize;
                if dsym >= 30 {
                    return Err(INVALID);
                }
                let dextra = bits.bits(DIST_EXTRA[dsym])?;
                let distance = DIST_BASE[dsym] as usize + dextra as usize;
                if distance == 0 || distance > out.len() || distance > WINDOW {
                    return Err(INVALID);
                }
                if out.len() + len > limit {
                    return Err(TOO_BIG);
                }
                for _ in 0..len {
                    let b = out[out.len() - distance];
                    out.push(b);
                }
            }
            _ => return Err(INVALID),
        }
    }
}

const TOO_BIG: Error = Error::new(crate::error::ErrorCode::TooLarge);

fn inflate_blocks(bits: &mut Bits<'_>, out: &mut Vec<u8>, limit: usize) -> Result<(), Error> {
    loop {
        let bfinal = bits.bit()?;
        let btype = bits.bits(2)?;
        match btype {
            0 => {
                let len = bits.u16()? as usize;
                let nlen = bits.u16()? as usize;
                if len ^ 0xffff != nlen {
                    return Err(INVALID);
                }
                let data = bits.bytes(len)?;
                if out.len() + data.len() > limit {
                    return Err(TOO_BIG);
                }
                out.extend_from_slice(data);
            }
            1 => {
                let (lit, dist) = fixed_pair();
                inflate_codes(bits, lit, dist, out, limit)?;
            }
            2 => {
                let hlit = bits.bits(5)? as usize + 257;
                let hdist = bits.bits(5)? as usize + 1;
                let hclen = bits.bits(4)? as usize + 4;
                if hlit > 286 || hdist > 32 {
                    return Err(INVALID);
                }
                const ORDER: [usize; 19] = [
                    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
                ];
                let mut clen = [0u16; 19];
                for i in 0..hclen {
                    clen[ORDER[i]] = bits.bits(3)? as u16;
                }
                let cl_tree = Huff::build(&clen)?;
                let mut lengths = vec![0u16; hlit + hdist];
                let mut i = 0;
                while i < lengths.len() {
                    let sym = cl_tree.symbol(bits)?;
                    match sym {
                        0..=15 => {
                            lengths[i] = sym;
                            i += 1;
                        }
                        16 => {
                            if i == 0 {
                                return Err(INVALID);
                            }
                            let rep = 3 + bits.bits(2)? as usize;
                            let prev = lengths[i - 1];
                            if i + rep > lengths.len() {
                                return Err(INVALID);
                            }
                            for _ in 0..rep {
                                lengths[i] = prev;
                                i += 1;
                            }
                        }
                        17 => {
                            let rep = 3 + bits.bits(3)? as usize;
                            if i + rep > lengths.len() {
                                return Err(INVALID);
                            }
                            i += rep;
                        }
                        18 => {
                            let rep = 11 + bits.bits(7)? as usize;
                            if i + rep > lengths.len() {
                                return Err(INVALID);
                            }
                            i += rep;
                        }
                        _ => return Err(INVALID),
                    }
                }
                let lit = Huff::build(&lengths[..hlit])?;
                let dist = Huff::build(&lengths[hlit..])?;
                inflate_codes(bits, &lit, &dist, out, limit)?;
            }
            _ => return Err(INVALID),
        }
        if bfinal == 1 {
            return Ok(());
        }
    }
}

/// Inflate a zlib wrapper. `limit` is the maximum decompressed size.
pub fn zlib_inflate(data: &[u8], limit: usize) -> Result<Vec<u8>, Error> {
    if data.len() < 2 {
        return Err(TRUNCATED);
    }
    let cmf = data[0];
    let flg = data[1];
    if cmf & 0x0f != 8 || cmf >> 4 > 7 {
        return Err(INVALID);
    }
    if ((cmf as u16) * 256 + flg as u16) % 31 != 0 {
        return Err(INVALID);
    }
    if flg & 0x20 != 0 {
        return Err(crate::error::UNSUPPORTED);
    }
    let mut bits = Bits { data, pos: 16 };
    let mut out = Vec::new();
    inflate_blocks(&mut bits, &mut out, limit)?;
    bits.align();
    let sum_bytes = bits.bytes(4)?;
    let got = u32::from_be_bytes([sum_bytes[0], sum_bytes[1], sum_bytes[2], sum_bytes[3]]);
    if got != adler32(&out) {
        return Err(INVALID);
    }
    Ok(out)
}

struct Lsb {
    out: Vec<u8>,
    acc: u64,
    n: u32,
}

impl Lsb {
    fn put(&mut self, v: u32, n: u8) {
        self.acc |= u64::from(v) << self.n;
        self.n += u32::from(n);
        while self.n >= 8 {
            self.out.push(self.acc as u8);
            self.acc >>= 8;
            self.n -= 8;
        }
    }

    fn code(&mut self, code: u16, len: u8) {
        if len == 0 {
            return;
        }
        let rev = (u32::from(code)).reverse_bits() >> (32 - u32::from(len));
        self.put(rev, len);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

struct Enc {
    code: [u16; 288],
    len: [u8; 288],
}

fn codes_from(lengths: &[u8]) -> Enc {
    let mut counts = [0u16; 16];
    for &len in lengths {
        if len > 0 {
            counts[len as usize] += 1;
        }
    }
    let mut next = [0u16; 16];
    let mut code = 0u16;
    for len in 1..16 {
        code = (code + counts[len - 1]) << 1;
        next[len] = code;
    }
    let mut enc = Enc {
        code: [0; 288],
        len: [0; 288],
    };
    for (sym, &len) in lengths.iter().enumerate() {
        if len == 0 {
            continue;
        }
        enc.code[sym] = next[len as usize];
        enc.len[sym] = len;
        next[len as usize] += 1;
    }
    enc
}

fn fixed_encoder() -> (Enc, Enc) {
    let mut ll = [0u8; 288];
    for (sym, len) in ll.iter_mut().enumerate() {
        *len = match sym {
            0..=143 => 8,
            144..=255 => 9,
            256..=279 => 7,
            _ => 8,
        };
    }
    let mut dd = [0u8; 32];
    for len in &mut dd {
        *len = 5;
    }
    (codes_from(&ll), codes_from(&dd[..32]))
}

fn len_symbol(len: usize) -> (u16, u32, u8) {
    let mut i = 28;
    while i > 0 && (len as u16) < LEN_BASE[i] {
        i -= 1;
    }
    let extra_n = LEN_EXTRA[i];
    let extra = (len - LEN_BASE[i] as usize) as u32;
    (257 + i as u16, extra, extra_n)
}

fn dist_symbol(dist: usize) -> (u16, u32, u8) {
    let mut i = 29;
    while i > 0 && (dist as u16) < DIST_BASE[i] {
        i -= 1;
    }
    let extra_n = DIST_EXTRA[i];
    let extra = (dist - DIST_BASE[i] as usize) as u32;
    (i as u16, extra, extra_n)
}

fn emit_match(w: &mut Lsb, lit: &Enc, dist_enc: &Enc, len: usize, dist: usize) {
    let (ls, lex, ln) = len_symbol(len);
    w.code(lit.code[ls as usize], lit.len[ls as usize]);
    if ln > 0 {
        w.put(lex, ln);
    }
    let (ds, dex, dn) = dist_symbol(dist);
    w.code(dist_enc.code[ds as usize], dist_enc.len[ds as usize]);
    if dn > 0 {
        w.put(dex, dn);
    }
}

/// Zlib-wrap `data` with one fixed-Huffman deflate block.
pub(crate) fn zlib_compress(data: &[u8]) -> Vec<u8> {
    let (lit, dist_enc) = fixed_encoder();
    let mut w = Lsb {
        out: Vec::with_capacity(data.len() / 2 + 64),
        acc: 0,
        n: 0,
    };
    // BFINAL = 1, BTYPE = 01 (fixed).
    w.put(1, 1);
    w.put(1, 2);

    let mut head = [u32::MAX; 1 << 15];
    let mut prev = vec![u32::MAX; data.len()];
    let mut i = 0;
    while i < data.len() {
        let mut matched = 0usize;
        let mut distance = 0usize;
        if i + 3 <= data.len() {
            let h = hash3(data, i);
            let mut cur = head[h];
            let mut chain = 0;
            while cur != u32::MAX && chain < 32 {
                let j = cur as usize;
                if i - j > WINDOW {
                    break;
                }
                let maxl = (data.len() - i).min(MAX_MATCH);
                let mut l = 0;
                while l < maxl && data[j + l] == data[i + l] {
                    l += 1;
                }
                if l >= 3 && l > matched {
                    matched = l;
                    distance = i - j;
                }
                cur = prev[j];
                chain += 1;
            }
            prev[i] = head[h];
            head[h] = i as u32;
        }
        if matched >= 3 {
            emit_match(&mut w, &lit, &dist_enc, matched, distance);
            for k in 1..matched {
                let p = i + k;
                if p + 2 < data.len() {
                    let h = hash3(data, p);
                    prev[p] = head[h];
                    head[h] = p as u32;
                }
            }
            i += matched;
        } else {
            let s = data[i] as usize;
            w.code(lit.code[s], lit.len[s]);
            i += 1;
        }
    }
    w.code(lit.code[256], lit.len[256]);
    let mut out = Vec::with_capacity(w.out.len() + 8);
    out.extend_from_slice(&[0x78, 0x9c]);
    out.extend(w.finish());
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

fn hash3(data: &[u8], i: usize) -> usize {
    ((usize::from(data[i]) << 5) ^ (usize::from(data[i + 1]) << 2) ^ usize::from(data[i + 2]))
        & 0x7fff
}

/// Stored-block zlib wrapper. Used to build filter fixtures whose bytes do not
/// depend on the Huffman writer.
#[cfg(test)]
pub(crate) fn zlib_store(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 16 + data.len() / 65535 * 5);
    out.extend_from_slice(&[0x78, 0x01]);
    if data.is_empty() {
        out.extend_from_slice(&[0x01, 0x00, 0x00, 0xff, 0xff]);
    } else {
        let mut i = 0;
        while i < data.len() {
            let n = (data.len() - i).min(65535);
            let last = i + n == data.len();
            out.push(if last { 0x01 } else { 0x00 });
            out.extend_from_slice(&(n as u16).to_le_bytes());
            out.extend_from_slice(&((n as u16) ^ 0xffff).to_le_bytes());
            out.extend_from_slice(&data[i..i + n]);
            i += n;
        }
    }
    out.extend_from_slice(&adler32(data).to_be_bytes());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn python_zlib_streams() {
        assert!(zlib_inflate(b"not zlib", 10).is_err());
        assert_eq!(zlib_inflate(&hex("789c030000000001"), 8).unwrap(), b"");
        assert_eq!(zlib_inflate(&hex("789c4b040000620062"), 8).unwrap(), b"a");
        assert_eq!(
            zlib_inflate(&hex("789ccb48cdc9c95728cf2fca4901001a0b045d"), 32).unwrap(),
            b"hello world"
        );
        let zeros = hex("789c6360181e000000c80001");
        assert_eq!(zlib_inflate(&zeros, 200).unwrap(), vec![0u8; 200]);
    }

    #[test]
    fn roundtrip_fixed_huffman() {
        let samples: [&[u8]; 4] = [b"", b"a", b"hello world", b"aaaaaaaaaaaaaaaaaaaa"];
        for s in samples {
            let c = zlib_compress(s);
            assert_eq!(zlib_inflate(&c, s.len() + 8).unwrap(), s);
        }
        let mut big = Vec::with_capacity(40_000);
        let mut x = 0x1234_5678u32;
        for _ in 0..40_000 {
            x = x.wrapping_mul(1664525).wrapping_add(1013904223);
            big.push((x >> 24) as u8);
        }
        let c = zlib_compress(&big);
        assert_eq!(zlib_inflate(&c, big.len()).unwrap(), big);
    }

    #[test]
    fn stored_roundtrip_and_limit() {
        let data: Vec<u8> = (0..1000).map(|i| (i % 251) as u8).collect();
        let c = zlib_store(&data);
        assert_eq!(zlib_inflate(&c, data.len()).unwrap(), data);
        assert!(zlib_inflate(&c, data.len() - 1).is_err());
    }
}
