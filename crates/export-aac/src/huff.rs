//! Huffman codes for the one frame we write.
//!
//! Spectral data uses AAC codebook 11 only (unsigned pairs, escape).
//! Scale-factor deltas are always zero, so only that one scale-factor
//! codeword is required. Both values are the ISO/IEC 14496-3 codes so a
//! standard decoder can read the frame. Length 0 means the table is empty.

use crate::bits::{BitReader, BitWriter};
use crate::Error;

/// (code, length) for scale-factor delta 0 (Huffman index 60).
pub const SF_DELTA_ZERO: (u32, u8) = (0x0, 1);

/// Codebook 11 symbols `y * 17 + z` with `y` and `z` in `0..=16`.
/// 16 means an escape (the value is at least 16).
pub static CB11: [(u32, u8); 289] = [
    (0x0, 4),
    (0x6, 5),
    (0x19, 6),
    (0x3d, 7),
    (0x9c, 8),
    (0xc6, 8),
    (0x1a7, 9),
    (0x390, 10),
    (0x3c2, 10),
    (0x3df, 10),
    (0x7e6, 11),
    (0x7f3, 11),
    (0xffb, 12),
    (0x7ec, 11),
    (0xffa, 12),
    (0xffe, 12),
    (0x38e, 10),
    (0x5, 5),
    (0x1, 4),
    (0x8, 5),
    (0x14, 6),
    (0x37, 7),
    (0x42, 7),
    (0x92, 8),
    (0xaf, 8),
    (0x191, 9),
    (0x1a5, 9),
    (0x1b5, 9),
    (0x39e, 10),
    (0x3c0, 10),
    (0x3a2, 10),
    (0x3cd, 10),
    (0x7d6, 11),
    (0xae, 8),
    (0x17, 6),
    (0x7, 5),
    (0x9, 5),
    (0x18, 6),
    (0x39, 7),
    (0x40, 7),
    (0x8e, 8),
    (0xa3, 8),
    (0xb8, 8),
    (0x199, 9),
    (0x1ac, 9),
    (0x1c1, 9),
    (0x3b1, 10),
    (0x396, 10),
    (0x3be, 10),
    (0x3ca, 10),
    (0x9d, 8),
    (0x3c, 7),
    (0x15, 6),
    (0x16, 6),
    (0x1a, 6),
    (0x3b, 7),
    (0x44, 7),
    (0x91, 8),
    (0xa5, 8),
    (0xbe, 8),
    (0x196, 9),
    (0x1ae, 9),
    (0x1b9, 9),
    (0x3a1, 10),
    (0x391, 10),
    (0x3a5, 10),
    (0x3d5, 10),
    (0x94, 8),
    (0x9a, 8),
    (0x36, 7),
    (0x38, 7),
    (0x3a, 7),
    (0x41, 7),
    (0x8c, 8),
    (0x9b, 8),
    (0xb0, 8),
    (0xc3, 8),
    (0x19e, 9),
    (0x1ab, 9),
    (0x1bc, 9),
    (0x39f, 10),
    (0x38f, 10),
    (0x3a9, 10),
    (0x3cf, 10),
    (0x93, 8),
    (0xbf, 8),
    (0x3e, 7),
    (0x3f, 7),
    (0x43, 7),
    (0x45, 7),
    (0x9e, 8),
    (0xa7, 8),
    (0xb9, 8),
    (0x194, 9),
    (0x1a2, 9),
    (0x1ba, 9),
    (0x1c3, 9),
    (0x3a6, 10),
    (0x3a7, 10),
    (0x3bb, 10),
    (0x3d4, 10),
    (0x9f, 8),
    (0x1a0, 9),
    (0x8f, 8),
    (0x8d, 8),
    (0x90, 8),
    (0x98, 8),
    (0xa6, 8),
    (0xb6, 8),
    (0xc4, 8),
    (0x19f, 9),
    (0x1af, 9),
    (0x1bf, 9),
    (0x399, 10),
    (0x3bf, 10),
    (0x3b4, 10),
    (0x3c9, 10),
    (0x3e7, 10),
    (0xa8, 8),
    (0x1b6, 9),
    (0xab, 8),
    (0xa4, 8),
    (0xaa, 8),
    (0xb2, 8),
    (0xc2, 8),
    (0xc5, 8),
    (0x198, 9),
    (0x1a4, 9),
    (0x1b8, 9),
    (0x38c, 10),
    (0x3a4, 10),
    (0x3c4, 10),
    (0x3c6, 10),
    (0x3dd, 10),
    (0x3e8, 10),
    (0xad, 8),
    (0x3af, 10),
    (0x192, 9),
    (0xbd, 8),
    (0xbc, 8),
    (0x18e, 9),
    (0x197, 9),
    (0x19a, 9),
    (0x1a3, 9),
    (0x1b1, 9),
    (0x38d, 10),
    (0x398, 10),
    (0x3b7, 10),
    (0x3d3, 10),
    (0x3d1, 10),
    (0x3db, 10),
    (0x7dd, 11),
    (0xb4, 8),
    (0x3de, 10),
    (0x1a9, 9),
    (0x19b, 9),
    (0x19c, 9),
    (0x1a1, 9),
    (0x1aa, 9),
    (0x1ad, 9),
    (0x1b3, 9),
    (0x38b, 10),
    (0x3b2, 10),
    (0x3b8, 10),
    (0x3ce, 10),
    (0x3e1, 10),
    (0x3e0, 10),
    (0x7d2, 11),
    (0x7e5, 11),
    (0xb7, 8),
    (0x7e3, 11),
    (0x1bb, 9),
    (0x1a8, 9),
    (0x1a6, 9),
    (0x1b0, 9),
    (0x1b2, 9),
    (0x1b7, 9),
    (0x39b, 10),
    (0x39a, 10),
    (0x3ba, 10),
    (0x3b5, 10),
    (0x3d6, 10),
    (0x7d7, 11),
    (0x3e4, 10),
    (0x7d8, 11),
    (0x7ea, 11),
    (0xba, 8),
    (0x7e8, 11),
    (0x3a0, 10),
    (0x1bd, 9),
    (0x1b4, 9),
    (0x38a, 10),
    (0x1c4, 9),
    (0x392, 10),
    (0x3aa, 10),
    (0x3b0, 10),
    (0x3bc, 10),
    (0x3d7, 10),
    (0x7d4, 11),
    (0x7dc, 11),
    (0x7db, 11),
    (0x7d5, 11),
    (0x7f0, 11),
    (0xc1, 8),
    (0x7fb, 11),
    (0x3c8, 10),
    (0x3a3, 10),
    (0x395, 10),
    (0x39d, 10),
    (0x3ac, 10),
    (0x3ae, 10),
    (0x3c5, 10),
    (0x3d8, 10),
    (0x3e2, 10),
    (0x3e6, 10),
    (0x7e4, 11),
    (0x7e7, 11),
    (0x7e0, 11),
    (0x7e9, 11),
    (0x7f7, 11),
    (0x190, 9),
    (0x7f2, 11),
    (0x393, 10),
    (0x1be, 9),
    (0x1c0, 9),
    (0x394, 10),
    (0x397, 10),
    (0x3ad, 10),
    (0x3c3, 10),
    (0x3c1, 10),
    (0x3d2, 10),
    (0x7da, 11),
    (0x7d9, 11),
    (0x7df, 11),
    (0x7eb, 11),
    (0x7f4, 11),
    (0x7fa, 11),
    (0x195, 9),
    (0x7f8, 11),
    (0x3bd, 10),
    (0x39c, 10),
    (0x3ab, 10),
    (0x3a8, 10),
    (0x3b3, 10),
    (0x3b9, 10),
    (0x3d0, 10),
    (0x3e3, 10),
    (0x3e5, 10),
    (0x7e2, 11),
    (0x7de, 11),
    (0x7ed, 11),
    (0x7f1, 11),
    (0x7f9, 11),
    (0x7fc, 11),
    (0x193, 9),
    (0xffd, 12),
    (0x3dc, 10),
    (0x3b6, 10),
    (0x3c7, 10),
    (0x3cc, 10),
    (0x3cb, 10),
    (0x3d9, 10),
    (0x3da, 10),
    (0x7d3, 11),
    (0x7e1, 11),
    (0x7ee, 11),
    (0x7ef, 11),
    (0x7f5, 11),
    (0x7f6, 11),
    (0xffc, 12),
    (0xfff, 12),
    (0x19d, 9),
    (0x1c2, 9),
    (0xb5, 8),
    (0xa1, 8),
    (0x96, 8),
    (0x97, 8),
    (0x95, 8),
    (0x99, 8),
    (0xa0, 8),
    (0xa2, 8),
    (0xac, 8),
    (0xa9, 8),
    (0xb1, 8),
    (0xb3, 8),
    (0xbb, 8),
    (0xc0, 8),
    (0x18f, 9),
    (0x4, 5),
];

