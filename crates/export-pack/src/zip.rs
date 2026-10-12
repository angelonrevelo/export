//! Store-method zip (method 0) on write. Payload bytes are copied, not wrapped
//! in a general compressor. Each entry records a CRC-32 of the bytes written.
//! The finished archive appears under its real name only after rename.
//! Method 8 is read with the inflater. The writer still emits store.

use crate::crc::Crc32;
use crate::error::{Error, Result};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Copy buffer. Entry payloads are not held in memory.
pub const CHUNK_BYTES: usize = 64 * 1024;
/// EOCD stores the entry count in a `u16`.
pub const MAX_ENTRIES: usize = 65_535;
/// `0xFFFFFFFF` is the zip64 sentinel, so a store entry stops one byte earlier.
pub const MAX_ENTRY_LEN: u64 = 0xFFFF_FFFE;
/// Central-directory offsets are `u32` until zip64.
pub const MAX_ARCHIVE_LEN: u64 = u32::MAX as u64;

const LOCAL_SIG: u32 = 0x0403_4B50;
const CENTRAL_SIG: u32 = 0x0201_4B50;
const EOCD_SIG: u32 = 0x0605_4B50;
const DESCRIPTOR_SIG: u32 = 0x0807_4B50;
const FLAG_DATA_DESCRIPTOR: u16 = 1 << 3;
const FLAG_UTF8: u16 = 1 << 11;
const VERSION: u16 = 20;
const DOS_TIME: u16 = 0;
const DOS_DATE: u16 = 33;

/// Compression method recorded in the central directory.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    Store,
    Deflate,
    Other(u16),
}

impl Method {
    fn from_u16(value: u16) -> Self {
        match value {
            0 => Self::Store,
            8 => Self::Deflate,
            other => Self::Other(other),
        }
    }
}

/// One central-directory entry. `name()` is the raw archive name.
/// Join it onto a directory only after [`Entry::safe_name`] succeeds.
#[derive(Clone, Debug)]
pub struct Entry {
    name: String,
    crc32: u32,
    size: u64,
    local_offset: u64,
    method: Method,
    flags: u16,
    compressed_size: u64,
}

impl Entry {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn crc32(&self) -> u32 {
        self.crc32
    }

    pub fn size(&self) -> u64 {
        self.size
    }

    pub fn method(&self) -> Method {
        self.method
    }

    pub fn compressed_size(&self) -> u64 {
        self.compressed_size
    }

    /// Name that is safe to join onto an extraction directory.
    pub fn safe_name(&self) -> Result<&str> {
        check_name(&self.name)?;
        Ok(&self.name)
    }
}

struct Central {
    name: String,
    crc32: u32,
    size: u32,
    local_offset: u32,
    flags: u16,
}

/// Writes a store zip to `path.tmp`, then renames it onto `path`.
///
/// Dropping a writer that has not [`finish`](ZipWriter::finish)ed deletes
/// the temporary file. The destination path is never a partial archive.
#[must_use = "call finish() to atomically rename the archive into place"]
pub struct ZipWriter {
    file: Option<File>,
    tmp_path: PathBuf,
    final_path: PathBuf,
    pos: u64,
    buf: Vec<u8>,
    entries: Vec<Central>,
    failed: bool,
    committed: bool,
}

impl ZipWriter {
    /// Create `path.tmp` in the same directory as `path`.
    pub fn create(path: impl AsRef<Path>) -> Result<Self> {
        let final_path = path.as_ref().to_path_buf();
        let tmp_path = sibling_tmp(&final_path);
        let file = File::create(&tmp_path)?;
        Ok(Self {
            file: Some(file),
            tmp_path,
            final_path,
            pos: 0,
            buf: vec![0; CHUNK_BYTES],
            entries: Vec::new(),
            failed: false,
            committed: false,
        })
    }

