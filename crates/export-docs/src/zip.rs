//! ZIP reader (stored and deflate) plus a stored writer for fixtures.
//! Central directory is the index. One entry is streamed; the archive is not unpacked.

use std::io::{Read, Seek, SeekFrom};

use crate::crc::{crc32, Crc32};
use crate::deflate::Inflater;
use crate::error::{Error, Result};

const MAX_COMPRESSED: u64 = 32 * 1024 * 1024;
const CHUNK: usize = 8 * 1024;

#[derive(Debug, Clone)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub flags: u16,
    pub crc: u32,
    pub comp_size: u64,
    pub uncomp_size: u64,
    pub local_off: u64,
}

#[derive(Debug, Clone)]
pub struct ArchiveIndex {
    pub entries: Vec<Entry>,
}

impl ArchiveIndex {
    pub fn read<R: Read + Seek>(r: &mut R) -> Result<Self> {
        let len = r.seek(SeekFrom::End(0))?;
        let tail_len = (len.min(22 + 65535)) as usize;
        r.seek(SeekFrom::End(-(tail_len as i64)))?;
        let mut tail = vec![0u8; tail_len];
        r.read_exact(&mut tail)?;
        let eocd = find_eocd(&tail).ok_or_else(|| Error::new("zip: end of central directory not found"))?;
        let eocd_pos = len - tail_len as u64 + eocd as u64;
        if eocd + 22 > tail.len() {
            return Err(Error::new("zip: truncated end of central directory"));
        }
        let disk_entries = u16::from_le_bytes([tail[eocd + 8], tail[eocd + 9]]);
        let total_entries = u16::from_le_bytes([tail[eocd + 10], tail[eocd + 11]]);
        let cd_size = u32::from_le_bytes(tail[eocd + 12..eocd + 16].try_into().unwrap()) as u64;
        let cd_off = u32::from_le_bytes(tail[eocd + 16..eocd + 20].try_into().unwrap()) as u64;
        let comment_len = u16::from_le_bytes([tail[eocd + 20], tail[eocd + 21]]) as u64;
        if disk_entries != total_entries {
            return Err(Error::new("zip: multi-disk archives are not supported"));
        }
        if cd_off == 0xFFFF_FFFF || cd_size == 0xFFFF_FFFF || total_entries == 0xFFFF {
            return Err(Error::new("zip: zip64 is not supported"));
        }
        if eocd_pos + 22 + comment_len != len && comment_len > 0 {
            // Comment length must fit. A mismatch is a truncated or corrupted tail.
            if eocd_pos + 22 + comment_len > len {
                return Err(Error::new("zip: comment exceeds file"));
            }
        }
        let mut cd = vec![0u8; cd_size as usize];
        r.seek(SeekFrom::Start(cd_off))?;
        r.read_exact(&mut cd)?;
        let mut entries = Vec::with_capacity(total_entries as usize);
        let mut i = 0usize;
        for _ in 0..total_entries {
            if i + 46 > cd.len() {
                return Err(Error::new("zip: central directory truncated"));
            }
            if u32::from_le_bytes(cd[i..i + 4].try_into().unwrap()) != 0x0201_4b50 {
                return Err(Error::new("zip: bad central directory signature"));
            }
            let flags = u16::from_le_bytes([cd[i + 8], cd[i + 9]]);
            let method = u16::from_le_bytes([cd[i + 10], cd[i + 11]]);
            let crc = u32::from_le_bytes(cd[i + 16..i + 20].try_into().unwrap());
            let comp_size = u32::from_le_bytes(cd[i + 20..i + 24].try_into().unwrap()) as u64;
            let uncomp_size = u32::from_le_bytes(cd[i + 24..i + 28].try_into().unwrap()) as u64;
            let name_len = u16::from_le_bytes([cd[i + 28], cd[i + 29]]) as usize;
            let extra_len = u16::from_le_bytes([cd[i + 30], cd[i + 31]]) as usize;
            let comment_len = u16::from_le_bytes([cd[i + 32], cd[i + 33]]) as usize;
            let local_off = u32::from_le_bytes(cd[i + 42..i + 46].try_into().unwrap()) as u64;
            let name_off = i + 46;
            let name_end = name_off
                .checked_add(name_len)
                .ok_or_else(|| Error::new("zip: name length overflow"))?;
            if name_end > cd.len() {
                return Err(Error::new("zip: name exceeds central directory"));
            }
            let name = String::from_utf8_lossy(&cd[name_off..name_end]).into_owned();
            let next = name_end
                .checked_add(extra_len)
                .and_then(|n| n.checked_add(comment_len))
                .ok_or_else(|| Error::new("zip: extra length overflow"))?;
            if next > cd.len() {
                return Err(Error::new("zip: extra exceeds central directory"));
            }
            if comp_size == 0xFFFF_FFFF || uncomp_size == 0xFFFF_FFFF || local_off == 0xFFFF_FFFF {
                return Err(Error::new("zip: zip64 is not supported"));
            }
            entries.push(Entry {
                name,
                method,
                flags,
                crc,
                comp_size,
                uncomp_size,
                local_off,
            });
            i = next;
        }
        Ok(Self { entries })
    }

