//! A zip reader and a stored-only writer.
//!
//! Writing uses method 0. There is no compressor here, so a file this crate
//! writes does not depend on a deflate guess. Reading accepts method 0 and
//! method 8. Method 8 goes through [`inflate`], which is checked against known
//! streams in the tests. Any other method, zip64, encryption, a size that does
//! not match, or a stream [`inflate`] rejects is refused.

use crate::error::{Error, Result};

const MAX_ENTRY: usize = 64 * 1024 * 1024;

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            if crc & 1 == 1 {
                crc = (crc >> 1) ^ 0xEDB8_8320;
            } else {
                crc >>= 1;
            }
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

const CRC_TABLE: [u32; 256] = crc_table();

pub(crate) fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        let idx = ((crc ^ u32::from(b)) & 0xFF) as usize;
        crc = CRC_TABLE[idx] ^ (crc >> 8);
    }
    !crc
}

struct EntryMeta {
    name: String,
    method: u16,
    crc: u32,
    comp_size: u32,
    uncomp_size: u32,
    offset: u32,
}

pub(crate) struct ZipWriter {
    out: Vec<u8>,
    entries: Vec<EntryMeta>,
}

impl ZipWriter {
    pub(crate) fn new() -> Self {
        Self {
            out: Vec::new(),
            entries: Vec::new(),
        }
    }

    pub(crate) fn add_stored(&mut self, name: &str, data: &[u8]) -> Result<()> {
        self.add(name, 0, data, data)
    }