    /// Stream `reader` into one store entry. Memory stays at one chunk.
    pub fn write_entry<R: Read>(&mut self, name: &str, mut reader: R) -> Result<u64> {
        if self.failed {
            return Err(Error::Format("incomplete entry"));
        }
        check_name(name)?;
        if self.entries.len() >= MAX_ENTRIES {
            return Err(Error::Limit("more than 65535 entries"));
        }
        if self.pos > MAX_ARCHIVE_LEN {
            return self.fail(Error::Limit("archive larger than 4 GiB"));
        }
        let local_offset = u32::try_from(self.pos).unwrap_or(u32::MAX);
        if u64::from(local_offset) != self.pos {
            return self.fail(Error::Limit("archive larger than 4 GiB"));
        }
        let flags = FLAG_DATA_DESCRIPTOR | FLAG_UTF8;
        self.write_local_header(name, flags)?;

        let mut buf = std::mem::take(&mut self.buf);
        let streamed = self.stream_payload(&mut reader, &mut buf);
        self.buf = buf;
        let (size, sum) = streamed?;
        let size_u = u32::try_from(size).unwrap_or(u32::MAX);
        if u64::from(size_u) != size {
            return self.fail(Error::Limit("entry larger than 4 GiB"));
        }
        self.write_descriptor(sum, size_u)?;
        self.entries.push(Central {
            name: name.to_owned(),
            crc32: sum,
            size: size_u,
            local_offset,
            flags,
        });
        Ok(size)
    }

    pub fn write_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<u64> {
        self.write_entry(name, bytes)
    }

    /// Write the central directory, fsync, and rename `path.tmp` onto `path`.
    pub fn finish(mut self) -> Result<()> {
        if self.failed {
            return Err(Error::Format("incomplete entry"));
        }
        self.write_central_directory()?;
        let file = self.file.take().ok_or(Error::Format("archive closed"))?;
        file.sync_all()?;
        drop(file);
        fs::rename(&self.tmp_path, &self.final_path)?;
        sync_parent(&self.final_path);
        self.committed = true;
        Ok(())
    }

    fn write_local_header(&mut self, name: &str, flags: u16) -> Result<()> {
        let mut hdr = [0u8; 30];
        put_u32(&mut hdr[0..4], LOCAL_SIG);
        put_u16(&mut hdr[4..6], VERSION);
        put_u16(&mut hdr[6..8], flags);
        put_u16(&mut hdr[8..10], 0);
        put_u16(&mut hdr[10..12], DOS_TIME);
        put_u16(&mut hdr[12..14], DOS_DATE);
        put_u16(&mut hdr[26..28], name_len_u16(name)?);
        self.emit(&hdr)?;
        self.emit(name.as_bytes())
    }

    fn write_descriptor(&mut self, crc: u32, size: u32) -> Result<()> {
        let mut hdr = [0u8; 16];
        put_u32(&mut hdr[0..4], DESCRIPTOR_SIG);
        put_u32(&mut hdr[4..8], crc);
        put_u32(&mut hdr[8..12], size);
        put_u32(&mut hdr[12..16], size);
        self.emit(&hdr)
    }

    fn write_central_directory(&mut self) -> Result<()> {
        let cd_offset = self.pos;
        let entries = std::mem::take(&mut self.entries);
        let count = entries.len();
        for entry in &entries {
            if let Err(err) = self.emit_central(entry) {
                self.failed = true;
                return Err(err);
            }
        }
        let cd_size = self.pos.saturating_sub(cd_offset);
        if count > MAX_ENTRIES || cd_offset > MAX_ARCHIVE_LEN || cd_size > MAX_ARCHIVE_LEN {
            self.failed = true;
            return Err(Error::Limit("archive larger than 4 GiB"));
        }
        let count_u = u16::try_from(count).unwrap_or(u16::MAX);
        let cd_size_u = u32::try_from(cd_size).unwrap_or(u32::MAX);
        let cd_offset_u = u32::try_from(cd_offset).unwrap_or(u32::MAX);
        self.emit_eocd(count_u, cd_size_u, cd_offset_u)
    }