    pub fn by_name(&self, name: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.name == name)
    }

    /// Stream one entry's uncompressed bytes. `scratch_peak` is the high water of
    /// buffers this call held (chunk, or the compressed entry plus the inflate window).
    #[cfg(test)]
    pub fn stream<R, F>(&self, r: &mut R, name: &str, mut emit: F) -> Result<usize>
    where
        R: Read + Seek,
        F: FnMut(&[u8]) -> Result<()>,
    {
        let entry = self
            .by_name(name)
            .ok_or_else(|| Error::new(format!("zip: missing {name}")))?
            .clone();
        if entry.flags & 1 != 0 {
            return Err(Error::new("zip: encrypted entries are not supported"));
        }
        if entry.comp_size > MAX_COMPRESSED {
            return Err(Error::new("zip: compressed entry exceeds bound"));
        }
        let data_off = local_data_offset(r, entry.local_off)?;
        r.seek(SeekFrom::Start(data_off))?;
        let mut crc = Crc32::new();
        let scratch = match entry.method {
            0 => {
                let mut left = entry.comp_size;
                let mut buf = [0u8; CHUNK];
                while left > 0 {
                    let n = CHUNK.min(left as usize);
                    r.read_exact(&mut buf[..n])?;
                    crc.update(&buf[..n]);
                    emit(&buf[..n])?;
                    left -= n as u64;
                }
                CHUNK
            }
            8 => {
                let mut comp = vec![0u8; entry.comp_size as usize];
                r.read_exact(&mut comp)?;
                let mut inf = Inflater::new(comp)?;
                let comp_held = entry.comp_size as usize;
                let mut buf = [0u8; CHUNK];
                loop {
                    let n = inf.read(&mut buf)?;
                    if n == 0 {
                        break;
                    }
                    crc.update(&buf[..n]);
                    emit(&buf[..n])?;
                }
                comp_held + 32 * 1024 + CHUNK
            }
            other => {
                return Err(Error::new(format!("zip: unsupported method {other}")));
            }
        };
        let got = crc.finish();
        if got != entry.crc {
            return Err(Error::new(format!(
                "zip: checksum mismatch for {name} (got {got:08x}, expected {:08x})",
                entry.crc
            )));
        }
        Ok(scratch)
    }
}

fn local_data_offset<R: Read + Seek>(r: &mut R, local_off: u64) -> Result<u64> {
    r.seek(SeekFrom::Start(local_off))?;
    let mut hdr = [0u8; 30];
    r.read_exact(&mut hdr)?;
    if u32::from_le_bytes(hdr[0..4].try_into().unwrap()) != 0x0403_4b50 {
        return Err(Error::new("zip: bad local header signature"));
    }
    let name_len = u16::from_le_bytes([hdr[26], hdr[27]]) as u64;
    let extra_len = u16::from_le_bytes([hdr[28], hdr[29]]) as u64;
    Ok(local_off + 30 + name_len + extra_len)
}