    fn add(
        &mut self,
        name: &str,
        method: u16,
        uncompressed: &[u8],
        compressed: &[u8],
    ) -> Result<()> {
        if uncompressed.len() > MAX_ENTRY || compressed.len() > MAX_ENTRY {
            return Err(Error::new("zip entry is larger than 64MiB"));
        }
        if self.out.len() > u32::MAX as usize {
            return Err(Error::new("zip64 is refused"));
        }
        let offset = self.out.len() as u32;
        let crc = crc32(uncompressed);
        let name_b = name.as_bytes();
        if name_b.len() > u16::MAX as usize {
            return Err(Error::new("zip name is too long"));
        }
        self.out.extend_from_slice(&0x0403_4B50u32.to_le_bytes());
        self.out.extend_from_slice(&20u16.to_le_bytes());
        self.out.extend_from_slice(&0x0800u16.to_le_bytes());
        self.out.extend_from_slice(&method.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&crc.to_le_bytes());
        self.out
            .extend_from_slice(&(compressed.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(uncompressed.len() as u32).to_le_bytes());
        self.out
            .extend_from_slice(&(name_b.len() as u16).to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(name_b);
        self.out.extend_from_slice(compressed);
        self.entries.push(EntryMeta {
            name: name.to_string(),
            method,
            crc,
            comp_size: compressed.len() as u32,
            uncomp_size: uncompressed.len() as u32,
            offset,
        });
        Ok(())
    }

    pub(crate) fn finish(mut self) -> Result<Vec<u8>> {
        if self.entries.len() > u16::MAX as usize || self.out.len() > u32::MAX as usize {
            return Err(Error::new("zip64 is refused"));
        }
        let cd_start = self.out.len() as u32;
        for entry in &self.entries {
            let name_b = entry.name.as_bytes();
            self.out.extend_from_slice(&0x0201_4B50u32.to_le_bytes());
            self.out.extend_from_slice(&20u16.to_le_bytes());
            self.out.extend_from_slice(&20u16.to_le_bytes());
            self.out.extend_from_slice(&0x0800u16.to_le_bytes());
            self.out.extend_from_slice(&entry.method.to_le_bytes());
            self.out.extend_from_slice(&0u16.to_le_bytes());
            self.out.extend_from_slice(&0u16.to_le_bytes());
            self.out.extend_from_slice(&entry.crc.to_le_bytes());
            self.out.extend_from_slice(&entry.comp_size.to_le_bytes());
            self.out.extend_from_slice(&entry.uncomp_size.to_le_bytes());
            self.out
                .extend_from_slice(&(name_b.len() as u16).to_le_bytes());
            self.out.extend_from_slice(&0u16.to_le_bytes());
            self.out.extend_from_slice(&0u16.to_le_bytes());
            self.out.extend_from_slice(&0u16.to_le_bytes());
            self.out.extend_from_slice(&0u16.to_le_bytes());
            self.out.extend_from_slice(&0u32.to_le_bytes());
            self.out.extend_from_slice(&entry.offset.to_le_bytes());
            self.out.extend_from_slice(name_b);
        }
        let cd_size = self.out.len() as u32 - cd_start;
        let n = self.entries.len() as u16;
        self.out.extend_from_slice(&0x0605_4B50u32.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out.extend_from_slice(&n.to_le_bytes());
        self.out.extend_from_slice(&n.to_le_bytes());
        self.out.extend_from_slice(&cd_size.to_le_bytes());
        self.out.extend_from_slice(&cd_start.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        Ok(self.out)
    }
}

#[derive(Debug)]
pub(crate) struct Part {
    pub(crate) name: String,
    pub(crate) bytes: Vec<u8>,
}

pub(crate) fn unzip(data: &[u8]) -> Result<(Vec<Part>, u64)> {
    if data.len() < 22 {
        return Err(Error::new("zip end of central directory not found"));
    }
    let eocd = find_eocd(data)?;
    let cd_off = u32::from_le_bytes(data[eocd + 16..eocd + 20].try_into().unwrap()) as usize;
    let n = u16::from_le_bytes(data[eocd + 10..eocd + 12].try_into().unwrap()) as usize;
    if cd_off == 0xFFFF_FFFF
        || u32::from_le_bytes(data[eocd + 12..eocd + 16].try_into().unwrap()) == 0xFFFF_FFFF
    {
        return Err(Error::new("zip64 is refused"));
    }
    if cd_off > data.len() {
        return Err(Error::new("zip central directory is truncated"));
    }
    let mut i = cd_off;
    let mut out: Vec<Part> = Vec::with_capacity(n);
    let mut lossy = 0u64;
    for _ in 0..n {
        if i + 46 > data.len() || data[i..i + 4] != [0x50, 0x4B, 0x01, 0x02] {
            return Err(Error::new("zip central directory is truncated"));
        }
        let flags = u16::from_le_bytes(data[i + 8..i + 10].try_into().unwrap());
        if flags & 1 != 0 {
            return Err(Error::new("encrypted zip entries are refused"));
        }
        let method = u16::from_le_bytes(data[i + 10..i + 12].try_into().unwrap());
        let crc = u32::from_le_bytes(data[i + 16..i + 20].try_into().unwrap());
        let comp_size = u32::from_le_bytes(data[i + 20..i + 24].try_into().unwrap()) as usize;
        let uncomp_size = u32::from_le_bytes(data[i + 24..i + 28].try_into().unwrap()) as usize;
        let name_len = u16::from_le_bytes(data[i + 28..i + 30].try_into().unwrap()) as usize;
        let extra_len = u16::from_le_bytes(data[i + 30..i + 32].try_into().unwrap()) as usize;
        let comment_len = u16::from_le_bytes(data[i + 32..i + 34].try_into().unwrap()) as usize;
        let local_off = u32::from_le_bytes(data[i + 42..i + 46].try_into().unwrap()) as usize;
        let name_at = i + 46;
        let extra_at = name_at + name_len;
        let next = extra_at + extra_len + comment_len;
        if next > data.len() {
            return Err(Error::new("zip central directory is truncated"));
        }
        if extra_is_zip64(&data[extra_at..extra_at + extra_len])
            || comp_size == 0xFFFF_FFFF
            || uncomp_size == 0xFFFF_FFFF
        {
            return Err(Error::new("zip64 is refused"));
        }
        if uncomp_size > MAX_ENTRY || comp_size > MAX_ENTRY {
            return Err(Error::new("zip entry is larger than 64MiB"));
        }
        let name_bytes = &data[name_at..name_at + name_len];
        let name = match std::str::from_utf8(name_bytes) {
            Ok(s) => s.to_string(),
            Err(_) => {
                lossy += 1;
                String::from_utf8_lossy(name_bytes).into_owned()
            }
        };
        i = next;
        if name.ends_with('/') && uncomp_size == 0 {
            continue;
        }
        let bytes = read_entry(data, local_off, method, comp_size, uncomp_size, crc)?;
        if out.iter().any(|have| have.name == name) {
            return Err(Error::new(format!("duplicate zip entry \"{name}\"")));
        }
        out.push(Part { name, bytes });
    }
    Ok((out, lossy))
}

fn read_entry(
    data: &[u8],
    local_off: usize,
    method: u16,
    comp_size: usize,
    uncomp_size: usize,
    crc: u32,
) -> Result<Vec<u8>> {
    if local_off + 30 > data.len() || data[local_off..local_off + 4] != [0x50, 0x4B, 0x03, 0x04] {
        return Err(Error::new("zip local header is missing"));
    }
    let name_len =
        u16::from_le_bytes(data[local_off + 26..local_off + 28].try_into().unwrap()) as usize;
    let extra_len =
        u16::from_le_bytes(data[local_off + 28..local_off + 30].try_into().unwrap()) as usize;
    let start = local_off + 30 + name_len + extra_len;
    let end = start
        .checked_add(comp_size)
        .ok_or_else(|| Error::new("zip entry is truncated"))?;
    if end > data.len() {
        return Err(Error::new("zip entry is truncated"));
    }
    let compressed = &data[start..end];
    let plain = match method {
        0 => {
            if comp_size != uncomp_size {
                return Err(Error::new("stored zip entry size does not match"));
            }
            compressed.to_vec()
        }
        8 => inflate_exact(compressed, uncomp_size)?,
        other => {
            return Err(Error::new(format!("zip method {other} is refused")));
        }
    };
    if crc32(&plain) != crc {
        return Err(Error::new("zip crc mismatch"));
    }
    Ok(plain)
}

fn extra_is_zip64(extra: &[u8]) -> bool {
    let mut i = 0;
    while i + 4 <= extra.len() {
        let id = u16::from_le_bytes([extra[i], extra[i + 1]]);
        let size = u16::from_le_bytes([extra[i + 2], extra[i + 3]]) as usize;
        if id == 1 {
            return true;
        }
        i = match i.checked_add(4 + size) {
            Some(n) if n <= extra.len() => n,
            _ => return false,
        };
    }
    false
}

fn find_eocd(data: &[u8]) -> Result<usize> {
    let start = data.len().saturating_sub(22 + 65_535);
    let mut i = data.len() - 22;
    loop {
        if data[i..i + 4] == [0x50, 0x4B, 0x05, 0x06] {
            let comment = u16::from_le_bytes([data[i + 20], data[i + 21]]) as usize;
            if i + 22 + comment == data.len() {
                return Ok(i);
            }
        }
        if i == start {
            break;
        }
        i -= 1;
    }
    Err(Error::new("zip end of central directory not found"))
}

/// Raw deflate (RFC 1951). Used for zip method 8 and nowhere else.
pub(crate) fn inflate(src: &[u8]) -> Result<Vec<u8>> {
    if src.is_empty() {
        return Err(Error::new("deflate stream is truncated"));
    }
    let mut bits = Bits::new(src);
    let mut out = Vec::new();
    loop {
        let bfinal = bits.pull(1)?;
        match bits.pull(2)? {
            0 => stored_block(&mut bits, &mut out)?,
            1 => {
                let lit = fixed_lit();
                let dist = fixed_dist();
                codes(&mut bits, &lit, &dist, &mut out)?;
            }
            2 => {
                let (lit, dist) = dynamic(&mut bits)?;
                codes(&mut bits, &lit, &dist, &mut out)?;
            }
            _ => return Err(Error::new("deflate block type 3 is refused")),
        }
        if bfinal == 1 {
            break;
        }
    }
    if bits.pos != bits.data.len() {
        return Err(Error::new("trailing bytes after deflate stream"));
    }
    if out.len() > MAX_ENTRY {
        return Err(Error::new("zip entry is larger than 64MiB"));
    }
    Ok(out)
}

fn inflate_exact(src: &[u8], expected: usize) -> Result<Vec<u8>> {
    if expected > MAX_ENTRY {
        return Err(Error::new("zip entry is larger than 64MiB"));
    }
    let out = inflate(src)?;
    if out.len() != expected {
        return Err(Error::new(
            "deflate stream does not match the zip entry size",
        ));
    }
    Ok(out)
}

struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    buf: u32,
    nbits: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            buf: 0,
            nbits: 0,
        }
    }

    fn pull(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        while self.nbits < n {
            if self.pos >= self.data.len() {
                return Err(Error::new("deflate stream is truncated"));
            }
            self.buf |= u32::from(self.data[self.pos]) << self.nbits;
            self.pos += 1;
            self.nbits += 8;
        }
        let mask = (1u32 << n) - 1;
        let value = self.buf & mask;
        self.buf >>= n;
        self.nbits -= n;
        Ok(value)
    }

    fn align(&mut self) {
        self.buf = 0;
        self.nbits = 0;
    }

    fn byte(&mut self) -> Result<u8> {
        self.align();
        if self.pos >= self.data.len() {
            return Err(Error::new("deflate stream is truncated"));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }
}

fn stored_block(bits: &mut Bits<'_>, out: &mut Vec<u8>) -> Result<()> {
    let len = u16::from(bits.byte()?) | (u16::from(bits.byte()?) << 8);
    let nlen = u16::from(bits.byte()?) | (u16::from(bits.byte()?) << 8);
    if len ^ nlen != 0xFFFF {
        return Err(Error::new("deflate stored block length is inconsistent"));
    }
    for _ in 0..len {
        out.push(bits.byte()?);
        if out.len() > MAX_ENTRY {
            return Err(Error::new("zip entry is larger than 64MiB"));
        }
    }
    Ok(())
}

struct Huff {
    count: [u16; 16],
    first: [u16; 16],
    index: [usize; 16],
    symbols: Vec<u16>,
}

fn build_huff(lengths: &[u8]) -> Result<Huff> {
    let mut count = [0u16; 16];
    for &len in lengths {
        if len > 15 {
            return Err(Error::new("deflate code length is refused"));
        }
        if len > 0 {
            count[len as usize] += 1;
        }
    }
    let mut left = 1i32;
    for &n in count.iter().skip(1) {
        left <<= 1;
        left -= i32::from(n);
        if left < 0 {
            return Err(Error::new("deflate huffman table is over-subscribed"));
        }
    }
    let mut first = [0u16; 16];
    let mut code = 0u16;
    for len in 1..=15 {
        code = code.wrapping_add(count[len - 1]) << 1;
        first[len] = code;
    }
    let mut index = [0usize; 16];
    let mut sum = 0usize;
    for len in 1..=15 {
        index[len] = sum;
        sum += count[len] as usize;
    }
    let mut symbols = vec![0u16; sum];
    let mut filled = index;
    for (sym, &len) in lengths.iter().enumerate() {
        if len > 0 {
            let slot = filled[len as usize];
            symbols[slot] = sym as u16;
            filled[len as usize] = slot + 1;
        }
    }
    Ok(Huff {
        count,
        first,
        index,
        symbols,
    })
}

impl Huff {
    fn decode(&self, bits: &mut Bits<'_>) -> Result<u16> {
        let mut code = 0u16;
        for len in 1..=15 {
            code = (code << 1) | bits.pull(1)? as u16;
            let count = self.count[len];
            let first = self.first[len];
            if count > 0 && code >= first && u32::from(code) < u32::from(first) + u32::from(count) {
                let slot = self.index[len] + (code - first) as usize;
                return Ok(self.symbols[slot]);
            }
        }
        Err(Error::new("deflate huffman code is refused"))
    }
}

fn fixed_lit() -> Huff {
    let mut lens = vec![8u8; 288];
    for item in lens.iter_mut().take(144) {
        *item = 8;
    }
    for item in lens.iter_mut().take(256).skip(144) {
        *item = 9;
    }
    for item in lens.iter_mut().take(280).skip(256) {
        *item = 7;
    }
    for item in lens.iter_mut().skip(280) {
        *item = 8;
    }
    build_huff(&lens).expect("fixed literal table is well formed")
}

fn fixed_dist() -> Huff {
    build_huff(&[5u8; 32]).expect("fixed distance table is well formed")
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
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];

fn codes(bits: &mut Bits<'_>, lit: &Huff, dist: &Huff, out: &mut Vec<u8>) -> Result<()> {
    loop {
        let sym = lit.decode(bits)?;
        match sym {
            0..=255 => {
                out.push(sym as u8);
                if out.len() > MAX_ENTRY {
                    return Err(Error::new("zip entry is larger than 64MiB"));
                }
            }
            256 => return Ok(()),
            257..=285 => {
                let idx = (sym - 257) as usize;
                let len =
                    usize::from(LEN_BASE[idx]) + bits.pull(u32::from(LEN_EXTRA[idx]))? as usize;
                let dsym = dist.decode(bits)? as usize;
                if dsym >= DIST_BASE.len() {
                    return Err(Error::new("deflate distance code is refused"));
                }
                let distance =
                    usize::from(DIST_BASE[dsym]) + bits.pull(u32::from(DIST_EXTRA[dsym]))? as usize;
                if distance == 0 || distance > out.len() {
                    return Err(Error::new("deflate distance is outside the output"));
                }
                for _ in 0..len {
                    let byte = out[out.len() - distance];
                    out.push(byte);
                    if out.len() > MAX_ENTRY {
                        return Err(Error::new("zip entry is larger than 64MiB"));
                    }
                }
            }
            _ => return Err(Error::new("deflate literal code is refused")),
        }
    }
}

const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

fn dynamic(bits: &mut Bits<'_>) -> Result<(Huff, Huff)> {
    let nlit = bits.pull(5)? as usize + 257;
    let ndist = bits.pull(5)? as usize + 1;
    let nclen = bits.pull(4)? as usize + 4;
    if nlit > 286 || ndist > 32 || nclen > 19 {
        return Err(Error::new("deflate dynamic header is refused"));
    }
    let mut cl_len = [0u8; 19];
    for slot in CL_ORDER.iter().take(nclen) {
        cl_len[*slot] = bits.pull(3)? as u8;
    }
    let cl = build_huff(&cl_len)?;
    let total = nlit + ndist;
    let mut lens = vec![0u8; total];
    let mut i = 0;
    while i < total {
        match cl.decode(bits)? {
            sym @ 0..=15 => {
                lens[i] = sym as u8;
                i += 1;
            }
            16 => {
                if i == 0 {
                    return Err(Error::new("deflate repeat with no previous length"));
                }
                let prev = lens[i - 1];
                let rep = 3 + bits.pull(2)? as usize;
                for _ in 0..rep {
                    if i >= total {
                        return Err(Error::new("deflate code-length run is too long"));
                    }
                    lens[i] = prev;
                    i += 1;
                }
            }
            17 => {
                let rep = 3 + bits.pull(3)? as usize;
                for _ in 0..rep {
                    if i >= total {
                        return Err(Error::new("deflate code-length run is too long"));
                    }
                    lens[i] = 0;
                    i += 1;
                }
            }
            18 => {
                let rep = 11 + bits.pull(7)? as usize;
                for _ in 0..rep {
                    if i >= total {
                        return Err(Error::new("deflate code-length run is too long"));
                    }
                    lens[i] = 0;
                    i += 1;
                }
            }
            _ => return Err(Error::new("deflate code-length symbol is refused")),
        }
    }
    Ok((build_huff(&lens[..nlit])?, build_huff(&lens[nlit..])?))
}

/// One stored deflate block. Tests use this to build a method-8 zip member
/// whose plaintext is known without a compressor.
#[cfg(test)]
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    assert!(data.len() <= 65_535);
    let n = data.len() as u16;
    let mut out = Vec::with_capacity(5 + data.len());
    out.push(0x01);
    out.extend_from_slice(&n.to_le_bytes());
    out.extend_from_slice(&(!n).to_le_bytes());
    out.extend_from_slice(data);
    out
}

#[cfg(test)]
fn zip_member(method: u16, name: &str, uncompressed: &[u8], compressed: &[u8]) -> Result<Vec<u8>> {
    let mut zip = ZipWriter::new();
    zip.add(name, method, uncompressed, compressed)?;
    zip.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn inflate_matches_known_vectors() {
        let cases: &[(&str, &[u8])] = &[("cb48cdc9c90700", b"hello"), ("0300", b"")];
        for (hex, plain) in cases {
            let got = inflate(&decode_hex(hex)).unwrap();
            assert_eq!(&got, plain, "{hex}");
        }
        let abc = b"abc".repeat(50);
        assert_eq!(abc.len(), 150);
        let got = inflate(&decode_hex("4b4c4a4e1c7c0800")).unwrap();
        assert_eq!(got, abc);
        let sheet = decode_hex(
            "b3495628b2557234545228b1552a56b2b329b333b0d12fb3b3d14fb6b301cb391982454d8c60c200",
        );
        let got = inflate(&sheet).unwrap();
        assert_eq!(
            got,
            br#"<c r="A1" t="s"><v>0</v></c><c r="B1"><v>42</v></c>"#
        );
        let utf = decode_hex("4b4e4c3bbc52e151c31485170b570000");
        let got = inflate(&utf).unwrap();
        assert_eq!(got, "café — 表".as_bytes());
    }

    #[test]
    fn stored_deflate_block_round_trips() {
        let plain = b"<c r=\"B1\"><v>42</v></c>";
        let got = inflate(&deflate_stored(plain)).unwrap();
        assert_eq!(got, plain);
    }

    #[test]
    fn method_8_zip_uses_the_same_inflater() {
        let plain = b"hello";
        let compressed = decode_hex("cb48cdc9c90700");
        let file = zip_member(8, "xl/hello.txt", plain, &compressed).unwrap();
        let (entries, lossy) = unzip(&file).unwrap();
        assert_eq!(lossy, 0);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "xl/hello.txt");
        assert_eq!(entries[0].bytes, plain);
    }

    #[test]
    fn other_methods_and_bad_deflate_are_refused() {
        let file = zip_member(99, "a.txt", b"hello", b"hello").unwrap();
        let err = unzip(&file).unwrap_err();
        assert!(err.message().contains("refused"), "{err}");

        let garbage = zip_member(8, "a.txt", b"hello", &[0x00, 0x01, 0x02]).unwrap();
        let err = unzip(&garbage).unwrap_err();
        assert!(
            err.message().contains("deflate")
                || err.message().contains("crc")
                || err.message().contains("match"),
            "{err}"
        );
        assert!(unzip(&garbage).is_err());
    }

    #[test]
    fn stored_round_trip() {
        let mut zip = ZipWriter::new();
        zip.add_stored("xl/workbook.xml", b"<workbook/>").unwrap();
        let file = zip.finish().unwrap();
        let (entries, _) = unzip(&file).unwrap();
        assert_eq!(entries[0].bytes, b"<workbook/>");
    }
}