    fn emit_central(&mut self, entry: &Central) -> Result<()> {
        let mut hdr = [0u8; 46];
        put_u32(&mut hdr[0..4], CENTRAL_SIG);
        put_u16(&mut hdr[4..6], VERSION);
        put_u16(&mut hdr[6..8], VERSION);
        put_u16(&mut hdr[8..10], entry.flags);
        put_u16(&mut hdr[10..12], 0);
        put_u16(&mut hdr[12..14], DOS_TIME);
        put_u16(&mut hdr[14..16], DOS_DATE);
        put_u32(&mut hdr[16..20], entry.crc32);
        put_u32(&mut hdr[20..24], entry.size);
        put_u32(&mut hdr[24..28], entry.size);
        put_u16(&mut hdr[28..30], name_len_u16(&entry.name)?);
        put_u32(&mut hdr[42..46], entry.local_offset);
        self.emit(&hdr)?;
        self.emit(entry.name.as_bytes())
    }

    fn emit_eocd(&mut self, count: u16, cd_size: u32, cd_offset: u32) -> Result<()> {
        let mut hdr = [0u8; 22];
        put_u32(&mut hdr[0..4], EOCD_SIG);
        put_u16(&mut hdr[8..10], count);
        put_u16(&mut hdr[10..12], count);
        put_u32(&mut hdr[12..16], cd_size);
        put_u32(&mut hdr[16..20], cd_offset);
        self.emit(&hdr)
    }

    fn stream_payload<R: Read>(&mut self, reader: &mut R, buf: &mut [u8]) -> Result<(u64, u32)> {
        if buf.is_empty() {
            return self.fail(Error::Format("empty copy buffer"));
        }
        let mut crc = Crc32::new();
        let mut size = 0u64;
        loop {
            let n = match reader.read(buf) {
                Ok(n) => n,
                Err(err) => return self.fail(Error::from(err)),
            };
            if n == 0 {
                break;
            }
            size = match checked_entry_len(size, n) {
                Ok(next) => next,
                Err(err) => return self.fail(err),
            };
            crc.update(&buf[..n]);
            self.emit(&buf[..n])?;
        }
        Ok((size, crc.finish()))
    }

    fn emit(&mut self, bytes: &[u8]) -> Result<()> {
        let Some(file) = self.file.as_mut() else {
            return self.fail(Error::Format("archive closed"));
        };
        if let Err(err) = file.write_all(bytes) {
            return self.fail(Error::from(err));
        }
        let Some(next) = self.pos.checked_add(bytes.len() as u64) else {
            return self.fail(Error::Limit("archive offset overflow"));
        };
        if next > MAX_ARCHIVE_LEN {
            self.pos = next;
            return self.fail(Error::Limit("archive larger than 4 GiB"));
        }
        self.pos = next;
        Ok(())
    }

    fn fail<T>(&mut self, err: Error) -> Result<T> {
        self.failed = true;
        Err(err)
    }
}

impl Drop for ZipWriter {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.file.take();
        let _ = fs::remove_file(&self.tmp_path);
    }
}

struct Eocd {
    total: u16,
    cd_size: u32,
    cd_offset: u32,
    eocd_offset: u64,
}

/// Reads a zip. Entry payloads are streamed out of `copy_entry`.
pub struct ZipReader<R> {
    reader: R,
    entries: Vec<Entry>,
    buf: Vec<u8>,
}

impl<R> std::fmt::Debug for ZipReader<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ZipReader")
            .field("entries", &self.entries.len())
            .finish()
    }
}

impl ZipReader<File> {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let file = File::open(path.as_ref())?;
        Self::from_reader(file)
    }
}

impl<R: Read + Seek> ZipReader<R> {
    pub fn from_reader(mut reader: R) -> Result<Self> {
        let eocd = find_eocd(&mut reader)?;
        let mut zip = Self {
            reader,
            entries: Vec::with_capacity(usize::from(eocd.total)),
            buf: vec![0; CHUNK_BYTES],
        };
        zip.parse_central(&eocd)?;
        Ok(zip)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.entries.iter().position(|entry| entry.name == name)
    }