fn find_eocd(tail: &[u8]) -> Option<usize> {
    if tail.len() < 22 {
        return None;
    }
    let mut i = tail.len() - 22;
    loop {
        if tail[i] == 0x50 && tail[i + 1] == 0x4b && tail[i + 2] == 0x05 && tail[i + 3] == 0x06 {
            let comment = u16::from_le_bytes([tail[i + 20], tail[i + 21]]) as usize;
            if i + 22 + comment == tail.len() {
                return Some(i);
            }
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    None
}

/// Stored-method ZIP. Fixture builder and a writer the tests can round-trip.
pub fn pack_stored(files: &[(&str, &[u8])]) -> Vec<u8> {
    pack(files, false)
}

/// Deflate-method ZIP using fixed Huffman blocks (no LZ77).
pub fn pack_deflated(files: &[(&str, &[u8])]) -> Vec<u8> {
    pack(files, true)
}

fn pack(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
    let mut local = Vec::new();
    let mut central = Vec::new();
    for (name, data) in files {
        let off = u32::try_from(local.len()).expect("zip fixture offset");
        let crc = crc32(data);
        let payload = if deflate {
            crate::deflate::deflate_fixed(data)
        } else {
            data.to_vec()
        };
        let meta = ZipMeta {
            method: if deflate { 8 } else { 0 },
            crc,
            comp: u32::try_from(payload.len()).expect("zip fixture size"),
            uncomp: u32::try_from(data.len()).expect("zip fixture size"),
            name: name.as_bytes(),
        };
        write_local(&mut local, &meta, &payload);
        write_central(&mut central, &meta, off);
    }
    let cd_off = u32::try_from(local.len()).expect("cd off");
    let cd_len = u32::try_from(central.len()).expect("cd len");
    let n = u16::try_from(files.len()).expect("entry count");
    local.extend_from_slice(&central);
    local.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local.extend_from_slice(&n.to_le_bytes());
    local.extend_from_slice(&n.to_le_bytes());
    local.extend_from_slice(&cd_len.to_le_bytes());
    local.extend_from_slice(&cd_off.to_le_bytes());
    local.extend_from_slice(&0u16.to_le_bytes());
    local
}

struct ZipMeta<'a> {
    method: u16,
    crc: u32,
    comp: u32,
    uncomp: u32,
    name: &'a [u8],
}

fn write_local(out: &mut Vec<u8>, meta: &ZipMeta<'_>, payload: &[u8]) {
    let name_len = u16::try_from(meta.name.len()).expect("zip fixture name");
    out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&meta.method.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&meta.crc.to_le_bytes());
    out.extend_from_slice(&meta.comp.to_le_bytes());
    out.extend_from_slice(&meta.uncomp.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(meta.name);
    out.extend_from_slice(payload);
}

fn write_central(out: &mut Vec<u8>, meta: &ZipMeta<'_>, off: u32) {
    let name_len = u16::try_from(meta.name.len()).expect("zip fixture name");
    out.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&20u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&meta.method.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&meta.crc.to_le_bytes());
    out.extend_from_slice(&meta.comp.to_le_bytes());
    out.extend_from_slice(&meta.uncomp.to_le_bytes());
    out.extend_from_slice(&name_len.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&off.to_le_bytes());
    out.extend_from_slice(meta.name);
}

/// A `Read` over one entry. Stored entries are pulled from the file in the
/// caller's buffer. Deflate entries keep the compressed bytes plus a 32 KiB window.
pub struct EntryReader<'a, R> {
    inner: &'a mut R,
    mode: EntryMode,
    crc: Crc32,
    expect_crc: u32,
    saw_eof: bool,
    scratch: usize,
}

enum EntryMode {
    Stored { left: u64 },
    Deflate { inf: Box<Inflater> },
}

impl<R> EntryReader<'_, R> {
    pub fn scratch_peak(&self) -> usize {
        self.scratch
    }
}

