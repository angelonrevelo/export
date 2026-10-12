//! Inflate (RFC 1951) and a fixed-Huffman compressor for fixtures.
//! Stored, fixed, and dynamic blocks. Bytes are pulled into the caller buffer.
//! History is a 32 KiB window; the uncompressed document is not retained.

use crate::error::{Error, Result};

const WIN: usize = 32 * 1024;

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

struct BitIn {
    input: Vec<u8>,
    pos: usize,
    acc: u32,
    nbits: u8,
}

impl BitIn {
    fn new(input: Vec<u8>) -> Self {
        Self {
            input,
            pos: 0,
            acc: 0,
            nbits: 0,
        }
    }

    fn bits(&mut self, n: u8) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        if n > 24 {
            return Err(Error::new("deflate: bit width"));
        }
        while self.nbits < n {
            if self.pos >= self.input.len() {
                return Err(Error::new("deflate: truncated bit stream"));
            }
            self.acc |= u32::from(self.input[self.pos]) << self.nbits;
            self.pos += 1;
            self.nbits += 8;
        }
        let mask = (1u32 << n) - 1;
        let v = self.acc & mask;
        self.acc >>= n;
        self.nbits -= n;
        Ok(v)
    }

    fn byte_align(&mut self) {
        let extra = self.nbits % 8;
        if extra != 0 {
            self.acc >>= extra;
            self.nbits -= extra;
        }
    }

    fn next_byte(&mut self) -> Result<u8> {
        if self.nbits >= 8 {
            let b = (self.acc & 0xff) as u8;
            self.acc >>= 8;
            self.nbits -= 8;
            return Ok(b);
        }
        if self.nbits != 0 {
            return Err(Error::new("deflate: align left partial bits"));
        }
        if self.pos >= self.input.len() {
            return Err(Error::new("deflate: truncated stored block"));
        }
        let b = self.input[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn next_u16(&mut self) -> Result<u16> {
        let lo = self.next_byte()?;
        let hi = self.next_byte()?;
        Ok(u16::from_le_bytes([lo, hi]))
    }
}

struct Huff {
    count: [i32; 16],
    symbol: Vec<u16>,
    empty: bool,
}

fn construct(lengths: &[u8]) -> Result<Huff> {
    let mut count = [0i32; 16];
    for &len in lengths {
        if len > 15 {
            return Err(Error::new("deflate: code length > 15"));
        }
        count[len as usize] += 1;
    }
    if count[0] == lengths.len() as i32 {
        return Ok(Huff {
            count,
            symbol: Vec::new(),
            empty: true,
        });
    }
    let mut left = 1i32;
    for count_len in count.iter().skip(1) {
        left <<= 1;
        left -= count_len;
        if left < 0 {
            return Err(Error::new("deflate: over-subscribed huffman tree"));
        }
    }
    let mut offs = [0i32; 16];
    for len in 1..15 {
        offs[len + 1] = offs[len] + count[len];
    }
    let nsym = lengths.len() as i32 - count[0];
    if nsym < 0 {
        return Err(Error::new("deflate: bad huffman lengths"));
    }
    let mut symbol = vec![0u16; nsym as usize];
    for (sym, &len) in lengths.iter().enumerate() {
        if len != 0 {
            let slot = offs[len as usize];
            if slot < 0 || slot as usize >= symbol.len() {
                return Err(Error::new("deflate: huffman symbol out of range"));
            }
            symbol[slot as usize] = sym as u16;
            offs[len as usize] += 1;
        }
    }
    Ok(Huff {
        count,
        symbol,
        empty: false,
    })
}

fn decode(h: &Huff, bits: &mut BitIn) -> Result<u16> {
    if h.empty {
        return Err(Error::new("deflate: empty huffman tree"));
    }
    let mut code: i32 = 0;
    let mut first: i32 = 0;
    let mut index: usize = 0;
    for len in 1..=15 {
        code |= bits.bits(1)? as i32;
        let count = h.count[len];
        if code - count < first {
            let slot = index + (code - first) as usize;
            if slot >= h.symbol.len() {
                return Err(Error::new("deflate: bad huffman code"));
            }
            return Ok(h.symbol[slot]);
        }
        index += count as usize;
        first += count;
        first <<= 1;
        code <<= 1;
    }
    Err(Error::new("deflate: bad huffman code"))
}

fn fixed_lit_lengths() -> Vec<u8> {
    let mut lit = vec![8u8; 288];
    for item in lit.iter_mut().take(256).skip(144) {
        *item = 9;
    }
    for item in lit.iter_mut().take(280).skip(256) {
        *item = 7;
    }
    lit
}

fn fixed_dist_lengths() -> Vec<u8> {
    vec![5u8; 32]
}

enum Mode {
    Header,
    Stored { left: usize },
    Fixed,
    Dynamic,
    Done,
}

/// Pull inflater over one compressed buffer.
pub struct Inflater {
    bits: BitIn,
    window: Box<[u8; WIN]>,
    wpos: usize,
    mode: Mode,
    block_final: bool,
    match_dist: usize,
    match_left: usize,
    fixed_lit: Huff,
    fixed_dist: Huff,
    dyn_lit: Huff,
    dyn_dist: Huff,
}

impl Inflater {
    pub fn new(input: Vec<u8>) -> Result<Self> {
        Ok(Self {
            bits: BitIn::new(input),
            window: Box::new([0u8; WIN]),
            wpos: 0,
            mode: Mode::Header,
            block_final: false,
            match_dist: 0,
            match_left: 0,
            fixed_lit: construct(&fixed_lit_lengths())?,
            fixed_dist: construct(&fixed_dist_lengths())?,
            dyn_lit: Huff {
                count: [0; 16],
                symbol: Vec::new(),
                empty: true,
            },
            dyn_dist: Huff {
                count: [0; 16],
                symbol: Vec::new(),
                empty: true,
            },
        })
    }

    pub fn read(&mut self, dst: &mut [u8]) -> Result<usize> {
        let mut filled = 0;
        while filled < dst.len() {
            if self.match_left > 0 {
                dst[filled] = self.copy_match_byte()?;
                filled += 1;
                continue;
            }
            match self.mode {
                Mode::Done => break,
                Mode::Header => self.read_header()?,
                Mode::Stored { left } => {
                    if left == 0 {
                        self.finish_block();
                        continue;
                    }
                    let b = self.bits.next_byte()?;
                    self.push(b);
                    dst[filled] = b;
                    filled += 1;
                    self.mode = Mode::Stored { left: left - 1 };
                }
                Mode::Fixed | Mode::Dynamic => {
                    self.step_symbol(dst, &mut filled)?;
                }
            }
        }
        Ok(filled)
    }

    fn copy_match_byte(&mut self) -> Result<u8> {
        if self.match_dist == 0 || self.match_dist > WIN {
            return Err(Error::new("deflate: bad distance"));
        }
        let src = (self.wpos + WIN - self.match_dist) & (WIN - 1);
        let b = self.window[src];
        self.push(b);
        self.match_left -= 1;
        Ok(b)
    }

    fn push(&mut self, b: u8) {
        self.window[self.wpos] = b;
        self.wpos = (self.wpos + 1) & (WIN - 1);
    }

    fn finish_block(&mut self) {
        self.mode = if self.block_final {
            Mode::Done
        } else {
            Mode::Header
        };
    }

    fn read_header(&mut self) -> Result<()> {
        let bfinal = self.bits.bits(1)?;
        let btype = self.bits.bits(2)?;
        self.block_final = bfinal == 1;
        match btype {
            0 => {
                self.bits.byte_align();
                let len = usize::from(self.bits.next_u16()?);
                let nlen = self.bits.next_u16()?;
                if (len as u16) != !nlen {
                    return Err(Error::new("deflate: stored length mismatch"));
                }
                if len == 0 {
                    self.finish_block();
                } else {
                    self.mode = Mode::Stored { left: len };
                }
                Ok(())
            }
            1 => {
                self.mode = Mode::Fixed;
                Ok(())
            }
            2 => {
                let (lit, dist) = read_dynamic(&mut self.bits)?;
                self.dyn_lit = lit;
                self.dyn_dist = dist;
                self.mode = Mode::Dynamic;
                Ok(())
            }
            _ => Err(Error::new("deflate: reserved block type")),
        }
    }

    fn step_symbol(&mut self, dst: &mut [u8], filled: &mut usize) -> Result<()> {
        let symbol = match self.mode {
            Mode::Fixed => decode(&self.fixed_lit, &mut self.bits)?,
            Mode::Dynamic => decode(&self.dyn_lit, &mut self.bits)?,
            _ => return Err(Error::new("deflate: internal mode")),
        };
        if symbol < 256 {
            let b = symbol as u8;
            self.push(b);
            dst[*filled] = b;
            *filled += 1;
            return Ok(());
        }
        if symbol == 256 {
            self.finish_block();
            return Ok(());
        }
        if !(257..=285).contains(&symbol) {
            return Err(Error::new("deflate: invalid length symbol"));
        }
        let idx = (symbol - 257) as usize;
        let mut len = usize::from(LEN_BASE[idx]);
        let extra = LEN_EXTRA[idx];
        if extra > 0 {
            len += self.bits.bits(extra)? as usize;
        }
        let dist_sym = match self.mode {
            Mode::Fixed => decode(&self.fixed_dist, &mut self.bits)?,
            Mode::Dynamic => decode(&self.dyn_dist, &mut self.bits)?,
            _ => return Err(Error::new("deflate: internal mode")),
        };
        if dist_sym >= 30 {
            return Err(Error::new("deflate: invalid distance symbol"));
        }
        let mut dist = usize::from(DIST_BASE[dist_sym as usize]);
        let dextra = DIST_EXTRA[dist_sym as usize];
        if dextra > 0 {
            dist += self.bits.bits(dextra)? as usize;
        }
        if dist == 0 || dist > WIN {
            return Err(Error::new("deflate: distance out of window"));
        }
        self.match_dist = dist;
        self.match_left = len;
        Ok(())
    }
}

fn read_dynamic(bits: &mut BitIn) -> Result<(Huff, Huff)> {
    let hlit = bits.bits(5)? as usize + 257;
    let hdist = bits.bits(5)? as usize + 1;
    let hclen = bits.bits(4)? as usize + 4;
    if hlit > 286 || hdist > 32 || hclen > 19 {
        return Err(Error::new("deflate: dynamic header out of range"));
    }
    let mut cl_len = [0u8; 19];
    for i in 0..hclen {
        cl_len[CL_ORDER[i]] = bits.bits(3)? as u8;
    }
    let cl = construct(&cl_len)?;
    let total = hlit + hdist;
    let mut lengths = vec![0u8; total];
    let mut n = 0;
    while n < total {
        let sym = decode(&cl, bits)?;
        match sym {
            0..=15 => {
                lengths[n] = sym as u8;
                n += 1;
            }
            16 => {
                if n == 0 {
                    return Err(Error::new("deflate: repeat with no previous length"));
                }
                let rep = bits.bits(2)? as usize + 3;
                let prev = lengths[n - 1];
                if n + rep > total {
                    return Err(Error::new("deflate: length repeat overrun"));
                }
                for _ in 0..rep {
                    lengths[n] = prev;
                    n += 1;
                }
            }
            17 => {
                let rep = bits.bits(3)? as usize + 3;
                if n + rep > total {
                    return Err(Error::new("deflate: zero repeat overrun"));
                }
                n += rep;
            }
            18 => {
                let rep = bits.bits(7)? as usize + 11;
                if n + rep > total {
                    return Err(Error::new("deflate: long zero repeat overrun"));
                }
                n += rep;
            }
            _ => return Err(Error::new("deflate: bad code-length symbol")),
        }
    }
    let lit = construct(&lengths[..hlit])?;
    let dist = construct(&lengths[hlit..])?;
    Ok((lit, dist))
}

/// Inflate a whole buffer. Used by tests and by ZIP when the caller wants bytes.
pub fn inflate_vec(input: &[u8]) -> Result<Vec<u8>> {
    let mut inf = Inflater::new(input.to_vec())?;
    let mut out = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        let n = inf.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

/// Same bytes as [`inflate_vec`], read one byte at a time (suspend/resume).
#[cfg(test)]
fn inflate_vec_bytewise(input: &[u8]) -> Result<Vec<u8>> {
    let mut inf = Inflater::new(input.to_vec())?;
    let mut out = Vec::new();
    let mut buf = [0u8; 1];
    loop {
        let n = inf.read(&mut buf)?;
        if n == 0 {
            break;
        }
        out.push(buf[0]);
    }
    Ok(out)
}

struct BitOut {
    out: Vec<u8>,
    acc: u32,
    n: u8,
}

impl BitOut {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            n: 0,
        }
    }

    fn bit(&mut self, bit: u32) {
        self.acc |= (bit & 1) << self.n;
        self.n += 1;
        if self.n == 8 {
            self.out.push(self.acc as u8);
            self.acc = 0;
            self.n = 0;
        }
    }

    fn bits_msb(&mut self, code: u32, len: u8) {
        for i in (0..len).rev() {
            self.bit((code >> i) & 1);
        }
    }

    fn bits_lsb(&mut self, val: u32, len: u8) {
        for i in 0..len {
            self.bit((val >> i) & 1);
        }
    }

    fn finish(mut self) -> Vec<u8> {
        if self.n > 0 {
            self.out.push(self.acc as u8);
        }
        self.out
    }
}