    /// Stream one entry into `out`, then check its CRC-32.
    ///
    /// On [`Error::ChecksumMismatch`] some payload bytes may already have
    /// been written. Discard `out`. [`Self::extract_entry`] does that itself.
    pub fn copy_entry<W: Write>(&mut self, index: usize, mut out: W) -> Result<u64> {
        let (local_offset, size, expect_crc, method, flags, comp_size) = {
            let entry = self
                .entries
                .get(index)
                .ok_or(Error::Format("entry index out of range"))?;
            (
                entry.local_offset,
                entry.size,
                entry.crc32,
                entry.method,
                entry.flags,
                entry.compressed_size,
            )
        };
        if flags & 1 != 0 {
            return Err(Error::Unsupported("encrypted zip"));
        }
        if method != Method::Store && method != Method::Deflate {
            return Err(Error::Unsupported("zip method is not store"));
        }
        self.reader.seek(SeekFrom::Start(local_offset))?;
        let mut hdr = [0u8; 30];
        read_exact(&mut self.reader, &mut hdr)?;
        if read_u32(&hdr[0..4]) != LOCAL_SIG {
            return Err(Error::Format("bad local header"));
        }
        if Method::from_u16(read_u16(&hdr[8..10])) != method {
            return Err(Error::Format("local method mismatch"));
        }
        let skip = u64::from(read_u16(&hdr[26..28])) + u64::from(read_u16(&hdr[28..30]));
        self.discard(skip)?;
        if method == Method::Deflate {
            return self.copy_deflated(comp_size, size, expect_crc, &mut out);
        }
        let mut crc = Crc32::new();
        let mut left = size;
        while left > 0 {
            let n = left.min(self.buf.len() as u64) as usize;
            read_exact(&mut self.reader, &mut self.buf[..n])?;
            crc.update(&self.buf[..n]);
            out.write_all(&self.buf[..n])?;
            left -= n as u64;
        }
        if crc.finish() != expect_crc {
            return Err(Error::ChecksumMismatch);
        }
        Ok(size)
    }

    fn copy_deflated<W: Write>(
        &mut self,
        comp_size: u64,
        size: u64,
        expect_crc: u32,
        out: &mut W,
    ) -> Result<u64> {
        let mut limited = (&mut self.reader).take(comp_size);
        let mut sink = CrcSink {
            inner: out,
            crc: Crc32::new(),
            n: 0,
        };
        let produced = crate::deflate::inflate_raw(&mut limited, comp_size, &mut sink, size)?;
        if produced != size || sink.n != size {
            return Err(Error::Format("deflate size mismatch"));
        }
        if sink.crc.finish() != expect_crc {
            return Err(Error::ChecksumMismatch);
        }
        Ok(size)
    }

    /// Stream one entry to `dest.tmp`, check the CRC, then rename onto `dest`.
    /// A checksum failure leaves neither file.
    pub fn extract_entry(&mut self, index: usize, dest: impl AsRef<Path>) -> Result<()> {
        let dest = dest.as_ref();
        let tmp = sibling_tmp(dest);
        let copy = {
            let mut file = File::create(&tmp)?;
            let copied = self.copy_entry(index, &mut file);
            if copied.is_ok() {
                file.sync_all()?;
            }
            copied
        };
        match copy {
            Ok(_) => match fs::rename(&tmp, dest) {
                Ok(()) => {
                    sync_parent(dest);
                    Ok(())
                }
                Err(err) => {
                    let _ = fs::remove_file(&tmp);
                    Err(Error::from(err))
                }
            },
            Err(err) => {
                let _ = fs::remove_file(&tmp);
                Err(err)
            }
        }
    }