impl<R: Read> Read for EntryReader<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let n = match &mut self.mode {
            EntryMode::Stored { left } => {
                if *left == 0 {
                    0
                } else {
                    let want = buf.len().min(*left as usize);
                    let n = self.inner.read(&mut buf[..want])?;
                    if n == 0 && *left > 0 {
                        return Err(std::io::Error::other("zip: truncated stored entry"));
                    }
                    *left -= n as u64;
                    n
                }
            }
            EntryMode::Deflate { inf } => inf
                .read(buf)
                .map_err(|e| std::io::Error::other(e.message.clone()))?,
        };
        if n == 0 {
            if !self.saw_eof {
                self.saw_eof = true;
                let got = self.crc.clone().finish();
                if got != self.expect_crc {
                    return Err(std::io::Error::other(format!(
                        "zip: checksum mismatch (got {got:08x}, expected {:08x})",
                        self.expect_crc
                    )));
                }
            }
            return Ok(0);
        }
        self.crc.update(&buf[..n]);
        Ok(n)
    }
}

pub fn open_entry<'a, R: Read + Seek>(
    r: &'a mut R,
    index: &ArchiveIndex,
    name: &str,
) -> Result<EntryReader<'a, R>> {
    let entry = index
        .by_name(name)
        .ok_or_else(|| Error::new(format!("zip: missing {name}")))?
        .clone();
    if entry.flags & 1 != 0 {
        return Err(Error::new("zip: encrypted entries are not supported"));
    }
    if entry.comp_size > MAX_COMPRESSED {
        return Err(Error::new("zip: compressed entry exceeds bound"));
    }
    // The declared uncompressed size is not a buffer. Stored entries stream;
    // deflate keeps only the compressed bytes plus a 32 KiB window.
    let _declared_uncompressed = entry.uncomp_size;
    let data_off = local_data_offset(r, entry.local_off)?;
    r.seek(SeekFrom::Start(data_off))
        .map_err(|e| Error::new(format!("zip: {e}")))?;
    let (mode, scratch) = match entry.method {
        0 => (
            EntryMode::Stored {
                left: entry.comp_size,
            },
            CHUNK,
        ),
        8 => {
            let mut comp = vec![0u8; entry.comp_size as usize];
            r.read_exact(&mut comp)
                .map_err(|e| Error::new(format!("zip: {e}")))?;
            let scratch = comp.len() + 32 * 1024;
            (
                EntryMode::Deflate {
                    inf: Box::new(Inflater::new(comp)?),
                },
                scratch,
            )
        }
        other => return Err(Error::new(format!("zip: unsupported method {other}"))),
    };
    Ok(EntryReader {
        inner: r,
        mode,
        crc: Crc32::new(),
        expect_crc: entry.crc,
        saw_eof: false,
        scratch,
    })
}

/// Read an entry fully. Fine for small fixtures; document conversion streams.
#[cfg(test)]
fn read_entry<R: Read + Seek>(r: &mut R, name: &str) -> Result<Vec<u8>> {
    let idx = ArchiveIndex::read(r)?;
    let mut out = Vec::new();
    idx.stream(r, name, |chunk| {
        out.extend_from_slice(chunk);
        Ok(())
    })?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn stored_and_deflated_roundtrip() {
        let files = [("word/document.xml", &b"<w:t>Hi</w:t>"[..]), ("[Content_Types].xml", &b"<t/>"[..])];
        for bytes in [pack_stored(&files), pack_deflated(&files)] {
            let mut c = Cursor::new(bytes);
            let got = read_entry(&mut c, "word/document.xml").unwrap();
            assert_eq!(got, b"<w:t>Hi</w:t>");
        }
    }

    #[test]
    fn bad_crc_errors() {
        let mut z = pack_stored(&[("a.txt", b"hello")]);
        // Flip a payload byte (after the local header + name).
        let pos = 30 + "a.txt".len();
        z[pos] ^= 0xff;
        let mut c = Cursor::new(z);
        let err = read_entry(&mut c, "a.txt").unwrap_err();
        assert!(err.message.contains("checksum mismatch"), "{}", err.message);
    }
}
