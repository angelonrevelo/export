//! One JSON array, pulled a row at a time.
//!
//! This is not a general JSON library. A spreadsheet row is either an object
//! of scalars or an array of scalars, and every scalar is wanted as text.
//! Building a tree would allocate the whole document, which is the thing this
//! crate exists to avoid.
//!
//! The input window is compacted as it is consumed. Resident memory is the
//! window plus the current row's arena, both reused.

use std::io::BufRead;

use crate::error::{Error, Result};
use crate::MAX_RECORD_BYTES;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowShape {
    Object,
    Array,
}

pub struct JsonReader<R> {
    src: R,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
    bom_done: bool,
    started: bool,
    done: bool,
    expect_comma: bool,
    shape: Option<RowShape>,
    arena: Vec<u8>,
    keys: Vec<(u32, u32)>,
    vals: Vec<(u32, u32)>,
    lossy: u64,
    row_bad: bool,
    index: u64,
}

impl<R: BufRead> JsonReader<R> {
    pub fn new(src: R) -> Self {
        Self {
            src,
            buf: Vec::new(),
            pos: 0,
            eof: false,
            bom_done: false,
            started: false,
            done: false,
            expect_comma: false,
            shape: None,
            arena: Vec::new(),
            keys: Vec::new(),
            vals: Vec::new(),
            lossy: 0,
            row_bad: false,
            index: 0,
        }
    }

    pub fn lossy(&self) -> u64 {
        self.lossy
    }

    pub fn shape(&self) -> Option<RowShape> {
        self.shape
    }

    /// Input window plus the current row. Neither grows with the file.
    pub fn resident_capacity(&self) -> usize {
        self.buf.capacity() + self.arena.capacity()
    }

    pub fn len(&self) -> usize {
        self.vals.len()
    }

    pub fn is_empty(&self) -> bool {
        self.vals.is_empty()
    }

    pub fn key(&self, i: usize) -> &str {
        let (a, b) = self.keys[i];
        self.span(a, b)
    }

    pub fn value(&self, i: usize) -> &str {
        let (a, b) = self.vals[i];
        self.span(a, b)
    }

    fn span(&self, a: u32, b: u32) -> &str {
        std::str::from_utf8(&self.arena[a as usize..b as usize])
            .expect("string spans are repaired to UTF-8")
    }

    /// `true` when a row is loaded. `false` after the closing bracket.
    pub fn next_row(&mut self) -> Result<bool> {
        if self.done {
            return Ok(false);
        }
        self.ws()?;
        if !self.started {
            match self.peek()? {
                None => {
                    self.done = true;
                    return Ok(false);
                }
                Some(b'[') => {
                    self.bump()?;
                    self.started = true;
                }
                Some(b) => {
                    return Err(self.err(&format!("tabular JSON is one array, found {}", show(b))));
                }
            }
        }
        if self.expect_comma {
            self.ws()?;
            match self.peek()? {
                Some(b',') => {
                    self.bump()?;
                }
                Some(b']') => return self.finish(),
                Some(b) => {
                    return Err(self.err(&format!("expected ',' or ']', found {}", show(b))));
                }
                None => return Err(self.err("unterminated JSON array")),
            }
        }
        self.ws()?;
        match self.peek()? {
            None => return Err(self.err("unterminated JSON array")),
            Some(b']') if !self.expect_comma => return self.finish(),
            Some(b']') => return Err(self.err("trailing comma in JSON array")),
            Some(b'{') => self.parse_object()?,
            Some(b'[') => self.parse_array()?,
            Some(b) => {
                return Err(self.err(&format!(
                    "row {} starts with {}, want an object or array",
                    self.index + 1,
                    show(b)
                )));
            }
        }
        if self.row_bad {
            self.lossy += 1;
            self.row_bad = false;
        }
        self.expect_comma = true;
        self.index += 1;
        Ok(true)
    }

    fn finish(&mut self) -> Result<bool> {
        self.bump()?;
        self.ws()?;
        if self.peek()?.is_some() {
            return Err(self.err("trailing data after JSON array"));
        }
        self.done = true;
        Ok(false)
    }