struct Node {
    /// Child index, or `u16::MAX` when absent.
    zero: u16,
    one: u16,
    /// Symbol once the code is complete, or -1 on an internal node.
    symbol: i16,
}

fn trie() -> Result<Vec<Node>, Error> {
    let mut nodes = vec![Node {
        zero: u16::MAX,
        one: u16::MAX,
        symbol: -1,
    }];
    for (symbol, &(code, len)) in CB11.iter().enumerate() {
        if len == 0 {
            return Err(Error::Invalid);
        }
        let mut node = 0u16;
        for shift in (0..len).rev() {
            let bit = (code >> shift) & 1;
            let child = if bit == 0 {
                nodes[node as usize].zero
            } else {
                nodes[node as usize].one
            };
            if child == u16::MAX {
                let next = nodes.len();
                if next > u16::MAX as usize {
                    return Err(Error::Invalid);
                }
                nodes.push(Node {
                    zero: u16::MAX,
                    one: u16::MAX,
                    symbol: -1,
                });
                if bit == 0 {
                    nodes[node as usize].zero = next as u16;
                } else {
                    nodes[node as usize].one = next as u16;
                }
                node = next as u16;
            } else {
                node = child;
            }
        }
        if nodes[node as usize].symbol != -1 {
            // Two symbols landed on one code: the table is not prefix-free.
            return Err(Error::Invalid);
        }
        nodes[node as usize].symbol = symbol as i16;
    }
    Ok(nodes)
}

