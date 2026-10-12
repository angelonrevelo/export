//! CSV and TSV, one record at a time.
//!
//! Quoting is RFC-4180-ish and the same for both delimiters: a field that
//! contains the delimiter, a quote, or a newline is wrapped in quotes, and a
//! quote inside a field is doubled. A tab-separated file that never quotes
//! (what `mysql -B` prints) still splits on tabs. A field that itself contains
//! a tab has to be quoted, or the rectangle would shift.
//!
//! Lines that are empty or only spaces are skipped. A line of only tabs is a
//! real TSV row of empty cells and is kept. A ragged row — a different column
//! count than the header — is an error at the conversion layer, not here.
//! This reader reports the cells it saw.

use std::io::BufRead;

use crate::error::{Error, Result};
use crate::MAX_RECORD_BYTES;

pub struct DelimReader<R> {
    src: R,
    delim: u8,
    buf: Vec<u8>,
    fields: Vec<(u32, u32)>,
    lossy: u64,
    records: u64,
    at_start: bool,
    max_record: usize,
}

impl<R: BufRead> DelimReader<R> {
    pub fn new(src: R, delim: u8) -> Self {
        Self {
            src,
            delim,
            buf: Vec::new(),
            fields: Vec::new(),
            lossy: 0,
            records: 0,
            at_start: true,
            max_record: MAX_RECORD_BYTES,
        }
    }

    /// Records so far whose bytes were not valid UTF-8 and were repaired.
    pub fn lossy(&self) -> u64 {
        self.lossy
    }

    /// Capacity of the record buffer. This stays near the widest record, not
    /// the size of the file, which is the streaming claim.
    pub fn resident_capacity(&self) -> usize {
        self.buf.capacity()
    }

    pub fn len(&self) -> usize {
        self.fields.len()
    }

    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Field `i` of the record last returned by [`Self::next_row`].
    pub fn field(&self, i: usize) -> &str {
        let (a, b) = self.fields[i];
        std::str::from_utf8(&self.buf[a as usize..b as usize])
            .expect("record bytes were repaired to UTF-8 before the split")
    }

    /// `true` when a record is sitting in the reader. `false` at a clean end.
    pub fn next_row(&mut self) -> Result<bool> {
        loop {
            if !self.read_logical()? {
                return Ok(false);
            }
            if is_skippable_blank(&self.buf) {
                continue;
            }
            self.split_fields()?;
            self.records += 1;
            return Ok(true);
        }
    }

    fn read_logical(&mut self) -> Result<bool> {
        self.buf.clear();
        let mut any = false;
        loop {
            let n = self.src.read_until(b'\n', &mut self.buf)?;
            if n == 0 {
                if !any {
                    return Ok(false);
                }
                if !quotes_balanced(&self.buf) {
                    return Err(Error::new(format!(
                        "unterminated quote at record {}",
                        self.records + 1
                    )));
                }
                break;
            }
            any = true;
            if self.buf.last() == Some(&b'\n') {
                self.buf.pop();
            }
            let balanced = quotes_balanced(&self.buf);
            if balanced && self.buf.last() == Some(&b'\r') {
                self.buf.pop();
            }
            if self.buf.len() > self.max_record {
                return Err(Error::new(format!(
                    "record {} exceeds {MAX_RECORD_BYTES} bytes",
                    self.records + 1
                )));
            }
            if !balanced {
                // The newline was part of a quoted field. Put one back so the
                // cell still contains the line break, and keep reading.
                self.buf.push(b'\n');
                continue;
            }
            break;
        }
        if self.at_start {
            self.at_start = false;
            if self.buf.starts_with(&[0xEF, 0xBB, 0xBF]) {
                self.buf.drain(..3);
            }
        }
        if std::str::from_utf8(&self.buf).is_err() {
            let fixed = String::from_utf8_lossy(&self.buf).into_owned();
            self.buf.clear();
            self.buf.extend(fixed.as_bytes());
            self.lossy += 1;
        }
        Ok(true)
    }

    fn split_fields(&mut self) -> Result<()> {
        self.fields.clear();
        let delim = self.delim;
        let mut w = 0usize;
        let mut i = 0usize;
        let mut start = 0usize;
        let mut quoted = false;
        while i < self.buf.len() {
            let c = self.buf[i];
            if quoted {
                if c == b'"' {
                    if i + 1 < self.buf.len() && self.buf[i + 1] == b'"' {
                        self.buf[w] = b'"';
                        w += 1;
                        i += 2;
                    } else {
                        quoted = false;
                        i += 1;
                    }
                } else {
                    self.buf[w] = c;
                    w += 1;
                    i += 1;
                }
            } else if c == b'"' {
                quoted = true;
                i += 1;
            } else if c == delim {
                self.fields.push((start as u32, w as u32));
                start = w;
                i += 1;
            } else {
                self.buf[w] = c;
                w += 1;
                i += 1;
            }
        }
        if quoted {
            return Err(Error::new(format!(
                "unterminated quote at record {}",
                self.records + 1
            )));
        }
        self.fields.push((start as u32, w as u32));
        self.buf.truncate(w);
        Ok(())
    }
}

fn is_skippable_blank(buf: &[u8]) -> bool {
    buf.is_empty() || buf.iter().all(|b| *b == b' ')
}

fn quotes_balanced(bytes: &[u8]) -> bool {
    let mut i = 0;
    let mut open = false;
    while i < bytes.len() {
        if bytes[i] == b'"' {
            if open && i + 1 < bytes.len() && bytes[i + 1] == b'"' {
                i += 2;
                continue;
            }
            open = !open;
        }
        i += 1;
    }
    !open
}

pub fn write_field<W: std::io::Write>(out: &mut W, delim: u8, field: &str) -> Result<()> {
    let quote = field
        .bytes()
        .any(|b| b == delim || b == b'"' || b == b'\n' || b == b'\r');
    if !quote {
        out.write_all(field.as_bytes())?;
        return Ok(());
    }
    out.write_all(b"\"")?;
    let bytes = field.as_bytes();
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'"' {
            out.write_all(&bytes[start..i])?;
            out.write_all(b"\"\"")?;
            start = i + 1;
        }
    }
    out.write_all(&bytes[start..])?;
    out.write_all(b"\"")?;
    Ok(())
}

pub fn write_row<'a, W, I>(out: &mut W, delim: u8, fields: I) -> Result<()>
where
    W: std::io::Write,
    I: IntoIterator<Item = &'a str>,
{
    let mut first = true;
    for field in fields {
        if !first {
            out.write_all(&[delim])?;
        }
        first = false;
        write_field(out, delim, field)?;
    }
    out.write_all(b"\n")?;
    Ok(())
}