    fn parse_object(&mut self) -> Result<()> {
        self.set_shape(RowShape::Object)?;
        self.expect(b'{')?;
        self.arena.clear();
        self.keys.clear();
        self.vals.clear();
        self.ws()?;
        if self.peek()? == Some(b'}') {
            self.bump()?;
            return Ok(());
        }
        loop {
            self.ws()?;
            let key = self.parse_string()?;
            self.ws()?;
            self.expect(b':')?;
            self.ws()?;
            let val = self.parse_value()?;
            self.keys.push(key);
            self.vals.push(val);
            self.ws()?;
            match self.bump()? {
                b',' => continue,
                b'}' => return Ok(()),
                b => return Err(self.err(&format!("in object, found {}", show(b)))),
            }
        }
    }

    fn parse_array(&mut self) -> Result<()> {
        self.set_shape(RowShape::Array)?;
        self.expect(b'[')?;
        self.arena.clear();
        self.keys.clear();
        self.vals.clear();
        self.ws()?;
        if self.peek()? == Some(b']') {
            self.bump()?;
            return Ok(());
        }
        loop {
            self.ws()?;
            let val = self.parse_value()?;
            self.vals.push(val);
            self.ws()?;
            match self.bump()? {
                b',' => continue,
                b']' => return Ok(()),
                b => return Err(self.err(&format!("in array, found {}", show(b)))),
            }
        }
    }

    fn set_shape(&mut self, shape: RowShape) -> Result<()> {
        match self.shape {
            None => self.shape = Some(shape),
            Some(prev) if prev == shape => {}
            Some(_) => {
                return Err(self.err(
                    "JSON array mixes objects and arrays; the first element decides the shape",
                ));
            }
        }
        Ok(())
    }

    fn parse_value(&mut self) -> Result<(u32, u32)> {
        match self.peek()? {
            Some(b'"') => self.parse_string(),
            Some(b't') => self.parse_ident(b"true"),
            Some(b'f') => self.parse_ident(b"false"),
            Some(b'n') => self.parse_ident(b"null"),
            Some(b'-') | Some(b'0'..=b'9') => self.parse_number(),
            Some(b'{') | Some(b'[') => Err(self.err(&format!(
                "row {}: nested values are outside the tabular subset",
                self.index + 1
            ))),
            Some(b) => Err(self.err(&format!("bad value {}", show(b)))),
            None => Err(self.err("unexpected end of JSON")),
        }
    }

    fn parse_string(&mut self) -> Result<(u32, u32)> {
        self.expect(b'"')?;
        let start = self.arena.len();
        loop {
            if self.arena.len() > MAX_RECORD_BYTES {
                return Err(self.err("JSON string exceeds the record cap"));
            }
            let b = self.bump()?;
            if b == b'"' {
                break;
            }
            if b == b'\\' {
                match self.bump()? {
                    b'"' => self.arena.push(b'"'),
                    b'\\' => self.arena.push(b'\\'),
                    b'/' => self.arena.push(b'/'),
                    b'b' => self.arena.push(0x08),
                    b'f' => self.arena.push(0x0c),
                    b'n' => self.arena.push(b'\n'),
                    b'r' => self.arena.push(b'\r'),
                    b't' => self.arena.push(b'\t'),
                    b'u' => self.push_unicode_escape()?,
                    other => {
                        return Err(self.err(&format!("bad JSON escape {}", show(other))));
                    }
                }
            } else {
                self.arena.push(b);
            }
        }
        self.repair_span(start);
        Ok((start as u32, self.arena.len() as u32))
    }