fn encode_length(len: usize) -> (u32, u8, u32) {
    let mut idx = LEN_BASE.len() - 1;
    while idx > 0 && (len as u16) < LEN_BASE[idx] {
        idx -= 1;
    }
    let extra_n = LEN_EXTRA[idx];
    let extra = (len as u32) - u32::from(LEN_BASE[idx]);
    (257 + idx as u32, extra_n, extra)
}

fn encode_distance(dist: usize) -> (u32, u8, u32) {
    let mut idx = DIST_BASE.len() - 1;
    while idx > 0 && (dist as u16) < DIST_BASE[idx] {
        idx -= 1;
    }
    let extra_n = DIST_EXTRA[idx];
    let extra = (dist as u32) - u32::from(DIST_BASE[idx]);
    (idx as u32, extra_n, extra)
}

fn hash3(data: &[u8], i: usize) -> usize {
    let v = u32::from(data[i])
        | (u32::from(data[i + 1]) << 8)
        | (u32::from(data[i + 2]) << 16);
    (v.wrapping_mul(0x1e35_a7bd) >> 17) as usize
}

fn find_match(data: &[u8], i: usize, head: &[i32], prev: &[i32]) -> (usize, usize) {
    let h = hash3(data, i);
    let mut node = head[h];
    let mut best_len = 0usize;
    let mut best_dist = 0usize;
    let mut tries = 0u32;
    while node >= 0 && tries < 8 {
        let j = node as usize;
        if j >= i {
            break;
        }
        let dist = i - j;
        if dist == 0 || dist > WIN {
            break;
        }
        let max_len = (data.len() - i).min(258);
        let mut len = 0usize;
        while len < max_len && data[j + len] == data[i + len] {
            len += 1;
        }
        if len > best_len && len >= 3 {
            best_len = len;
            best_dist = dist;
            if len >= 200 {
                break;
            }
        }
        node = prev[j];
        tries += 1;
    }
    (best_len, best_dist)
}

