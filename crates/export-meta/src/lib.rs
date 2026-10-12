//! Portable metadata record (`EXMETA01`).
//!
//! Filesystem modified time, birth time, Finder comments and tags do not
//! survive a browser download. This crate stores them in the file itself:
//! a fixed little-endian head, then the comment / tags / extra bytes, then
//! the untouched payload.
//!
//! # Head, fetched blind
//!
//! | bytes | field |
//! |---|---|
//! | 0..8 | magic `EXMETA01` |
//! | 8..10 | flags (`1` = created time is present) |
//! | 10..12 | reserved, must be `0` |
//! | 12..16 | `header_len` `u32`, payload starts here |
//! | 16..24 | `body_len` `u64` |
//! | 24..32 | modified time, Unix nanoseconds, `i64` |
//! | 32..40 | created time, Unix nanoseconds, `i64` (0 when the flag is clear) |
//! | 40..44 | CRC-32 of the payload |
//! | 44..48 | CRC-32 of the bytes `[HEAD, header_len)` |
//! | 48..52 | CRC-32 of these `HEAD` bytes, with this field read as zero |
//! | 52..80 | comment / tags / extras spans: `(off, len)` `u32` pairs, then 4 reserved bytes |
//!
//! `HEAD` is 80. A reader fetches 80 bytes, checks the fixed checksum, and
//! only then trusts `header_len`. The payload offset does not move when
//! comment, tags, or extras are cleared, so [`scrub`] rewrites the head and
//! the variable region and does not read the payload.
//!
//! # What a bad file returns
//!
//! Truncation, a wrong magic, a checksum miss, and a span that does not
//! tile the header are [`Error`]. They are not an empty record. A single
//! bit flipped in the head fails the fixed checksum before `header_len` is
//! used as a length. A single bit flipped in the payload fails [`read`] and
//! [`verify`].
//!
//! # Honest limits
//!
//! - CRC-32 catches accidental damage. It is not an authenticator.
//! - [`scrub`] checks the header and the file length. It does not read the
//!   payload, so a payload bit-flip still passes `scrub`. The next `read`
//!   or `verify` returns [`Error::Checksum`]. Header-scrub throughput is
//!   not payload bandwidth.
//! - Empty comment and "no comment" are the same bytes. Created time at the
//!   Unix epoch is `Some(0)`, which is a different encoding from `None`.
//! - Tags are opaque UTF-8. Nothing here writes a colour index or an xattr.
//! - Times are signed Unix nanoseconds. No timezone, no leap seconds.
//! - `scrub` is one header write. A crash can tear it; the next open then
//!   fails the checksum. It is not a journal.
//! - [`write_path`] renames into place. Two writers of the same path share
//!   one temporary name.
//! - The record starts at byte 0 of the stream. There is no embedded offset.
//! - No older magic is recognised. This is the first one.
//! - [`decrypt_ooxml`] is a separate path. It does not read or write this record,
//!   and [`scrub`] does not decrypt.
//!
//! ```
//! use export_meta::{read, scrub, write, Record};
//! use std::io::Cursor;
//!
//! let rec = Record {
//!     modified_ns: 1_700_000_000_000_000_000,
//!     created_ns: None,
//!     comment: "hi".into(),
//!     tags: vec!["red".into()],
//!     extras: vec![],
//! };
//! let mut buf = Cursor::new(Vec::new());
//! write(&mut buf, &rec, &mut &b"payload"[..]).unwrap();
//! let mut file = Cursor::new(buf.into_inner());
//! scrub(&mut file).unwrap();
//! file.set_position(0);
//! let opened = read(&mut file, 1024).unwrap();
//! assert_eq!(opened.record.comment, "");
//! assert_eq!(opened.record.tags, Vec::<String>::new());
//! assert_eq!(opened.body, b"payload");
//! ```

#![forbid(unsafe_code)]

mod aes;
mod crc;
mod ooxml;
mod sha1;

pub use ooxml::{decrypt_ooxml, encrypt_ooxml, DecryptError};

use crc::{crc32, Crc32};
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

/// Eight bytes, fixed. A later incompatible encoding bumps this, it does not
/// grow the head.
pub const MAGIC: [u8; 8] = *b"EXMETA01";

/// Fixed head width. Fetch this many bytes before trusting any length in the file.
pub const HEAD: usize = 80;

/// Largest header this reader will allocate. The payload is not under this cap.
pub const MAX_HEADER: u32 = 1024 * 1024;

const OFF_FLAGS: usize = 8;
const OFF_RESERVED: usize = 10;
const OFF_HEADER_LEN: usize = 12;
const OFF_BODY_LEN: usize = 16;
const OFF_MODIFIED: usize = 24;
const OFF_CREATED: usize = 32;
const OFF_BODY_CRC: usize = 40;
const OFF_VAR_CRC: usize = 44;
const OFF_FIXED_CRC: usize = 48;
const OFF_COMMENT_OFF: usize = 52;
const OFF_COMMENT_LEN: usize = 56;
const OFF_TAGS_OFF: usize = 60;
const OFF_TAGS_LEN: usize = 64;
const OFF_EXTRAS_OFF: usize = 68;
const OFF_EXTRAS_LEN: usize = 72;
const OFF_RESERVED2: usize = 76;