pub fn write_sf_delta_zero(w: &mut BitWriter) -> Result<(), Error> {
    let (code, len) = SF_DELTA_ZERO;
    if len == 0 {
        return Err(Error::Invalid);
    }
    w.put(code, u32::from(len));
    Ok(())
}

pub fn read_sf_delta_zero(r: &mut BitReader<'_>) -> Result<(), Error> {
    let (code, len) = SF_DELTA_ZERO;
    if len == 0 {
        return Err(Error::Invalid);
    }
    let bits = r.read(u32::from(len))?;
    if bits != code {
        return Err(Error::Invalid);
    }
    Ok(())
}

pub fn write_cb11_pair(w: &mut BitWriter, a: i32, b: i32) -> Result<(), Error> {
    let (sym_a, mag_a) = cb11_mag(a)?;
    let (sym_b, mag_b) = cb11_mag(b)?;
    let symbol = sym_a * 17 + sym_b;
    let (code, len) = CB11[symbol as usize];
    if len == 0 {
        return Err(Error::Invalid);
    }
    w.put(code, u32::from(len));
    // Non-zero magnitudes, including escapes, take a sign bit before any
    // escape extension. Both signs come before either escape word.
    if mag_a != 0 {
        w.put(u32::from(a < 0), 1);
    }
    if mag_b != 0 {
        w.put(u32::from(b < 0), 1);
    }
    if sym_a == 16 {
        write_escape(w, mag_a)?;
    }
    if sym_b == 16 {
        write_escape(w, mag_b)?;
    }
    Ok(())
}