    fn parse_central(&mut self, eocd: &Eocd) -> Result<()> {
        let cd_offset = u64::from(eocd.cd_offset);
        let cd_size = u64::from(eocd.cd_size);
        let cd_end = cd_offset
            .checked_add(cd_size)
            .ok_or(Error::Format("central directory overruns"))?;
        if cd_end > eocd.eocd_offset {
            return Err(Error::Format("central directory overruns"));
        }
        if eocd.total == 0 && cd_size != 0 {
            return Err(Error::Format("central directory length mismatch"));
        }
        self.reader.seek(SeekFrom::Start(cd_offset))?;
        let mut consumed = 0u64;
        for _ in 0..eocd.total {
            if consumed.checked_add(46).ok_or(Error::Format("truncated"))? > cd_size {
                return Err(Error::Format("central directory truncated"));
            }
            let mut hdr = [0u8; 46];
            read_exact(&mut self.reader, &mut hdr)?;
            consumed += 46;
            if read_u32(&hdr[0..4]) != CENTRAL_SIG {
                return Err(Error::Format("bad central header"));
            }
            let flags = read_u16(&hdr[8..10]);
            let method = read_u16(&hdr[10..12]);
            let crc32 = read_u32(&hdr[16..20]);
            let comp_size = read_u32(&hdr[20..24]);
            let uncomp_size = read_u32(&hdr[24..28]);
            let name_len = u64::from(read_u16(&hdr[28..30]));
            let extra_len = u64::from(read_u16(&hdr[30..32]));
            let comment_len = u64::from(read_u16(&hdr[32..34]));
            let local_offset = read_u32(&hdr[42..46]);
            let rest = name_len
                .checked_add(extra_len)
                .and_then(|n| n.checked_add(comment_len))
                .ok_or(Error::Format("central directory truncated"))?;
            if consumed
                .checked_add(rest)
                .ok_or(Error::Format("truncated"))?
                > cd_size
            {
                return Err(Error::Format("central directory truncated"));
            }
            if name_len > self.buf.len() as u64 {
                return Err(Error::Limit("entry name longer than 64 KiB"));
            }
            let name_n = name_len as usize;
            read_exact(&mut self.reader, &mut self.buf[..name_n])?;
            let name = decode_name(&self.buf[..name_n], flags)?;
            self.discard(extra_len + comment_len)?;
            consumed += rest;
            if comp_size == u32::MAX || uncomp_size == u32::MAX || local_offset == u32::MAX {
                return Err(Error::Unsupported("zip64"));
            }
            if method == 0 && comp_size != uncomp_size {
                return Err(Error::Format("store sizes disagree"));
            }
            self.entries.push(Entry {
                name,
                crc32,
                size: u64::from(uncomp_size),
                local_offset: u64::from(local_offset),
                method: Method::from_u16(method),
                flags,
                compressed_size: u64::from(comp_size),
            });
        }
        if consumed != cd_size {
            return Err(Error::Format("central directory length mismatch"));
        }
        Ok(())
    }

    fn discard(&mut self, mut n: u64) -> Result<()> {
        while n > 0 {
            let take = n.min(self.buf.len() as u64) as usize;
            read_exact(&mut self.reader, &mut self.buf[..take])?;
            n -= take as u64;
        }
        Ok(())
    }
}

fn find_eocd<R: Read + Seek>(reader: &mut R) -> Result<Eocd> {
    let file_len = reader.seek(SeekFrom::End(0))?;
    if file_len < 22 {
        return Err(Error::Format("truncated"));
    }
    let scan = file_len.min(22 + 65_535);
    let start = file_len - scan;
    reader.seek(SeekFrom::Start(start))?;
    let mut buf = vec![0u8; scan as usize];
    read_exact(reader, &mut buf)?;
    let mut i = buf.len() - 22;
    loop {
        if read_u32(&buf[i..i + 4]) == EOCD_SIG {
            let comment_len = usize::from(read_u16(&buf[i + 20..i + 22]));
            if i + 22 + comment_len == buf.len() {
                let disk = read_u16(&buf[i + 4..i + 6]);
                let cd_disk = read_u16(&buf[i + 6..i + 8]);
                let disk_entries = read_u16(&buf[i + 8..i + 10]);
                let total = read_u16(&buf[i + 10..i + 12]);
                if disk != 0 || cd_disk != 0 || disk_entries != total {
                    return Err(Error::Unsupported("split zip"));
                }
                return Ok(Eocd {
                    total,
                    cd_size: read_u32(&buf[i + 12..i + 16]),
                    cd_offset: read_u32(&buf[i + 16..i + 20]),
                    eocd_offset: start + i as u64,
                });
            }
        }
        if i == 0 {
            break;
        }
        i -= 1;
    }
    Err(Error::Format("missing end of central directory"))
}

