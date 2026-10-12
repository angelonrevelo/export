//! A pull parser for the XML that OOXML actually writes.
//!
//! Tags, attributes, text, comments, and the five predefined entities. A DTD
//! is refused. An entity this parser does not know is refused, rather than
//! left in the text as a guess.

use crate::error::{Error, Result};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Ev {
    Open,
    Empty,
    Close,
    Text,
}

pub(crate) struct Pull<'a> {
    b: &'a [u8],
    i: usize,
    local: String,
}

impl<'a> Pull<'a> {
    pub(crate) fn new(input: &'a [u8]) -> Self {
        let mut i = 0;
        if input.starts_with(&[0xEF, 0xBB, 0xBF]) {
            i = 3;
        }
        Self {
            b: input,
            i,
            local: String::new(),
        }
    }

    pub(crate) fn local(&self) -> &str {
        &self.local
    }

    pub(crate) fn next(
        &mut self,
        text: &mut String,
        attrs: &mut Vec<(String, String)>,
    ) -> Result<Option<Ev>> {
        if self.i >= self.b.len() {
            return Ok(None);
        }
        if self.b[self.i] != b'<' {
            text.clear();
            self.read_text(text)?;
            return Ok(Some(Ev::Text));
        }
        self.i += 1;
        if self.starts_with(b"?") {
            self.skip_until(b"?>")?;
            return self.next(text, attrs);
        }
        if self.starts_with(b"!--") {
            self.i += 3;
            self.skip_until(b"-->")?;
            return self.next(text, attrs);
        }
        if self.starts_with(b"![CDATA[") {
            self.i += 8;
            text.clear();
            let end = self
                .find(b"]]>")
                .ok_or_else(|| Error::new("unterminated cdata"))?;
            text.push_str(
                std::str::from_utf8(&self.b[self.i..end])
                    .map_err(|_| Error::new("cdata is not utf-8"))?,
            );
            self.i = end + 3;
            return Ok(Some(Ev::Text));
        }
        if self.starts_with(b"!DOCTYPE") || self.starts_with(b"!") {
            return Err(Error::new("xml dtd is refused"));
        }
        if self.starts_with(b"/") {
            self.i += 1;
            let name = self.read_name()?;
            self.skip_ws();
            self.expect(b'>')?;
            self.local = local_name(&name);
            return Ok(Some(Ev::Close));
        }
        let name = self.read_name()?;
        self.local = local_name(&name);
        attrs.clear();
        loop {
            self.skip_ws();
            if self.starts_with(b"/>") {
                self.i += 2;
                return Ok(Some(Ev::Empty));
            }
            if self.starts_with(b">") {
                self.i += 1;
                return Ok(Some(Ev::Open));
            }
            let key = self.read_name()?;
            self.skip_ws();
            self.expect(b'=')?;
            self.skip_ws();
            let quote = self.bump()?;
            if quote != b'"' && quote != b'\'' {
                return Err(Error::new("xml attribute is not quoted"));
            }
            let value = self.read_attr(quote)?;
            attrs.push((local_name(&key), value));
        }
    }

    fn read_text(&mut self, text: &mut String) -> Result<()> {
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i] != b'<' && self.b[self.i] != b'&' {
            self.i += 1;
        }
        push_utf8(text, &self.b[start..self.i])?;
        if self.i < self.b.len() && self.b[self.i] == b'&' {
            text.push(self.read_entity()?);
            if self.i < self.b.len() && self.b[self.i] != b'<' {
                self.read_text(text)?;
            }
        }
        Ok(())
    }

    fn read_attr(&mut self, quote: u8) -> Result<String> {
        let mut out = String::new();
        loop {
            if self.i >= self.b.len() {
                return Err(Error::new("unterminated xml attribute"));
            }
            if self.b[self.i] == quote {
                self.i += 1;
                return Ok(out);
            }
            if self.b[self.i] == b'&' {
                out.push(self.read_entity()?);
            } else if self.b[self.i] == b'<' {
                return Err(Error::new("raw < in xml attribute"));
            } else {
                let start = self.i;
                while self.i < self.b.len()
                    && self.b[self.i] != quote
                    && self.b[self.i] != b'&'
                    && self.b[self.i] != b'<'
                {
                    self.i += 1;
                }
                push_utf8(&mut out, &self.b[start..self.i])?;
            }
        }
    }

    fn read_entity(&mut self) -> Result<char> {
        self.expect(b'&')?;
        if self.starts_with(b"#") {
            self.i += 1;
            let hex = if self.starts_with(b"x") || self.starts_with(b"X") {
                self.i += 1;
                true
            } else {
                false
            };
            let start = self.i;
            while self.i < self.b.len() && self.b[self.i] != b';' {
                self.i += 1;
            }
            let digits = std::str::from_utf8(&self.b[start..self.i])
                .map_err(|_| Error::new("bad numeric character reference"))?;
            let cp = if hex {
                u32::from_str_radix(digits, 16)
            } else {
                digits.parse::<u32>()
            }
            .map_err(|_| Error::new("bad numeric character reference"))?;
            self.expect(b';')?;
            return char::from_u32(cp).ok_or_else(|| Error::new("bad numeric character reference"));
        }
        let start = self.i;
        while self.i < self.b.len() && self.b[self.i] != b';' {
            self.i += 1;
        }
        let name = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| Error::new("unknown xml entity"))?;
        self.expect(b';')?;
        match name {
            "amp" => Ok('&'),
            "lt" => Ok('<'),
            "gt" => Ok('>'),
            "quot" => Ok('"'),
            "apos" => Ok('\''),
            _ => Err(Error::new(format!("unknown xml entity \"{name}\""))),
        }
    }

    fn read_name(&mut self) -> Result<String> {
        let start = self.i;
        if self.i >= self.b.len() || !is_name_start(self.b[self.i]) {
            return Err(Error::new("expected an xml name"));
        }
        self.i += 1;
        while self.i < self.b.len() && is_name(self.b[self.i]) {
            self.i += 1;
        }
        let bytes = &self.b[start..self.i];
        std::str::from_utf8(bytes)
            .map(|s| s.to_string())
            .map_err(|_| Error::new("xml name is not utf-8"))
    }

    fn skip_ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<()> {
        if self.i >= self.b.len() || self.b[self.i] != byte {
            return Err(Error::new("unexpected xml byte"));
        }
        self.i += 1;
        Ok(())
    }

    fn bump(&mut self) -> Result<u8> {
        if self.i >= self.b.len() {
            return Err(Error::new("unexpected end of xml"));
        }
        let b = self.b[self.i];
        self.i += 1;
        Ok(b)
    }

    fn starts_with(&self, pat: &[u8]) -> bool {
        self.b[self.i..].starts_with(pat)
    }

    fn find(&self, pat: &[u8]) -> Option<usize> {
        self.b[self.i..]
            .windows(pat.len())
            .position(|w| w == pat)
            .map(|p| self.i + p)
    }

    fn skip_until(&mut self, pat: &[u8]) -> Result<()> {
        let end = self
            .find(pat)
            .ok_or_else(|| Error::new("unterminated xml"))?;
        self.i = end + pat.len();
        Ok(())
    }
}

fn local_name(name: &str) -> String {
    match name.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => name.to_string(),
    }
}

fn is_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b':'
}

fn is_name(b: u8) -> bool {
    is_name_start(b) || b.is_ascii_digit() || b == b'-' || b == b'.'
}

fn push_utf8(out: &mut String, bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| Error::new("xml text is not utf-8"))?;
    out.push_str(text);
    Ok(())
}

pub(crate) fn attr<'a>(attrs: &'a [(String, String)], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}