    fn push_unicode_escape(&mut self) -> Result<()> {
        let cp = self.hex4()?;
        if !(0xD800..=0xDBFF).contains(&cp) {
            if (0xDC00..=0xDFFF).contains(&cp) {
                self.push_cp(0xFFFD);
                self.row_bad = true;
            } else {
                self.push_cp(cp);
            }
            return Ok(());
        }
        // A high surrogate has to be paired. There is no rewind: the input
        // window may compact as soon as we read past it, so a byte we decide
        // not to treat as a pair is parsed here instead of put back.
        if self.peek()? != Some(b'\\') {
            self.push_cp(0xFFFD);
            self.row_bad = true;
            return Ok(());
        }
        self.bump()?;
        if self.peek()? != Some(b'u') {
            self.push_cp(0xFFFD);
            self.row_bad = true;
            return self.escape_after_backslash();
        }
        self.bump()?;
        let low = self.hex4()?;
        if (0xDC00..=0xDFFF).contains(&low) {
            let u = 0x10000 + (((cp - 0xD800) << 10) | (low - 0xDC00));
            self.push_cp(u);
            return Ok(());
        }
        self.push_cp(0xFFFD);
        self.row_bad = true;
        if !(0xD800..=0xDFFF).contains(&low) {
            self.push_cp(low);
        } else {
            self.push_cp(0xFFFD);
        }
        Ok(())
    }

    fn escape_after_backslash(&mut self) -> Result<()> {
        match self.bump()? {
            b'"' => self.arena.push(b'"'),
            b'\\' => self.arena.push(b'\\'),
            b'/' => self.arena.push(b'/'),
            b'b' => self.arena.push(0x08),
            b'f' => self.arena.push(0x0c),
            b'n' => self.arena.push(b'\n'),
            b'r' => self.arena.push(b'\r'),
            b't' => self.arena.push(b'\t'),
            b'u' => return self.push_unicode_escape(),
            other => return Err(self.err(&format!("bad JSON escape {}", show(other)))),
        }
        Ok(())
    }

    fn hex4(&mut self) -> Result<u32> {
        let mut cp = 0u32;
        for _ in 0..4 {
            let b = self.bump()?;
            let d = match b {
                b'0'..=b'9' => b - b'0',
                b'a'..=b'f' => b - b'a' + 10,
                b'A'..=b'F' => b - b'A' + 10,
                _ => return Err(self.err("bad \\u escape")),
            };
            cp = (cp << 4) | u32::from(d);
        }
        Ok(cp)
    }

    fn push_cp(&mut self, cp: u32) {
        let c = char::from_u32(cp).unwrap_or('\u{FFFD}');
        if c == '\u{FFFD}' && cp != 0xFFFD {
            self.row_bad = true;
        }
        let mut tmp = [0u8; 4];
        self.arena.extend(c.encode_utf8(&mut tmp).as_bytes());
    }

    fn parse_ident(&mut self, ident: &[u8]) -> Result<(u32, u32)> {
        for &want in ident {
            let got = self.bump()?;
            if got != want {
                return Err(self.err(&format!("expected {}, found {}", show(want), show(got))));
            }
        }
        // null is an empty cell. The four letters are not a value.
        if ident == b"null" {
            let at = self.arena.len() as u32;
            return Ok((at, at));
        }
        let start = self.arena.len();
        self.arena.extend_from_slice(ident);
        Ok((start as u32, self.arena.len() as u32))
    }

    fn parse_number(&mut self) -> Result<(u32, u32)> {
        let start = self.arena.len();
        if self.peek()? == Some(b'-') {
            let b = self.bump()?;
            self.arena.push(b);
        }
        match self.peek()? {
            Some(b'0') => {
                let b = self.bump()?;
                self.arena.push(b);
            }
            Some(b'1'..=b'9') => {
                let b = self.bump()?;
                self.arena.push(b);
                while matches!(self.peek()?, Some(b'0'..=b'9')) {
                    let b = self.bump()?;
                    self.arena.push(b);
                }
            }
            _ => return Err(self.err("bad number")),
        }
        if self.peek()? == Some(b'.') {
            let b = self.bump()?;
            self.arena.push(b);
            let mut digits = 0;
            while matches!(self.peek()?, Some(b'0'..=b'9')) {
                let b = self.bump()?;
                self.arena.push(b);
                digits += 1;
            }
            if digits == 0 {
                return Err(self.err("bad number"));
            }
        }
        if matches!(self.peek()?, Some(b'e' | b'E')) {
            let b = self.bump()?;
            self.arena.push(b);
            if matches!(self.peek()?, Some(b'+' | b'-')) {
                let b = self.bump()?;
                self.arena.push(b);
            }
            let mut digits = 0;
            while matches!(self.peek()?, Some(b'0'..=b'9')) {
                let b = self.bump()?;
                self.arena.push(b);
                digits += 1;
            }
            if digits == 0 {
                return Err(self.err("bad number"));
            }
        }
        Ok((start as u32, self.arena.len() as u32))
    }

