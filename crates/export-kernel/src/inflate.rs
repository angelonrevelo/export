//! Raw deflate (RFC 1951). No zlib wrapper.
//!
//! A bit error or a length that runs past the output is an error.
//! This is the checked decoder. It is not a SIMD inflater.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadInput,
}

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
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13, 13,
];
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

struct Bits<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    fn pull(&mut self, n: u32) -> Result<u32, Error> {
        let mut value = 0u32;
        for i in 0..n {
            let pos = self.bit;
            let byte_index = pos / 8;
            if byte_index >= self.data.len() {
                return Err(Error::Truncated);
            }
            let bit = (self.data[byte_index] >> (pos % 8)) & 1;
            value |= u32::from(bit) << i;
            self.bit += 1;
        }
        Ok(value)
    }

    fn align(&mut self) {
        let rem = self.bit % 8;
        if rem != 0 {
            self.bit += 8 - rem;
        }
    }

    fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        self.align();
        let start = self.bit / 8;
        let end = start.checked_add(n).ok_or(Error::BadInput)?;
        if end > self.data.len() {
            return Err(Error::Truncated);
        }
        self.bit = end * 8;
        Ok(&self.data[start..end])
    }
}

struct Huff {
    counts: [u16; 16],
    symbols: Vec<u16>,
}