fn note_hash(data: &[u8], i: usize, head: &mut [i32], prev: &mut [i32]) {
    if i + 2 >= data.len() {
        return;
    }
    let h = hash3(data, i);
    prev[i] = head[h];
    head[h] = i as i32;
}

/// Fixed Huffman plus LZ77. The caller checks that inflate returns the same bytes.
pub fn deflate_lz77(data: &[u8]) -> Vec<u8> {
    let lit = canonical_codes(&fixed_lit_lengths());
    let dist_codes = canonical_codes(&fixed_dist_lengths());
    let mut w = BitOut::new();
    w.bit(1);
    w.bit(1);
    w.bit(0);
    if !data.is_empty() {
        let mut head = vec![-1i32; 1 << 15];
        let mut prev = vec![-1i32; data.len()];
        let mut i = 0usize;
        while i < data.len() {
            if i + 2 < data.len() {
                let (mut len, mut dist) = find_match(data, i, &head, &prev);
                note_hash(data, i, &mut head, &mut prev);
                if len >= 3 && i + 3 < data.len() {
                    let (len2, dist2) = find_match(data, i + 1, &head, &prev);
                    if len2 > len + 1 {
                        let (code, bits) = lit[data[i] as usize];
                        w.bits_msb(code, bits);
                        i += 1;
                        note_hash(data, i, &mut head, &mut prev);
                        len = len2;
                        dist = dist2;
                    }
                }
                if len >= 3 {
                    let (sym, extra_n, extra) = encode_length(len);
                    let (code, bits) = lit[sym as usize];
                    w.bits_msb(code, bits);
                    w.bits_lsb(extra, extra_n);
                    let (dsym, dextra_n, dextra) = encode_distance(dist);
                    let (dcode, dbits) = dist_codes[dsym as usize];
                    w.bits_msb(dcode, dbits);
                    w.bits_lsb(dextra, dextra_n);
                    let end = i + len;
                    i += 1;
                    while i < end {
                        note_hash(data, i, &mut head, &mut prev);
                        i += 1;
                    }
                    continue;
                }
            }
            let (code, bits) = lit[data[i] as usize];
            w.bits_msb(code, bits);
            i += 1;
        }
    }
    let (code, bits) = lit[256];
    w.bits_msb(code, bits);
    w.finish()
}