    fn repair_span(&mut self, start: usize) {
        if std::str::from_utf8(&self.arena[start..]).is_ok() {
            return;
        }
        let fixed = String::from_utf8_lossy(&self.arena[start..]).into_owned();
        self.arena.truncate(start);
        self.arena.extend(fixed.as_bytes());
        self.row_bad = true;
    }

    fn expect(&mut self, want: u8) -> Result<()> {
        let got = self.bump()?;
        if got != want {
            return Err(self.err(&format!("expected {}, found {}", show(want), show(got))));
        }
        Ok(())
    }

    fn ws(&mut self) -> Result<()> {
        loop {
            match self.peek()? {
                Some(b' ' | b'\t' | b'\n' | b'\r') => {
                    self.pos += 1;
                }
                _ => return Ok(()),
            }
        }
    }

    fn peek(&mut self) -> Result<Option<u8>> {
        self.ensure()?;
        Ok(self.buf.get(self.pos).copied())
    }

    fn bump(&mut self) -> Result<u8> {
        match self.peek()? {
            Some(b) => {
                self.pos += 1;
                Ok(b)
            }
            None => Err(self.err("unexpected end of JSON")),
        }
    }

    fn ensure(&mut self) -> Result<()> {
        loop {
            if self.pos < self.buf.len() && self.bom_done {
                return Ok(());
            }
            if self.eof {
                self.consume_bom();
                return Ok(());
            }
            if self.pos >= self.buf.len() && self.pos > 4096 {
                self.buf.drain(..self.pos);
                self.pos = 0;
            }
            let mut chunk = [0u8; 8192];
            let n = self.src.read(&mut chunk)?;
            if n == 0 {
                self.eof = true;
                self.consume_bom();
                return Ok(());
            }
            self.buf.extend_from_slice(&chunk[..n]);
            self.consume_bom();
        }
    }

    fn consume_bom(&mut self) {
        if self.bom_done {
            return;
        }
        const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];
        if self.buf.len() < BOM.len() && !self.eof && BOM.starts_with(&self.buf) {
            return;
        }
        self.bom_done = true;
        if self.buf.starts_with(BOM) {
            self.buf.drain(..BOM.len());
        }
    }

    fn err(&self, message: &str) -> Error {
        Error::new(format!("row {}: {message}", self.index + 1))
    }
}

fn show(b: u8) -> String {
    match b {
        b' '..=b'~' => format!("'{}'", b as char),
        _ => format!("0x{b:02x}"),
    }
}

pub fn write_string<W: std::io::Write>(out: &mut W, text: &str) -> Result<()> {
    out.write_all(b"\"")?;
    let bytes = text.as_bytes();
    let mut start = 0;
    for (i, &c) in bytes.iter().enumerate() {
        let esc: &[u8] = match c {
            b'"' => b"\\\"",
            b'\\' => b"\\\\",
            b'\n' => b"\\n",
            b'\r' => b"\\r",
            b'\t' => b"\\t",
            0x00..=0x1F => {
                out.write_all(&bytes[start..i])?;
                let hex = b"0123456789abcdef";
                let buf = [
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    hex[(c >> 4) as usize],
                    hex[(c & 0x0f) as usize],
                ];
                out.write_all(&buf)?;
                start = i + 1;
                continue;
            }
            _ => continue,
        };
        out.write_all(&bytes[start..i])?;
        out.write_all(esc)?;
        start = i + 1;
    }
    out.write_all(&bytes[start..])?;
    out.write_all(b"\"")?;
    Ok(())
}
