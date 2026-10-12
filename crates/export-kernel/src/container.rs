//! Little-endian box. A client can read the section table before it knows the body.
//!
//! ```text
//! 0..8     magic EXPBOX01
//! 8        version
//! 9        flags
//! 10..12   section count (u16 le)
//! 12..16   reserved
//! 16..     count × 20-byte rows: id u16, pad u16, offset u64, length u64
//! body
//! last 4   CRC-32 over every preceding byte
//! ```
//!
//! A zero length means the feature is off. A bad CRC or a short file is an error,
//! not a plausible document.

use crate::crc32::crc32;

pub const MAGIC: &[u8; 8] = b"EXPBOX01";
pub const VERSION: u8 = 1;
const PREFIX: usize = 16;
const ROW: usize = 20;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    pub id: u16,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadMagic,
    BadVersion,
    BadCrc,
    BadSection,
}

pub fn write_box(sections: &[Section]) -> Vec<u8> {
    let count = sections.len();
    let table = count * ROW;
    let mut body_len = 0usize;
    for section in sections {
        body_len += section.bytes.len();
    }
    let mut out = Vec::with_capacity(PREFIX + table + body_len + 4);
    out.extend_from_slice(MAGIC);
    out.push(VERSION);
    out.push(0);
    out.extend_from_slice(&(count as u16).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());

    let mut cursor = PREFIX + table;
    for section in sections {
        out.extend_from_slice(&section.id.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        if section.bytes.is_empty() {
            out.extend_from_slice(&0u64.to_le_bytes());
            out.extend_from_slice(&0u64.to_le_bytes());
        } else {
            out.extend_from_slice(&(cursor as u64).to_le_bytes());
            out.extend_from_slice(&(section.bytes.len() as u64).to_le_bytes());
            cursor += section.bytes.len();
        }
    }
    for section in sections {
        out.extend_from_slice(&section.bytes);
    }
    let sum = crc32(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    out
}

pub fn read_box(bytes: &[u8]) -> Result<Vec<Section>, Error> {
    if bytes.len() < PREFIX + 4 {
        return Err(Error::Truncated);
    }
    let (body, crc_bytes) = bytes.split_at(bytes.len() - 4);
    let expect = u32::from_le_bytes(crc_bytes.try_into().unwrap());
    if crc32(body) != expect {
        return Err(Error::BadCrc);
    }
    if body.len() < 8 || &body[0..8] != MAGIC {
        return Err(Error::BadMagic);
    }
    if body[8] != VERSION {
        return Err(Error::BadVersion);
    }
    let count = u16::from_le_bytes([body[10], body[11]]) as usize;
    let table_end = PREFIX + count * ROW;
    if body.len() < table_end {
        return Err(Error::Truncated);
    }
    let mut sections = Vec::with_capacity(count);
    for i in 0..count {
        let row = PREFIX + i * ROW;
        let id = u16::from_le_bytes([body[row], body[row + 1]]);
        let offset = u64::from_le_bytes(body[row + 4..row + 12].try_into().unwrap()) as usize;
        let len = u64::from_le_bytes(body[row + 12..row + 20].try_into().unwrap()) as usize;
        if len == 0 {
            sections.push(Section {
                id,
                bytes: Vec::new(),
            });
            continue;
        }
        let end = offset.checked_add(len).ok_or(Error::BadSection)?;
        if offset < table_end || end > body.len() {
            return Err(Error::BadSection);
        }
        sections.push(Section {
            id,
            bytes: body[offset..end].to_vec(),
        });
    }
    Ok(sections)
}

#[cfg(test)]
mod tests {
    use super::{read_box, write_box, Error, Section, MAGIC};

    #[test]
    fn round_trip_and_empty_section() {
        let sections = vec![
            Section {
                id: 1,
                bytes: b"hello".to_vec(),
            },
            Section {
                id: 7,
                bytes: Vec::new(),
            },
            Section {
                id: 3,
                bytes: vec![0, 1, 2, 255],
            },
        ];
        let bytes = write_box(&sections);
        assert_eq!(&bytes[0..8], MAGIC);
        assert_eq!(read_box(&bytes).unwrap(), sections);
    }

    #[test]
    fn truncated_and_flipped_bytes_error() {
        let bytes = write_box(&[Section {
            id: 1,
            bytes: b"payload-that-is-long-enough".to_vec(),
        }]);
        assert_eq!(read_box(&bytes[..12]), Err(Error::Truncated));
        let mut flipped = bytes.clone();
        let last = flipped.len() - 5;
        flipped[last] ^= 0xFF;
        assert_eq!(read_box(&flipped), Err(Error::BadCrc));
    }
}
