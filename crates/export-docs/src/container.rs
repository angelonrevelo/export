//! One little-endian segment. Magic, fixed span table, CRC-32 trailer.
//! Writers put bytes in `name.tmp` in the same directory and rename.

use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::crc::crc32;
use crate::error::{Error, Result};

pub const MAGIC: &[u8; 8] = b"EXDOC001";
const OLD: &[u8; 8] = b"EXDOC000";
pub const SPANS: usize = 4;
pub const TABLE_BYTE: usize = SPANS * 16;
pub const HEAD: usize = 8 + TABLE_BYTE;

pub const SPAN_META: usize = 0;
pub const SPAN_BODY: usize = 1;
pub const SPAN_PAGES: usize = 2;
pub const SPAN_NOTE: usize = 3;

pub const META_LEN: usize = 24;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Meta {
    pub source: u16,
    pub target: u16,
    pub flags: u32,
    pub input_bytes: u64,
    pub output_bytes: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Segment {
    pub meta: Meta,
    pub body: Vec<u8>,
    pub pages: Vec<u8>,
}

pub fn encode(meta: &Meta, body: &[u8], pages: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(HEAD + META_LEN + body.len() + pages.len() + 4);
    out.extend_from_slice(MAGIC);
    out.resize(HEAD, 0);
    let meta_off = out.len();
    write_meta(&mut out, meta);
    let body_off = out.len();
    out.extend_from_slice(body);
    let pages_off = out.len();
    out.extend_from_slice(pages);
    let end = out.len();
    put_span(&mut out, SPAN_META, meta_off, META_LEN);
    put_span(&mut out, SPAN_BODY, body_off, body.len());
    put_span(&mut out, SPAN_PAGES, pages_off, pages.len());
    put_span(&mut out, SPAN_NOTE, end, 0);
    let sum = crc32(&out);
    out.extend_from_slice(&sum.to_le_bytes());
    debug_assert_eq!(HEAD, 72);
    debug_assert_eq!(TABLE_BYTE, 64);
    Ok(out)
}

pub fn decode(bytes: &[u8]) -> Result<Segment> {
    if bytes.len() < HEAD + 4 {
        return Err(Error::new("segment: truncated"));
    }
    let magic = &bytes[0..8];
    if magic == OLD {
        return Err(Error::new("rebuild, this is EXDOC000"));
    }
    if magic != MAGIC {
        return Err(Error::new("segment: bad magic"));
    }
    let (body_bytes, crc_bytes) = bytes.split_at(bytes.len() - 4);
    let expect = u32::from_le_bytes(crc_bytes.try_into().unwrap());
    let got = crc32(body_bytes);
    if got != expect {
        return Err(Error::new("checksum mismatch"));
    }
    let spans = read_spans(body_bytes)?;
    if spans[SPAN_META].1 != META_LEN {
        return Err(Error::new("segment: meta span is the wrong size"));
    }
    check_tile(body_bytes.len(), &spans)?;
    let meta_bytes = slice_span(body_bytes, spans[SPAN_META])?;
    let meta = read_meta(meta_bytes)?;
    let body = slice_span(body_bytes, spans[SPAN_BODY])?.to_vec();
    let pages = slice_span(body_bytes, spans[SPAN_PAGES])?.to_vec();
    Ok(Segment { meta, body, pages })
}

fn write_meta(out: &mut Vec<u8>, meta: &Meta) {
    out.extend_from_slice(&meta.source.to_le_bytes());
    out.extend_from_slice(&meta.target.to_le_bytes());
    out.extend_from_slice(&meta.flags.to_le_bytes());
    out.extend_from_slice(&meta.input_bytes.to_le_bytes());
    out.extend_from_slice(&meta.output_bytes.to_le_bytes());
}

fn read_meta(b: &[u8]) -> Result<Meta> {
    if b.len() != META_LEN {
        return Err(Error::new("segment: short meta"));
    }
    Ok(Meta {
        source: u16::from_le_bytes([b[0], b[1]]),
        target: u16::from_le_bytes([b[2], b[3]]),
        flags: u32::from_le_bytes(b[4..8].try_into().unwrap()),
        input_bytes: u64::from_le_bytes(b[8..16].try_into().unwrap()),
        output_bytes: u64::from_le_bytes(b[16..24].try_into().unwrap()),
    })
}

fn put_span(buf: &mut [u8], index: usize, offset: usize, len: usize) {
    let at = 8 + index * 16;
    buf[at..at + 8].copy_from_slice(&(offset as u64).to_le_bytes());
    buf[at + 8..at + 16].copy_from_slice(&(len as u64).to_le_bytes());
}

fn read_spans(bytes: &[u8]) -> Result<[(usize, usize); SPANS]> {
    let mut spans = [(0usize, 0usize); SPANS];
    for (i, span) in spans.iter_mut().enumerate() {
        let at = 8 + i * 16;
        let off = u64::from_le_bytes(bytes[at..at + 8].try_into().unwrap());
        let len = u64::from_le_bytes(bytes[at + 8..at + 16].try_into().unwrap());
        let off = usize::try_from(off).map_err(|_| Error::new("segment: offset does not fit"))?;
        let len = usize::try_from(len).map_err(|_| Error::new("segment: length does not fit"))?;
        let end = off
            .checked_add(len)
            .ok_or_else(|| Error::new("segment: span overflow"))?;
        if off < HEAD || end > bytes.len() {
            return Err(Error::new("segment: span out of range"));
        }
        *span = (off, len);
    }
    Ok(spans)
}

fn slice_span(bytes: &[u8], span: (usize, usize)) -> Result<&[u8]> {
    bytes
        .get(span.0..span.0 + span.1)
        .ok_or_else(|| Error::new("segment: span out of range"))
}

/// Non-empty spans tile `[HEAD, len)` in order. A zero-length span sits on a boundary.
fn check_tile(len: usize, spans: &[(usize, usize); SPANS]) -> Result<()> {
    let mut cursor = HEAD;
    let mut last_off = HEAD;
    for (off, n) in spans {
        if *off < last_off {
            return Err(Error::new("segment: spans are not ascending"));
        }
        last_off = *off;
        if *n == 0 {
            if *off != cursor && *off != len {
                // A zero-length feature may sit at the cursor (where the next
                // byte would go) and nowhere else.
                if *off != cursor {
                    return Err(Error::new("segment: zero span is not on a boundary"));
                }
            }
            continue;
        }
        if *off != cursor {
            return Err(Error::new("segment: spans do not tile"));
        }
        cursor = off + n;
    }
    if cursor != len {
        return Err(Error::new("segment: spans do not tile"));
    }
    Ok(())
}

pub fn segment_name(index: u32) -> Result<String> {
    if index > 9999 {
        return Err(Error::new("segment: index does not fit in four digits"));
    }
    Ok(format!("{index:04}.exd"))
}

pub fn is_segment_name(name: &str) -> bool {
    let b = name.as_bytes();
    b.len() == 8
        && b[4] == b'.'
        && &b[5..] == b"exd"
        && b[..4].iter().all(|c| c.is_ascii_digit())
}

/// Write `NNNN.exd.tmp`, fsync, then rename. A `.tmp` is not a segment.
pub fn write_segment(dir: &Path, index: u32, bytes: &[u8]) -> Result<PathBuf> {
    let name = segment_name(index)?;
    let tmp = dir.join(format!("{name}.tmp"));
    let dst = dir.join(&name);
    {
        let mut f = File::create(&tmp).map_err(|e| Error::new(format!("segment: {e}")))?;
        f.write_all(bytes)
            .map_err(|e| Error::new(format!("segment: {e}")))?;
        f.sync_all()
            .map_err(|e| Error::new(format!("segment: {e}")))?;
    }
    fs::rename(&tmp, &dst).map_err(|e| Error::new(format!("segment: {e}")))?;
    Ok(dst)
}

pub fn list_segments(dir: &Path) -> Result<Vec<PathBuf>> {
    let rd = fs::read_dir(dir).map_err(|e| Error::new(format!("segment: {e}")))?;
    let mut names = Vec::new();
    for ent in rd {
        let ent = ent.map_err(|e| Error::new(format!("segment: {e}")))?;
        let name = ent.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if is_segment_name(name) {
            names.push(name.to_string());
        }
    }
    names.sort();
    Ok(names.into_iter().map(|n| dir.join(n)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn table_byte_and_roundtrip() {
        assert_eq!(TABLE_BYTE, 64);
        assert_eq!(HEAD, 72);
        let meta = Meta {
            source: 1,
            target: 3,
            flags: 0,
            input_bytes: 10,
            output_bytes: 4,
        };
        let bytes = encode(&meta, b"<p>Hi</p>", &[]).unwrap();
        let seg = decode(&bytes).unwrap();
        assert_eq!(seg.meta, meta);
        assert_eq!(seg.body, b"<p>Hi</p>");
        assert!(seg.pages.is_empty());
        let mut bad = bytes.clone();
        let last = bad.len() - 1;
        bad[last] ^= 0xff;
        let err = decode(&bad).unwrap_err();
        assert_eq!(err.message, "checksum mismatch");
    }

    #[test]
    fn old_magic_says_rebuild() {
        let mut bytes = encode(
            &Meta {
                source: 1,
                target: 1,
                flags: 0,
                input_bytes: 0,
                output_bytes: 0,
            },
            b"x",
            &[],
        )
        .unwrap();
        bytes[..8].copy_from_slice(OLD);
        // CRC will also fail if we only change magic; check magic first.
        let err = decode(&bytes).unwrap_err();
        assert_eq!(err.message, "rebuild, this is EXDOC000");
    }

    #[test]
    fn twelve_segments_sort_and_tmp_is_ignored() {
        let n = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("export-docs-seg-{n}"));
        fs::create_dir_all(&dir).unwrap();
        let payload = encode(
            &Meta {
                source: 5,
                target: 5,
                flags: 0,
                input_bytes: 1,
                output_bytes: 1,
            },
            b"p",
            &[],
        )
        .unwrap();
        for i in 0..12 {
            write_segment(&dir, i, &payload).unwrap();
        }
        fs::write(dir.join("0012.exd.tmp"), b"partial").unwrap();
        fs::write(dir.join("nope.txt"), b"no").unwrap();
        let listed = list_segments(&dir).unwrap();
        assert_eq!(listed.len(), 12);
        assert!(listed[0].ends_with("0000.exd"));
        assert!(listed[9].ends_with("0009.exd"));
        assert!(listed[10].ends_with("0010.exd"));
        assert!(listed[11].ends_with("0011.exd"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn truncated_does_not_panic() {
        assert!(decode(b"EXDOC001").is_err());
        assert!(decode(&[]).is_err());
    }
}