pub(crate) fn checked_entry_len(current: u64, n: usize) -> Result<u64> {
    let next = current
        .checked_add(n as u64)
        .ok_or(Error::Limit("entry larger than 4 GiB"))?;
    if next > MAX_ENTRY_LEN {
        Err(Error::Limit("entry larger than 4 GiB"))
    } else {
        Ok(next)
    }
}

pub(crate) fn check_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > u16::MAX as usize {
        return Err(Error::UnsafeName);
    }
    if name.as_bytes().contains(&0) || name.contains('\\') || name.starts_with('/') {
        return Err(Error::UnsafeName);
    }
    let mut start = 0usize;
    for (i, byte) in name.bytes().enumerate() {
        if byte == b'/' {
            if !component_ok(&name[start..i]) {
                return Err(Error::UnsafeName);
            }
            start = i + 1;
        }
    }
    if name.ends_with('/') {
        if start != name.len() {
            return Err(Error::UnsafeName);
        }
        return Ok(());
    }
    if component_ok(&name[start..]) {
        Ok(())
    } else {
        Err(Error::UnsafeName)
    }
}

fn component_ok(component: &str) -> bool {
    !component.is_empty() && component != "." && component != ".."
}

fn name_len_u16(name: &str) -> Result<u16> {
    u16::try_from(name.len()).map_err(|_| Error::UnsafeName)
}

fn decode_name(bytes: &[u8], flags: u16) -> Result<String> {
    if flags & FLAG_UTF8 != 0 {
        String::from_utf8(bytes.to_vec()).map_err(|_| Error::Format("entry name is not utf-8"))
    } else {
        Ok(bytes.iter().map(|&byte| char::from(byte)).collect())
    }
}

fn sibling_tmp(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".tmp");
    PathBuf::from(os)
}

fn sync_parent(path: &Path) {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
}

struct CrcSink<'a, W> {
    inner: &'a mut W,
    crc: Crc32,
    n: u64,
}

impl<W: Write> Write for CrcSink<'_, W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write_all(buf)?;
        self.crc.update(buf);
        self.n += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

fn read_exact<R: Read>(reader: &mut R, buf: &mut [u8]) -> Result<()> {
    reader.read_exact(buf)?;
    Ok(())
}

fn put_u16(dst: &mut [u8], value: u16) {
    dst[..2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(dst: &mut [u8], value: u32) {
    dst[..4].copy_from_slice(&value.to_le_bytes());
}

fn read_u16(src: &[u8]) -> u16 {
    u16::from_le_bytes([src[0], src[1]])
}

fn read_u32(src: &[u8]) -> u32 {
    u32::from_le_bytes([src[0], src[1], src[2], src[3]])
}

#[cfg(test)]
mod tests {
    use super::{check_name, checked_entry_len, MAX_ENTRY_LEN};
    use crate::error::Error;

    #[test]
    fn entry_len_stops_before_zip64_sentinel() {
        assert_eq!(checked_entry_len(0, 1).unwrap(), 1);
        assert!(checked_entry_len(MAX_ENTRY_LEN, 1).is_err());
        assert!(checked_entry_len(MAX_ENTRY_LEN - 1, 2).is_err());
        assert!(matches!(
            checked_entry_len(MAX_ENTRY_LEN, 1),
            Err(Error::Limit(_))
        ));
    }

    #[test]
    fn names() {
        assert!(check_name("a.txt").is_ok());
        assert!(check_name("dir/a.txt").is_ok());
        assert!(check_name("dir/").is_ok());
        assert!(check_name("café.txt").is_ok());
        for bad in [
            "",
            "/",
            "../x",
            "foo/../../x",
            "/etc/passwd",
            "a\\b",
            "foo//bar",
            ".",
            "..",
            "foo/./bar",
            "foo/../bar",
        ] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }
}