pub struct Cb11 {
    nodes: Vec<Node>,
}

impl Cb11 {
    pub fn new() -> Result<Self, Error> {
        Ok(Self { nodes: trie()? })
    }

    pub fn read_pair(&self, r: &mut BitReader<'_>) -> Result<(i32, i32), Error> {
        read_cb11_pair_with(&self.nodes, r)
    }
}

#[cfg(test)]
pub fn read_cb11_pair(r: &mut BitReader<'_>) -> Result<(i32, i32), Error> {
    read_cb11_pair_with(&trie()?, r)
}

fn read_cb11_pair_with(nodes: &[Node], r: &mut BitReader<'_>) -> Result<(i32, i32), Error> {
    let mut node = 0u16;
    let mut symbol = -1i16;
    for _ in 0..32 {
        let bit = r.read_bit()?;
        let next = if bit == 0 {
            nodes[node as usize].zero
        } else {
            nodes[node as usize].one
        };
        if next == u16::MAX {
            return Err(Error::Invalid);
        }
        node = next;
        if nodes[node as usize].symbol >= 0 {
            symbol = nodes[node as usize].symbol;
            break;
        }
    }
    if symbol < 0 {
        return Err(Error::Invalid);
    }
    let mut y = i32::from(symbol) / 17;
    let mut z = i32::from(symbol) % 17;
    let neg_y = y != 0 && r.read_bit()? == 1;
    let neg_z = z != 0 && r.read_bit()? == 1;
    if y == 16 {
        y = read_escape(r)?;
    }
    if z == 16 {
        z = read_escape(r)?;
    }
    if neg_y {
        y = -y;
    }
    if neg_z {
        z = -z;
    }
    Ok((y, z))
}

fn cb11_mag(value: i32) -> Result<(i32, u32), Error> {
    let mag = value.unsigned_abs();
    if mag > 8191 {
        return Err(Error::Invalid);
    }
    let symbol = if mag >= 16 { 16 } else { mag as i32 };
    Ok((symbol, mag))
}

/// AAC escape: `N` ones, a zero, then `N + 4` bits of offset above `2^(N+4)`.
fn write_escape(w: &mut BitWriter, value: u32) -> Result<(), Error> {
    if value < 16 {
        return Err(Error::Invalid);
    }
    let mut n = 0u32;
    while n < 13 && value >= (1u32 << (n + 5)) {
        n += 1;
    }
    if value >= (1u32 << (n + 5)) {
        return Err(Error::Invalid);
    }
    if n > 0 {
        w.put((1u32 << n) - 1, n);
    }
    w.put(0, 1);
    let word = value - (1u32 << (n + 4));
    w.put(word, n + 4);
    Ok(())
}

fn read_escape(r: &mut BitReader<'_>) -> Result<i32, Error> {
    let mut n = 0u32;
    while r.read_bit()? == 1 {
        n += 1;
        if n > 13 {
            return Err(Error::Invalid);
        }
    }
    let word = r.read(n + 4)?;
    let value = (1u32 << (n + 4)) + word;
    Ok(value as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cb11_pairs_round_trip() {
        let samples = [(0, 0), (1, -1), (-4, 7), (16, -16), (31, 100), (-255, 2)];
        let mut w = BitWriter::new();
        for (a, b) in samples {
            write_cb11_pair(&mut w, a, b).unwrap();
        }
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        for (a, b) in samples {
            assert_eq!(read_cb11_pair(&mut r).unwrap(), (a, b));
        }
    }

    #[test]
    fn escape_round_trip() {
        for value in [16u32, 17, 31, 32, 63, 64, 100, 255, 256, 1000, 4096] {
            let mut w = BitWriter::new();
            write_escape(&mut w, value).unwrap();
            let bytes = w.finish();
            let mut r = BitReader::new(&bytes);
            assert_eq!(read_escape(&mut r).unwrap() as u32, value);
        }
    }
}