fn canonical_codes(lengths: &[u8]) -> Vec<(u32, u8)> {
    let mut bl_count = [0u32; 16];
    for &len in lengths {
        if len > 0 {
            bl_count[len as usize] += 1;
        }
    }
    let mut next_code = [0u32; 16];
    let mut code = 0u32;
    for bits in 1..=15 {
        code = (code + bl_count[bits - 1]) << 1;
        next_code[bits] = code;
    }
    let mut out = vec![(0u32, 0u8); lengths.len()];
    for (sym, &len) in lengths.iter().enumerate() {
        if len != 0 {
            out[sym] = (next_code[len as usize], len);
            next_code[len as usize] += 1;
        }
    }
    out
}

/// One fixed-Huffman block, no LZ77. Round-trips through [`inflate_vec`].
/// Fixture ZIP bytes use this. PDF shrink does not.
pub fn deflate_fixed(data: &[u8]) -> Vec<u8> {
    let codes = canonical_codes(&fixed_lit_lengths());
    let mut w = BitOut::new();
    w.bit(1); // final
    w.bit(1); // BTYPE = 01, low bit
    w.bit(0);
    for &b in data {
        let (code, len) = codes[b as usize];
        w.bits_msb(code, len);
    }
    let (code, len) = codes[256];
    w.bits_msb(code, len);
    w.finish()
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
    fn stored_hello() {
        let raw = hex("010500faff68656c6c6f");
        assert_eq!(inflate_vec(&raw).unwrap(), b"hello");
        assert_eq!(inflate_vec_bytewise(&raw).unwrap(), b"hello");
    }

    #[test]
    fn zlib_vectors() {
        let cases: &[(&str, &[u8])] = &[
            ("0300", b""),
            ("cb48cdc9c90700", b"hello"),
            ("cb48cdc9c95728cf2fca490100", b"hello world"),
        ];
        for (h, expect) in cases {
            let raw = hex(h);
            assert_eq!(inflate_vec(&raw).unwrap(), *expect, "{h}");
            assert_eq!(inflate_vec_bytewise(&raw).unwrap(), *expect, "{h}");
        }
    }

    #[test]
    fn zlib_all_bytes_and_text() {
        let all = hex(concat!(
            "010001fffe",
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
            "202122232425262728292a2b2c2d2e2f303132333435363738393a3b3c3d3e3f",
            "404142434445464748494a4b4c4d4e4f505152535455565758595a5b5c5d5e5f",
            "606162636465666768696a6b6c6d6e6f707172737475767778797a7b7c7d7e7f",
            "808182838485868788898a8b8c8d8e8f909192939495969798999a9b9c9d9e9f",
            "a0a1a2a3a4a5a6a7a8a9aaabacadaeafb0b1b2b3b4b5b6b7b8b9babbbcbdbebf",
            "c0c1c2c3c4c5c6c7c8c9cacbcccdcecfd0d1d2d3d4d5d6d7d8d9dadbdcdddedf",
            "e0e1e2e3e4e5e6e7e8e9eaebecedeeeff0f1f2f3f4f5f6f7f8f9fafbfcfdfeff"
        ));
        let expect: Vec<u8> = (0..=255).collect();
        assert_eq!(inflate_vec(&all).unwrap(), expect);

        let text = hex("0bc94855282ccd4cce56482aca2fcf5348cbaf50c82acd2d2856c82f4b2d5228014ae72456552aa4e4a7eb29848c2a1e553caa7854f1a86254c500");
        let got = inflate_vec(&text).unwrap();
        assert!(got.starts_with(b"The quick brown fox"));
        assert_eq!(got.len(), 1350);
    }

    #[test]
    fn fixed_compressor_roundtrip() {
        for sample in [b"".as_slice(), b"hello", b"Hello, export.", &[0u8, 255, 10, 13]] {
            let c = deflate_fixed(sample);
            assert_eq!(inflate_vec(&c).unwrap(), sample);
            assert_eq!(inflate_vec_bytewise(&c).unwrap(), sample);
        }
        let mut big = Vec::new();
        for i in 0..5000 {
            big.extend_from_slice(format!("para {i} word\n").as_bytes());
        }
        assert_eq!(inflate_vec(&deflate_fixed(&big)).unwrap(), big);
    }

    #[test]
    fn lz77_roundtrip_and_beats_fixed_on_repeats() {
        for sample in [b"".as_slice(), b"hello", b"abc", &[0u8, 255, 10, 13]] {
            assert_eq!(inflate_vec(&deflate_lz77(sample)).unwrap(), sample);
        }
        let unit = b"BT /F1 12.00 Tf (alpha beta gamma) Tj\n";
        let mut big = Vec::new();
        for _ in 0..400 {
            big.extend_from_slice(unit);
        }
        let packed = deflate_lz77(&big);
        assert_eq!(inflate_vec(&packed).unwrap(), big);
        assert!(packed.len() < big.len());
        assert!(packed.len() < deflate_fixed(&big).len());
    }

    #[test]
    fn truncated_errors() {
        let err = inflate_vec(&[0x01, 0x05, 0x00]).unwrap_err();
        assert!(err.message.contains("deflate"));
    }
}