const FLAG_CREATED: u16 = 1;
const CHUNK: usize = 64 * 1024;

/// Modified time, optional created time, comment, tags, and extra fields.
///
/// `modified_ns` and `created_ns` are Unix time in nanoseconds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub modified_ns: i64,
    pub created_ns: Option<i64>,
    pub comment: String,
    pub tags: Vec<String>,
    pub extras: Vec<(String, Vec<u8>)>,
}

/// A record plus the payload bytes that followed it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Opened {
    pub record: Record,
    pub body: Vec<u8>,
}

/// Why a file was refused. Library paths return this instead of panicking.
#[derive(Debug)]
pub enum Error {
    /// The stream ended before the header or the declared payload did.
    Truncated,
    /// The first eight bytes are not [`MAGIC`].
    BadMagic,
    /// A checksum did not match. Head, variable region, or payload.
    Checksum,
    /// Lengths, flags, UTF-8, or padding disagree with the checksummed bytes.
    Corrupt,
    /// The header, or the payload relative to `max_body`, is over the cap.
    TooLarge,
    /// Bytes follow the declared payload.
    Trailing,
    /// A filesystem or stream error that is not a short read.
    Io(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Truncated => f.write_str("truncated metadata record"),
            Error::BadMagic => f.write_str("bad magic"),
            Error::Checksum => f.write_str("checksum mismatch: metadata record is corrupt"),
            Error::Corrupt => f.write_str("metadata record structure is corrupt"),
            Error::TooLarge => f.write_str("metadata record exceeds the size cap"),
            Error::Trailing => f.write_str("trailing bytes after metadata record"),
            Error::Io(e) => write!(f, "io error: {e}"),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Error::Io(e) => Some(e),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

struct Head {
    header_len: u32,
    body_len: u64,
    body_crc: u32,
    record: Record,
}

/// Write `record` and then `body` at `out`'s current position.
///
/// The payload is copied in 64 KiB chunks. Peak extra memory is the header
/// (at most 1 MiB) plus that chunk. A failure leaves whatever bytes were
/// already written; use [`write_path`] when the result has to appear whole.
pub fn write<W: Write + Seek, R: Read>(out: &mut W, record: &Record, body: &mut R) -> Result<()> {
    let natural = encode_header(record, 0, 0, None)?;
    let header_len = natural.len();
    let start = out.stream_position().map_err(Error::Io)?;
    out.write_all(&vec![0u8; header_len]).map_err(Error::Io)?;

    let mut crc = Crc32::new();
    let body_len = copy_body(body, out, &mut crc)?;
    let header = encode_header(record, body_len, crc.finish(), None)?;
    if header.len() != header_len {
        return Err(Error::Corrupt);
    }
    out.seek(SeekFrom::Start(start)).map_err(Error::Io)?;
    out.write_all(&header).map_err(Error::Io)?;
    let end = start
        .checked_add(header_len as u64)
        .and_then(|n| n.checked_add(body_len))
        .ok_or(Error::TooLarge)?;
    out.seek(SeekFrom::Start(end)).map_err(Error::Io)?;
    Ok(())
}

/// Write to `path` via a temporary file in the same directory, then rename.
pub fn write_path<R: Read>(path: &Path, record: &Record, body: &mut R) -> Result<()> {
    let tmp = tmp_path(path)?;
    let write_result = (|| {
        let mut file = File::create(&tmp).map_err(Error::Io)?;
        write(&mut file, record, body)?;
        file.sync_all().map_err(Error::Io)?;
        Ok(())
    })();
    if let Err(e) = write_result {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    fs::rename(&tmp, path).map_err(Error::Io)?;
    Ok(())
}

/// Read the record and the payload.
///
/// `max_body` is the most payload bytes that will be allocated. A declared
/// payload above that is [`Error::TooLarge`] and is not read.
pub fn read<R: Read>(input: &mut R, max_body: u64) -> Result<Opened> {
    let head = read_head_and_record(input)?;
    if head.body_len > max_body {
        return Err(Error::TooLarge);
    }
    let n = usize::try_from(head.body_len).map_err(|_| Error::TooLarge)?;
    let mut body = vec![0u8; n];
    if n > 0 {
        read_exact(input, &mut body)?;
    }
    if crc32(&body) != head.body_crc {
        return Err(Error::Checksum);
    }
    expect_eof(input)?;
    Ok(Opened {
        record: head.record,
        body,
    })
}

/// Check the header and the payload without keeping the payload.
///
/// Memory is the header (strings included) plus a 64 KiB chunk.
pub fn verify<R: Read>(input: &mut R, max_body: u64) -> Result<Record> {
    let head = read_head_and_record(input)?;
    if head.body_len > max_body {
        return Err(Error::TooLarge);
    }
    let mut left = head.body_len;
    let mut crc = Crc32::new();
    let mut buf = [0u8; CHUNK];
    while left > 0 {
        let want = std::cmp::min(left, CHUNK as u64) as usize;
        read_exact(input, &mut buf[..want])?;
        crc.update(&buf[..want]);
        left -= want as u64;
    }
    if crc.finish() != head.body_crc {
        return Err(Error::Checksum);
    }
    expect_eof(input)?;
    Ok(head.record)
}

pub fn read_path(path: &Path, max_body: u64) -> Result<Opened> {
    let mut file = File::open(path).map_err(Error::Io)?;
    read(&mut file, max_body)
}

pub fn verify_path(path: &Path, max_body: u64) -> Result<Record> {
    let mut file = File::open(path).map_err(Error::Io)?;
    verify(&mut file, max_body)
}

/// Drop comment, tags, and extra fields by rewriting the header in place.
///
/// The payload offset stays put, so the payload is not read. The header
/// checksum is verified first: a corrupt header is returned as [`Error`] and
/// is not rewritten into a clean record. The stream is the whole file and
/// the record starts at byte 0.
///
/// On success the returned record is what a later [`read`] will see: the
/// same times, empty comment, no tags, no extras.
pub fn scrub<S: Read + Write + Seek>(file: &mut S) -> Result<Record> {
    let file_len = file.seek(SeekFrom::End(0)).map_err(Error::Io)?;
    file.seek(SeekFrom::Start(0)).map_err(Error::Io)?;
    let head = read_head_and_record(file)?;
    let total = (head.header_len as u64)
        .checked_add(head.body_len)
        .ok_or(Error::Corrupt)?;
    if file_len < total {
        return Err(Error::Truncated);
    }
    if file_len > total {
        return Err(Error::Trailing);
    }
    let cleared = Record {
        modified_ns: head.record.modified_ns,
        created_ns: head.record.created_ns,
        comment: String::new(),
        tags: Vec::new(),
        extras: Vec::new(),
    };
    let new_header = encode_header(
        &cleared,
        head.body_len,
        head.body_crc,
        Some(head.header_len),
    )?;
    file.seek(SeekFrom::Start(0)).map_err(Error::Io)?;
    file.write_all(&new_header).map_err(Error::Io)?;
    Ok(cleared)
}

pub fn scrub_path(path: &Path) -> Result<Record> {
    let mut file = File::options()
        .read(true)
        .write(true)
        .open(path)
        .map_err(Error::Io)?;
    let record = scrub(&mut file)?;
    file.sync_all().map_err(Error::Io)?;
    Ok(record)
}

fn read_head_and_record<R: Read>(input: &mut R) -> Result<Head> {
    let mut fixed = [0u8; HEAD];
    read_exact(input, &mut fixed)?;
    if fixed[0..8] != MAGIC {
        return Err(Error::BadMagic);
    }
    if fixed_crc(&fixed) != u32_at(&fixed, OFF_FIXED_CRC)? {
        return Err(Error::Checksum);
    }
    let reserved = u16::from_le_bytes([fixed[OFF_RESERVED], fixed[OFF_RESERVED + 1]]);
    if reserved != 0 || u32_at(&fixed, OFF_RESERVED2)? != 0 {
        return Err(Error::Corrupt);
    }
    let header_len = u32_at(&fixed, OFF_HEADER_LEN)?;
    if header_len < HEAD as u32 {
        return Err(Error::Corrupt);
    }
    if header_len > MAX_HEADER {
        return Err(Error::TooLarge);
    }
    let var_len = (header_len as usize) - HEAD;
    let mut variable = vec![0u8; var_len];
    if var_len > 0 {
        read_exact(input, &mut variable)?;
    }
    if crc32(&variable) != u32_at(&fixed, OFF_VAR_CRC)? {
        return Err(Error::Checksum);
    }
    let flags = u16::from_le_bytes([fixed[OFF_FLAGS], fixed[OFF_FLAGS + 1]]);
    if flags & !FLAG_CREATED != 0 {
        return Err(Error::Corrupt);
    }
    let modified_ns = i64::from_le_bytes(bytes_at(&fixed, OFF_MODIFIED)?);
    let created_raw = i64::from_le_bytes(bytes_at(&fixed, OFF_CREATED)?);
    let created_ns = if flags & FLAG_CREATED != 0 {
        Some(created_raw)
    } else if created_raw == 0 {
        None
    } else {
        return Err(Error::Corrupt);
    };

    let comment_span = span_at(&fixed, OFF_COMMENT_OFF, OFF_COMMENT_LEN)?;
    let tags_span = span_at(&fixed, OFF_TAGS_OFF, OFF_TAGS_LEN)?;
    let extras_span = span_at(&fixed, OFF_EXTRAS_OFF, OFF_EXTRAS_LEN)?;
    check_packed(header_len, comment_span, tags_span, extras_span, &variable)?;

    let comment = take_span(&variable, comment_span)?;
    let comment = String::from_utf8(comment).map_err(|_| Error::Corrupt)?;
    let tags = decode_tags(&take_span(&variable, tags_span)?)?;
    let extras = decode_extras(&take_span(&variable, extras_span)?)?;

    Ok(Head {
        header_len,
        body_len: u64::from_le_bytes(bytes_at(&fixed, OFF_BODY_LEN)?),
        body_crc: u32_at(&fixed, OFF_BODY_CRC)?,
        record: Record {
            modified_ns,
            created_ns,
            comment,
            tags,
            extras,
        },
    })
}

fn encode_header(
    record: &Record,
    body_len: u64,
    body_crc: u32,
    pad_to: Option<u32>,
) -> Result<Vec<u8>> {
    let tags = encode_tags(&record.tags)?;
    let extras = encode_extras(&record.extras)?;
    let comment = record.comment.as_bytes();
    let variable_len = comment
        .len()
        .checked_add(tags.len())
        .and_then(|n| n.checked_add(extras.len()))
        .ok_or(Error::TooLarge)?;
    let natural = HEAD.checked_add(variable_len).ok_or(Error::TooLarge)?;
    let natural_u32 = u32::try_from(natural).map_err(|_| Error::TooLarge)?;
    let header_len = match pad_to {
        Some(n) if n >= natural_u32 => n,
        Some(_) => return Err(Error::TooLarge),
        None => natural_u32,
    };
    if header_len > MAX_HEADER {
        return Err(Error::TooLarge);
    }

    let mut h = vec![0u8; header_len as usize];
    h[0..8].copy_from_slice(&MAGIC);
    let flags: u16 = if record.created_ns.is_some() {
        FLAG_CREATED
    } else {
        0
    };
    h[OFF_FLAGS..OFF_FLAGS + 2].copy_from_slice(&flags.to_le_bytes());
    h[OFF_HEADER_LEN..OFF_HEADER_LEN + 4].copy_from_slice(&header_len.to_le_bytes());
    h[OFF_BODY_LEN..OFF_BODY_LEN + 8].copy_from_slice(&body_len.to_le_bytes());
    h[OFF_MODIFIED..OFF_MODIFIED + 8].copy_from_slice(&record.modified_ns.to_le_bytes());
    let created = record.created_ns.unwrap_or(0);
    h[OFF_CREATED..OFF_CREATED + 8].copy_from_slice(&created.to_le_bytes());
    h[OFF_BODY_CRC..OFF_BODY_CRC + 4].copy_from_slice(&body_crc.to_le_bytes());

    let mut cursor = HEAD as u32;
    cursor = write_span(&mut h, OFF_COMMENT_OFF, OFF_COMMENT_LEN, cursor, comment)?;
    cursor = write_span(&mut h, OFF_TAGS_OFF, OFF_TAGS_LEN, cursor, &tags)?;
    let _ = write_span(&mut h, OFF_EXTRAS_OFF, OFF_EXTRAS_LEN, cursor, &extras)?;

    let var_crc = crc32(&h[HEAD..header_len as usize]);
    h[OFF_VAR_CRC..OFF_VAR_CRC + 4].copy_from_slice(&var_crc.to_le_bytes());
    let fixed = fixed_crc(&h);
    h[OFF_FIXED_CRC..OFF_FIXED_CRC + 4].copy_from_slice(&fixed.to_le_bytes());
    Ok(h)
}

fn write_span(
    header: &mut [u8],
    off_at: usize,
    len_at: usize,
    cursor: u32,
    bytes: &[u8],
) -> Result<u32> {
    if bytes.is_empty() {
        return Ok(cursor);
    }
    let len = u32::try_from(bytes.len()).map_err(|_| Error::TooLarge)?;
    let end = cursor.checked_add(len).ok_or(Error::TooLarge)?;
    header[off_at..off_at + 4].copy_from_slice(&cursor.to_le_bytes());
    header[len_at..len_at + 4].copy_from_slice(&len.to_le_bytes());
    let start = cursor as usize;
    header[start..start + bytes.len()].copy_from_slice(bytes);
    Ok(end)
}

fn encode_tags(tags: &[String]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for tag in tags {
        let b = tag.as_bytes();
        let n = u16::try_from(b.len()).map_err(|_| Error::TooLarge)?;
        out.extend_from_slice(&n.to_le_bytes());
        out.extend_from_slice(b);
    }
    Ok(out)
}

fn encode_extras(extras: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    for (key, value) in extras {
        let kb = key.as_bytes();
        let kn = u16::try_from(kb.len()).map_err(|_| Error::TooLarge)?;
        let vn = u32::try_from(value.len()).map_err(|_| Error::TooLarge)?;
        out.extend_from_slice(&kn.to_le_bytes());
        out.extend_from_slice(kb);
        out.extend_from_slice(&vn.to_le_bytes());
        out.extend_from_slice(value);
    }
    Ok(out)
}

fn decode_tags(span: &[u8]) -> Result<Vec<String>> {
    let mut i = 0;
    let mut tags = Vec::new();
    while i < span.len() {
        if span.len() - i < 2 {
            return Err(Error::Corrupt);
        }
        let n = u16::from_le_bytes([span[i], span[i + 1]]) as usize;
        i += 2;
        let end = i.checked_add(n).ok_or(Error::Corrupt)?;
        if end > span.len() {
            return Err(Error::Corrupt);
        }
        let s = std::str::from_utf8(&span[i..end]).map_err(|_| Error::Corrupt)?;
        tags.push(s.to_string());
        i = end;
    }
    Ok(tags)
}

fn decode_extras(span: &[u8]) -> Result<Vec<(String, Vec<u8>)>> {
    let mut i = 0;
    let mut extras = Vec::new();
    while i < span.len() {
        if span.len() - i < 2 {
            return Err(Error::Corrupt);
        }
        let kn = u16::from_le_bytes([span[i], span[i + 1]]) as usize;
        i += 2;
        let key_end = i.checked_add(kn).ok_or(Error::Corrupt)?;
        if key_end > span.len() || span.len() - key_end < 4 {
            return Err(Error::Corrupt);
        }
        let key = std::str::from_utf8(&span[i..key_end]).map_err(|_| Error::Corrupt)?;
        i = key_end;
        let vn = u32::from_le_bytes([span[i], span[i + 1], span[i + 2], span[i + 3]]) as usize;
        i += 4;
        let val_end = i.checked_add(vn).ok_or(Error::Corrupt)?;
        if val_end > span.len() {
            return Err(Error::Corrupt);
        }
        extras.push((key.to_string(), span[i..val_end].to_vec()));
        i = val_end;
    }
    Ok(extras)
}

/// Spans are packed from `HEAD` with holes forbidden. Bytes after the last
/// live span, up to `header_len`, are padding and must be zero.
fn check_packed(
    header_len: u32,
    comment: (u32, u32),
    tags: (u32, u32),
    extras: (u32, u32),
    variable: &[u8],
) -> Result<()> {
    let mut cursor = HEAD as u32;
    for (off, len) in [comment, tags, extras] {
        if len == 0 {
            if off != 0 {
                return Err(Error::Corrupt);
            }
            continue;
        }
        if off != cursor {
            return Err(Error::Corrupt);
        }
        let end = off.checked_add(len).ok_or(Error::Corrupt)?;
        if end > header_len {
            return Err(Error::Corrupt);
        }
        cursor = end;
    }
    let pad_from = (cursor as usize).checked_sub(HEAD).ok_or(Error::Corrupt)?;
    if pad_from > variable.len() {
        return Err(Error::Corrupt);
    }
    if variable[pad_from..].iter().any(|&b| b != 0) {
        return Err(Error::Corrupt);
    }
    Ok(())
}

fn take_span(variable: &[u8], span: (u32, u32)) -> Result<Vec<u8>> {
    let (off, len) = span;
    if len == 0 {
        return Ok(Vec::new());
    }
    let start = (off as usize).checked_sub(HEAD).ok_or(Error::Corrupt)?;
    let end = start.checked_add(len as usize).ok_or(Error::Corrupt)?;
    if end > variable.len() {
        return Err(Error::Corrupt);
    }
    Ok(variable[start..end].to_vec())
}

fn span_at(fixed: &[u8], off_at: usize, len_at: usize) -> Result<(u32, u32)> {
    Ok((u32_at(fixed, off_at)?, u32_at(fixed, len_at)?))
}

fn fixed_crc(head: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(&head[..OFF_FIXED_CRC]);
    c.update(&[0, 0, 0, 0]);
    c.update(&head[OFF_FIXED_CRC + 4..HEAD]);
    c.finish()
}

fn u32_at(buf: &[u8], off: usize) -> Result<u32> {
    let s = buf.get(off..off + 4).ok_or(Error::Corrupt)?;
    Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn bytes_at<const N: usize>(buf: &[u8], off: usize) -> Result<[u8; N]> {
    let s = buf.get(off..off + N).ok_or(Error::Corrupt)?;
    let mut a = [0u8; N];
    a.copy_from_slice(s);
    Ok(a)
}

fn read_exact<R: Read>(r: &mut R, buf: &mut [u8]) -> Result<()> {
    let mut off = 0;
    while off < buf.len() {
        match r.read(&mut buf[off..]) {
            Ok(0) => return Err(Error::Truncated),
            Ok(n) => off += n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::Io(e)),
        }
    }
    Ok(())
}

fn expect_eof<R: Read>(r: &mut R) -> Result<()> {
    let mut b = [0u8; 1];
    loop {
        match r.read(&mut b) {
            Ok(0) => return Ok(()),
            Ok(_) => return Err(Error::Trailing),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::Io(e)),
        }
    }
}

fn copy_body<W: Write>(body: &mut impl Read, out: &mut W, crc: &mut Crc32) -> Result<u64> {
    let mut buf = [0u8; CHUNK];
    let mut total = 0u64;
    loop {
        let n = match body.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(Error::Io(e)),
        };
        crc.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(Error::Io)?;
        total = total.checked_add(n as u64).ok_or(Error::TooLarge)?;
    }
    Ok(total)
}

fn tmp_path(path: &Path) -> Result<PathBuf> {
    let name = path.file_name().ok_or_else(|| {
        Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "path has no file name",
        ))
    })?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(".exmeta-tmp");
    Ok(path.with_file_name(tmp_name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{self, Cursor};

    fn sample() -> Record {
        Record {
            modified_ns: 1_600_000_000_000_000_000,
            created_ns: Some(1_500_000_000_000_000_000),
            comment: "hello".into(),
            tags: vec!["Red".into(), "work".into(), String::new()],
            extras: vec![
                ("where_from".into(), b"https://example.com/a".to_vec()),
                ("bin".into(), vec![0x00, 0xff, 0xfe]),
            ],
        }
    }

    fn store(record: &Record, body: &[u8]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        write(&mut buf, record, &mut &body[..]).unwrap();
        buf.into_inner()
    }

    #[test]
    fn head_width_is_the_documented_table() {
        assert_eq!(MAGIC.len(), 8);
        assert_eq!(HEAD, 80);
        assert_eq!(OFF_RESERVED2 + 4, HEAD);
        assert_eq!(OFF_EXTRAS_LEN + 4, OFF_RESERVED2);
    }

    #[test]
    fn roundtrip_times_comment_tags_extras_and_body() {
        let body = b"payload-bytes";
        let bytes = store(&sample(), body);
        assert_eq!(
            u32_at(&bytes, OFF_HEADER_LEN).unwrap() as usize,
            header_len_of(&bytes)
        );
        // Payload starts at header_len and is not wrapped.
        let hlen = u32_at(&bytes, OFF_HEADER_LEN).unwrap() as usize;
        assert_eq!(&bytes[hlen..], body);
        let opened = read(&mut &bytes[..], 1024).unwrap();
        assert_eq!(opened.record, sample());
        assert_eq!(opened.body, body);
        assert_eq!(verify(&mut &bytes[..], 1024).unwrap(), sample());
    }

    #[test]
    fn created_absent_is_not_epoch() {
        let mut rec = sample();
        rec.created_ns = None;
        let bytes = store(&rec, b"");
        let opened = read(&mut &bytes[..], 0).unwrap();
        assert_eq!(opened.record.created_ns, None);
        assert_eq!(opened.body, b"");

        rec.created_ns = Some(0);
        let bytes = store(&rec, b"x");
        let opened = read(&mut &bytes[..], 8).unwrap();
        assert_eq!(opened.record.created_ns, Some(0));
    }

    #[test]
    fn negative_modified_time_roundtrips() {
        let rec = Record {
            modified_ns: -1,
            created_ns: None,
            comment: String::new(),
            tags: vec![],
            extras: vec![],
        };
        let bytes = store(&rec, b"z");
        assert_eq!(read(&mut &bytes[..], 8).unwrap().record.modified_ns, -1);
    }

    #[test]
    fn layout_packs_spans_from_head_with_no_holes() {
        let bytes = store(&sample(), b"BODY");
        let hlen = u32_at(&bytes, OFF_HEADER_LEN).unwrap();
        let comment = span_at(&bytes, OFF_COMMENT_OFF, OFF_COMMENT_LEN).unwrap();
        let tags = span_at(&bytes, OFF_TAGS_OFF, OFF_TAGS_LEN).unwrap();
        let extras = span_at(&bytes, OFF_EXTRAS_OFF, OFF_EXTRAS_LEN).unwrap();
        assert_eq!(comment.0, HEAD as u32);
        assert_eq!(comment.1, 5); // "hello"
        assert_eq!(tags.0, comment.0 + comment.1);
        assert_eq!(extras.0, tags.0 + tags.1);
        assert!(extras.0 + extras.1 <= hlen);
        assert!(bytes[(extras.0 + extras.1) as usize..hlen as usize]
            .iter()
            .all(|&b| b == 0));
        assert_eq!(&bytes[hlen as usize..], b"BODY");
    }

    #[test]
    fn scrub_drops_comment_tags_extras_and_keeps_times_and_body() {
        let rec = sample();
        let body = vec![7u8; 256 * 1024];
        let bytes = store(&rec, &body);
        let hlen = u32_at(&bytes, OFF_HEADER_LEN).unwrap() as u64;
        let mut spy = Spy {
            inner: Cursor::new(bytes),
            pos: 0,
            bytes_read: 0,
            read_end: 0,
        };
        let cleared = scrub(&mut spy).unwrap();
        assert_eq!(cleared.modified_ns, rec.modified_ns);
        assert_eq!(cleared.created_ns, rec.created_ns);
        assert_eq!(cleared.comment, "");
        assert!(cleared.tags.is_empty());
        assert!(cleared.extras.is_empty());
        assert_eq!(spy.bytes_read, hlen, "scrub read past the header");
        assert!(spy.read_end <= hlen);

        let bytes = spy.inner.into_inner();
        assert_eq!(u32_at(&bytes, OFF_HEADER_LEN).unwrap() as u64, hlen);
        assert_eq!(&bytes[hlen as usize..], body.as_slice());
        let opened = read(&mut &bytes[..], body.len() as u64).unwrap();
        assert_eq!(opened.record, cleared);
        assert_eq!(opened.body, body);

        let mut again = Cursor::new(bytes.clone());
        scrub(&mut again).unwrap();
        assert_eq!(again.into_inner(), bytes, "second scrub rewrote bytes");
    }

    #[test]
    fn scrub_refuses_a_short_file_without_writing() {
        let mut bytes = store(&sample(), b"abcdef");
        let original = bytes.clone();
        bytes.pop();
        let mut file = Cursor::new(bytes);
        let err = scrub(&mut file).unwrap_err();
        assert!(matches!(err, Error::Truncated));
        assert_eq!(file.into_inner(), original[..original.len() - 1].to_vec());
    }

    #[test]
    fn scrub_does_not_repair_a_corrupt_header() {
        let mut bytes = store(&sample(), b"abcdef");
        let original = bytes.clone();
        bytes[HEAD] ^= 0x01;
        let mut file = Cursor::new(bytes);
        let err = scrub(&mut file).unwrap_err();
        assert!(matches!(err, Error::Checksum));
        assert_eq!(file.into_inner(), original_flipped(&original));
    }

    #[test]
    fn truncated_prefixes_error() {
        let bytes = store(&sample(), b"xyz");
        for n in 0..bytes.len() {
            let err = read(&mut &bytes[..n], 1024).unwrap_err();
            assert!(
                matches!(err, Error::Truncated | Error::BadMagic | Error::Checksum),
                "prefix {n} returned {err:?}"
            );
        }
    }

    #[test]
    fn every_single_bit_flip_errors_on_read() {
        let bytes = store(&sample(), b"0123456789abcdef");
        let opened = read(&mut &bytes[..], 1024).unwrap();
        assert_eq!(opened.record.comment, "hello");
        for i in 0..bytes.len() {
            for bit in 0..8 {
                let mut bad = bytes.clone();
                bad[i] ^= 1 << bit;
                let err = read(&mut &bad[..], 1024).unwrap_err();
                assert!(
                    !matches!(err, Error::Io(_)),
                    "byte {i} bit {bit} was an io error, not a format error: {err}"
                );
                assert!(
                    verify(&mut &bad[..], 1024).is_err(),
                    "verify accepted byte {i} bit {bit}"
                );
            }
        }
    }

    #[test]
    fn flipped_header_len_does_not_read_past_the_head() {
        let mut bytes = store(&sample(), vec![1u8; 50_000].as_slice());
        bytes[OFF_HEADER_LEN] ^= 0x80;
        let mut src = Budget {
            data: bytes,
            pos: 0,
            reads: 0,
        };
        let err = read(&mut src, u64::MAX).unwrap_err();
        assert!(matches!(err, Error::Checksum));
        assert_eq!(src.reads, HEAD);
    }

    #[test]
    fn body_bit_flip_fails_read_and_is_the_scrub_limit() {
        let mut bytes = store(&sample(), b"0123456789abcdef");
        let hlen = u32_at(&bytes, OFF_HEADER_LEN).unwrap() as usize;
        bytes[hlen] ^= 0x01;
        let err = read(&mut &bytes[..], 1024).unwrap_err();
        assert!(matches!(err, Error::Checksum));

        // Scrub does not read the payload, so this stays a checksum error
        // for the next open. The header write must not clear that failure.
        let mut file = Cursor::new(bytes);
        assert!(scrub(&mut file).is_ok());
        file.set_position(0);
        let err = read(&mut file, 1024).unwrap_err();
        assert!(matches!(err, Error::Checksum));
    }

    #[test]
    fn invalid_utf8_with_a_matching_checksum_is_corrupt() {
        let mut bytes = store(&sample(), b"q");
        let hlen = u32_at(&bytes, OFF_HEADER_LEN).unwrap() as usize;
        let comment_off = u32_at(&bytes, OFF_COMMENT_OFF).unwrap() as usize;
        bytes[comment_off] = 0xff;
        // Rebuild both checksums so the failure is structural, not CRC.
        restamp(&mut bytes, hlen);
        let err = read(&mut &bytes[..], 16).unwrap_err();
        assert!(matches!(err, Error::Corrupt), "{err:?}");
    }

    #[test]
    fn nonzero_padding_with_a_matching_checksum_is_corrupt() {
        let rec = Record {
            modified_ns: 1,
            created_ns: None,
            comment: "a".into(),
            tags: vec![],
            extras: vec![],
        };
        // `write` packs spans with no spare bytes. Pad on purpose, then dirty
        // the spare byte and restamp so the failure is structural.
        let mut crafted = encode_header(&rec, 1, crc32(b"q"), Some(HEAD as u32 + 8)).unwrap();
        let hlen = crafted.len();
        crafted[hlen - 1] = 0x01;
        restamp(&mut crafted, hlen);
        crafted.extend_from_slice(b"q");
        // body_crc was of b"q" and restamp keeps body bytes outside the header.
        // restamp recomputes var and fixed crc only. body_crc still matches b"q".
        let err = read(&mut &crafted[..], 16).unwrap_err();
        assert!(matches!(err, Error::Corrupt), "{err:?}");
    }

    #[test]
    fn unknown_flag_bit_with_a_matching_checksum_is_corrupt() {
        let mut bytes = store(&sample(), b"q");
        let hlen = u32_at(&bytes, OFF_HEADER_LEN).unwrap() as usize;
        bytes[OFF_FLAGS + 1] |= 0x80;
        restamp(&mut bytes, hlen);
        let err = read(&mut &bytes[..], 16).unwrap_err();
        assert!(matches!(err, Error::Corrupt), "{err:?}");
    }

    #[test]
    fn trailing_byte_errors() {
        let mut bytes = store(&sample(), b"q");
        bytes.push(0);
        let err = read(&mut &bytes[..], 16).unwrap_err();
        assert!(matches!(err, Error::Trailing));
    }

    #[test]
    fn max_body_refuses_before_the_payload_is_accepted() {
        let bytes = store(&sample(), &[9u8; 100]);
        let err = read(&mut &bytes[..], 10).unwrap_err();
        assert!(matches!(err, Error::TooLarge));
        let err = verify(&mut &bytes[..], 10).unwrap_err();
        assert!(matches!(err, Error::TooLarge));
    }

    #[test]
    fn odd_chunks_roundtrip() {
        let body: Vec<u8> = (0..100_000u32).map(|n| n as u8).collect();
        let mut src = Chunks {
            data: body.clone(),
            pos: 0,
            size: 3,
        };
        let mut buf = Cursor::new(Vec::new());
        write(&mut buf, &sample(), &mut src).unwrap();
        let bytes = buf.into_inner();
        let opened = read(&mut &bytes[..], body.len() as u64).unwrap();
        assert_eq!(opened.body, body);
        assert_eq!(opened.record, sample());
    }

    #[test]
    fn write_path_renames_and_removes_the_temp_on_failure() {
        let dir = std::env::temp_dir().join(format!("export-meta-{}-{}", std::process::id(), "ok"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("note.bin");
        write_path(&path, &sample(), &mut &b"body"[..]).unwrap();
        let opened = read_path(&path, 16).unwrap();
        assert_eq!(opened.body, b"body");
        assert!(!tmp_path(&path).unwrap().exists());
        scrub_path(&path).unwrap();
        let opened = read_path(&path, 16).unwrap();
        assert_eq!(opened.record.comment, "");
        assert_eq!(opened.body, b"body");

        let bad = dir.join("bad.bin");
        let err = write_path(&bad, &sample(), &mut Boom { left: 4 }).unwrap_err();
        assert!(matches!(err, Error::Io(_)));
        assert!(!bad.exists());
        assert!(!tmp_path(&bad).unwrap().exists());
        let _ = fs::remove_dir_all(&dir);
    }

    fn header_len_of(bytes: &[u8]) -> usize {
        u32_at(bytes, OFF_HEADER_LEN).unwrap() as usize
    }

    fn original_flipped(original: &[u8]) -> Vec<u8> {
        let mut bad = original.to_vec();
        bad[HEAD] ^= 0x01;
        bad
    }

    fn restamp(bytes: &mut [u8], hlen: usize) {
        let var = crc32(&bytes[HEAD..hlen]);
        bytes[OFF_VAR_CRC..OFF_VAR_CRC + 4].copy_from_slice(&var.to_le_bytes());
        let fixed = fixed_crc(bytes);
        bytes[OFF_FIXED_CRC..OFF_FIXED_CRC + 4].copy_from_slice(&fixed.to_le_bytes());
    }

    struct Spy<T> {
        inner: T,
        pos: u64,
        bytes_read: u64,
        read_end: u64,
    }

    impl<T: Read> Read for Spy<T> {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let n = self.inner.read(buf)?;
            self.bytes_read += n as u64;
            self.read_end = self.read_end.max(self.pos + n as u64);
            self.pos += n as u64;
            Ok(n)
        }
    }

    impl<T: Write> Write for Spy<T> {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            let n = self.inner.write(buf)?;
            self.pos += n as u64;
            Ok(n)
        }
        fn flush(&mut self) -> io::Result<()> {
            self.inner.flush()
        }
    }

    impl<T: Seek> Seek for Spy<T> {
        fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
            self.pos = self.inner.seek(pos)?;
            Ok(self.pos)
        }
    }

    struct Budget {
        data: Vec<u8>,
        pos: usize,
        reads: usize,
    }

    impl Read for Budget {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let n = buf.len().min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            self.reads += n;
            Ok(n)
        }
    }

    struct Chunks {
        data: Vec<u8>,
        pos: usize,
        size: usize,
    }

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.pos >= self.data.len() {
                return Ok(0);
            }
            let n = buf.len().min(self.size).min(self.data.len() - self.pos);
            buf[..n].copy_from_slice(&self.data[self.pos..self.pos + n]);
            self.pos += n;
            Ok(n)
        }
    }

    struct Boom {
        left: usize,
    }

    impl Read for Boom {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if self.left == 0 {
                return Err(io::Error::new(io::ErrorKind::Other, "boom"));
            }
            let n = buf.len().min(self.left).min(8);
            buf[..n].fill(1);
            self.left -= n;
            Ok(n)
        }
    }
}