impl Huff {
    fn from_lengths(lengths: &[u8]) -> Result<Self, Error> {
        let mut counts = [0u16; 16];
        for &len in lengths {
            if len > 15 {
                return Err(Error::BadInput);
            }
            if len > 0 {
                counts[len as usize] += 1;
            }
        }
        let mut symbols = vec![0u16; lengths.len()];
        let mut offs = [0usize; 16];
        let mut sum = 0usize;
        for len in 1..16 {
            offs[len] = sum;
            sum += counts[len] as usize;
        }
        for (sym, &len) in lengths.iter().enumerate() {
            if len > 0 {
                let slot = offs[len as usize];
                symbols[slot] = sym as u16;
                offs[len as usize] = slot + 1;
            }
        }
        symbols.truncate(sum);
        Ok(Self { counts, symbols })
    }

    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16, Error> {
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

pub fn inflate(input: &[u8]) -> Result<Vec<u8>, Error> {
    let mut bits = Bits::new(input);
    let mut out = Vec::new();
    loop {
        let bfinal = bits.pull(1)?;
        let btype = bits.pull(2)?;
        match btype {
            0 => inflate_stored(&mut bits, &mut out)?,
            1 => {
                let lit = Huff::from_lengths(&fixed_lit_lengths())?;
                let dist = Huff::from_lengths(&[5u8; 32])?;
                inflate_codes(&mut bits, &lit, &dist, &mut out)?;
            }
            2 => {
                let (lit, dist) = dynamic(&mut bits)?;
                inflate_codes(&mut bits, &lit, &dist, &mut out)?;
            }
            _ => return Err(Error::BadInput),
        }
        if bfinal == 1 {
            break;
        }
    }
    Ok(out)
}

fn inflate_stored(bits: &mut Bits<'_>, out: &mut Vec<u8>) -> Result<(), Error> {
    bits.align();
    let len = bits.pull(16)? as usize;
    let nlen = bits.pull(16)? as usize;
    if len != (!nlen & 0xFFFF) {
        return Err(Error::BadInput);
    }
    out.extend_from_slice(bits.bytes(len)?);
    Ok(())
}

fn inflate_codes(bits: &mut Bits<'_>, lit: &Huff, dist: &Huff, out: &mut Vec<u8>) -> Result<(), Error> {
    loop {
        let symbol = lit.decode(bits)?;
        match symbol {
            0..=255 => out.push(symbol as u8),
            256 => return Ok(()),
            257..=285 => {
                let idx = (symbol - 257) as usize;
                let length = LEN_BASE[idx] as usize + bits.pull(u32::from(LEN_EXTRA[idx]))? as usize;
                let dsym = dist.decode(bits)? as usize;
                if dsym >= DIST_BASE.len() {
                    return Err(Error::BadInput);
                }
                let distance = DIST_BASE[dsym] as usize + bits.pull(u32::from(DIST_EXTRA[dsym]))? as usize;
                if distance == 0 || distance > out.len() {
                    return Err(Error::BadInput);
                }
                for _ in 0..length {
                    let byte = out[out.len() - distance];
                    out.push(byte);
                }
            }
            _ => return Err(Error::BadInput),
        }
    }
}

fn fixed_lit_lengths() -> [u8; 288] {
    let mut lengths = [0u8; 288];
    for item in lengths.iter_mut().take(144) {
        *item = 8;
    }
    for item in lengths.iter_mut().take(256).skip(144) {
        *item = 9;
    }
    for item in lengths.iter_mut().take(280).skip(256) {
        *item = 7;
    }
    for item in lengths.iter_mut().skip(280) {
        *item = 8;
    }
    lengths
}

fn dynamic(bits: &mut Bits<'_>) -> Result<(Huff, Huff), Error> {
    let hlit = bits.pull(5)? as usize + 257;
    let hdist = bits.pull(5)? as usize + 1;
    let hclen = bits.pull(4)? as usize + 4;
    let mut cl_len = [0u8; 19];
    for i in 0..hclen {
        cl_len[CL_ORDER[i]] = bits.pull(3)? as u8;
    }
    let cl = Huff::from_lengths(&cl_len)?;
    let mut lengths = vec![0u8; hlit + hdist];
    let mut i = 0;
    while i < lengths.len() {
        let sym = cl.decode(bits)?;
        match sym {
            0..=15 => {
                lengths[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(Error::BadInput);
                }
                let rep = 3 + bits.pull(2)? as usize;
                let value = lengths[i - 1];
                if i + rep > lengths.len() {
                    return Err(Error::BadInput);
                }
                for _ in 0..rep {
                    lengths[i] = value;
                    i += 1;
                }
            }
            17 => {
                let rep = 3 + bits.pull(3)? as usize;
                if i + rep > lengths.len() {
                    return Err(Error::BadInput);
                }
                i += rep;
            }
            18 => {
                let rep = 11 + bits.pull(7)? as usize;
                if i + rep > lengths.len() {
                    return Err(Error::BadInput);
                }
                i += rep;
            }
            _ => return Err(Error::BadInput),
        }
    }
    let lit = Huff::from_lengths(&lengths[..hlit])?;
    let dist = Huff::from_lengths(&lengths[hlit..])?;
    Ok((lit, dist))
}

#[cfg(test)]
mod tests {
    use super::{inflate, Error};

    fn decode(hex_input: &str) -> Result<Vec<u8>, Error> {
        let bytes = hex_input
            .as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect::<Vec<_>>();
        inflate(&bytes)
    }

    #[test]
    fn zlib_raw_vectors() {
        assert_eq!(decode("0300").unwrap(), b"");
        assert_eq!(decode("4b0400").unwrap(), b"a");
        assert_eq!(decode("cb48cdc9c90700").unwrap(), b"hello");
        let fox = b"The quick brown fox jumps over the lazy dog. ".repeat(4);
        assert_eq!(
            decode("0bc94855282ccd4cce56482aca2fcf5348cbaf50c82acd2d2856c82f4b2d5228014ae72456552aa4e4a7eb29840c0ec500").unwrap(),
            fox
        );
        assert_eq!(decode("4b4c4a4e1c45a388da0800").unwrap(), b"abc".repeat(200));
        let bytes: Vec<u8> = (0..=255).collect();
        let mut stored = vec![0x01, 0x00, 0x01, 0xff, 0xfe];
        stored.extend_from_slice(&bytes);
        assert_eq!(inflate(&stored).unwrap(), bytes);
    }

    #[test]
    fn flipped_block_errors() {
        let mut hello = hex_bytes("cb48cdc9c90700");
        hello[0] = 0x07;
        assert!(inflate(&hello).is_err());
        assert_eq!(inflate(&[0x05, 0x00]), Err(Error::Truncated));
    }

    fn hex_bytes(hex_input: &str) -> Vec<u8> {
        hex_input
            .as_bytes()
            .chunks(2)
            .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
            .collect()
    }
}
