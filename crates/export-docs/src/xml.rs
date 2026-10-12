//! Pull XML parser. One token at a time, window capped, no DOM.
//! The sink must copy anything it wants to keep; the buffer is reused.

use std::io::Read;

use crate::error::{Error, Result};

pub const XML_WINDOW: usize = 64 * 1024;
const READ_CHUNK: usize = 8 * 1024;

#[derive(Debug)]
pub struct Attr {
    pub name: String,
    pub value: String,
}

pub trait XmlSink {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()>;
    fn end(&mut self, name: &str) -> Result<()>;
    fn text(&mut self, text: &str) -> Result<()>;
}

#[derive(Debug, Default)]
pub struct XmlStats {
    pub scratch_peak: usize,
}

pub fn pull<R: Read, S: XmlSink>(src: R, sink: &mut S) -> Result<XmlStats> {
    let mut p = Parser {
        src,
        buf: Vec::with_capacity(READ_CHUNK),
        pos: 0,
        eof: false,
        peak: 0,
        text: String::new(),
        attrs: Vec::new(),
    };
    loop {
        if p.pos >= p.buf.len() && !p.fill()? && p.pos >= p.buf.len() {
            break;
        }
        if p.buf[p.pos] == b'<' {
            p.tag(sink)?;
        } else {
            p.text_token(sink)?;
        }
    }
    Ok(XmlStats {
        scratch_peak: p.peak.max(p.text.capacity()).max(READ_CHUNK.min(XML_WINDOW)),
    })
}

struct Parser<R> {
    src: R,
    buf: Vec<u8>,
    pos: usize,
    eof: bool,
    peak: usize,
    text: String,
    attrs: Vec<Attr>,
}

impl<R: Read> Parser<R> {
    fn fill(&mut self) -> Result<bool> {
        if self.pos > 0 {
            self.buf.drain(..self.pos);
            self.pos = 0;
        }
        if self.eof {
            return Ok(false);
        }
        if self.buf.len() >= XML_WINDOW {
            return Err(Error::new("xml: token exceeds window"));
        }
        let want = READ_CHUNK.min(XML_WINDOW - self.buf.len());
        let mut tmp = [0u8; READ_CHUNK];
        let n = self
            .src
            .read(&mut tmp[..want])
            .map_err(|e| Error::new(format!("xml: {e}")))?;
        if n == 0 {
            self.eof = true;
            return Ok(false);
        }
        self.buf.extend_from_slice(&tmp[..n]);
        self.peak = self.peak.max(self.buf.len());
        Ok(true)
    }

    fn rest(&self) -> &[u8] {
        &self.buf[self.pos..]
    }

    fn tag<S: XmlSink>(&mut self, sink: &mut S) -> Result<()> {
        loop {
            if let Some(rel) = find_bytes(self.rest(), b">") {
                break self.emit_tag(rel, sink);
            }
            if self.buf.len() >= XML_WINDOW && self.pos == 0 {
                return Err(Error::new("xml: token exceeds window"));
            }
            if !self.fill()? {
                return Err(Error::new("xml: truncated tag"));
            }
        }
    }

    fn emit_tag<S: XmlSink>(&mut self, rel_gt: usize, sink: &mut S) -> Result<()> {
        let start = self.pos + 1;
        let end = self.pos + rel_gt;
        let raw = String::from_utf8_lossy(&self.buf[start..end]).into_owned();
        self.pos = end + 1;
        let body = raw.trim();
        if body.starts_with('?') {
            return Ok(());
        }
        if let Some(inner) = body.strip_prefix("!--") {
            if !inner.ends_with("--") && !body.ends_with("--") {
                // `>` inside a comment is rare; we closed at the first `>`.
                // A well-formed comment ends with `-->`, so `body` ends with `--`.
            }
            if !body.ends_with("--") {
                return Err(Error::new("xml: truncated comment"));
            }
            return Ok(());
        }
        if body.starts_with('!') {
            return Ok(());
        }
        if let Some(name) = body.strip_prefix('/') {
            let name = name.split_whitespace().next().unwrap_or(name).trim();
            sink.end(name)?;
            return Ok(());
        }
        let empty = body.ends_with('/');
        let body = if empty { body.trim_end_matches('/').trim() } else { body };
        let (name, attr_src) = split_name(body);
        self.attrs.clear();
        parse_attrs(attr_src, &mut self.attrs)?;
        sink.start(name, &self.attrs)?;
        if empty {
            sink.end(name)?;
        }
        Ok(())
    }

    fn text_token<S: XmlSink>(&mut self, sink: &mut S) -> Result<()> {
        loop {
            if let Some(rel) = find_bytes(self.rest(), b"<") {
                let end = self.pos + rel;
                self.emit_text(end, sink)?;
                return Ok(());
            }
            if self.eof {
                let end = self.buf.len();
                if self.pos < end {
                    self.emit_text(end, sink)?;
                }
                return Ok(());
            }
            if self.buf.len() >= XML_WINDOW && self.pos == 0 {
                let cut = cut_text(&self.buf, XML_WINDOW - 64);
                self.emit_text(cut, sink)?;
                return Ok(());
            }
            if !self.fill()? {
                continue;
            }
        }
    }

    fn emit_text<S: XmlSink>(&mut self, end: usize, sink: &mut S) -> Result<()> {
        let raw = std::str::from_utf8(&self.buf[self.pos..end])
            .map_err(|_| Error::new("xml: text is not utf-8"))?;
        decode_entities(raw, &mut self.text)?;
        self.pos = end;
        if !self.text.is_empty() {
            sink.text(&self.text)?;
        }
        Ok(())
    }
}

fn find_bytes(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn cut_text(buf: &[u8], max: usize) -> usize {
    let mut i = max.min(buf.len());
    if let Some(rel) = buf[..i].iter().rposition(|b| *b == b'&') {
        if !buf[rel..i].contains(&b';') {
            i = rel;
        }
    }
    while i > 0 && (buf[i] & 0b1100_0000) == 0b1000_0000 {
        i -= 1;
    }
    if i == 0 {
        max.min(buf.len())
    } else {
        i
    }
}

fn split_name(body: &str) -> (&str, &str) {
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
        i += 1;
    }
    (body[..i].trim(), body[i..].trim())
}

fn parse_attrs(mut src: &str, out: &mut Vec<Attr>) -> Result<()> {
    src = src.trim();
    while !src.is_empty() {
        let bytes = src.as_bytes();
        let mut i = 0;
        while i < bytes.len() && bytes[i] != b'=' && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i == 0 {
            return Err(Error::new("xml: bad attribute"));
        }
        let name = src[..i].to_string();
        src = src[i..].trim_start();
        if !src.starts_with('=') {
            return Err(Error::new("xml: attribute missing value"));
        }
        src = src[1..].trim_start();
        let quote = src.as_bytes().first().copied().unwrap_or(0);
        if quote != b'"' && quote != b'\'' {
            return Err(Error::new("xml: attribute value is not quoted"));
        }
        src = &src[1..];
        let end = src
            .find(quote as char)
            .ok_or_else(|| Error::new("xml: unterminated attribute"))?;
        let mut value = String::new();
        decode_entities(&src[..end], &mut value)?;
        out.push(Attr { name, value });
        src = src[end + 1..].trim_start();
    }
    Ok(())
}

pub(crate) fn unescape(raw: &str) -> Result<String> {
    let mut out = String::new();
    decode_entities(raw, &mut out)?;
    Ok(out)
}

fn decode_entities(raw: &str, out: &mut String) -> Result<()> {
    out.clear();
    let b = raw.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'&' {
            if let Some(rel) = raw[i + 1..].find(';') {
                let ent = &raw[i + 1..i + 1 + rel];
                if let Some(ch) = entity(ent)? {
                    out.push(ch);
                    i = i + 1 + rel + 1;
                    continue;
                }
            }
        }
        let ch = raw[i..]
            .chars()
            .next()
            .ok_or_else(|| Error::new("xml: bad utf-8"))?;
        out.push(ch);
        i += ch.len_utf8();
    }
    Ok(())
}

fn entity(ent: &str) -> Result<Option<char>> {
    Ok(Some(match ent {
        "amp" => '&',
        "lt" => '<',
        "gt" => '>',
        "quot" => '"',
        "apos" => '\'',
        _ if ent.starts_with("#x") || ent.starts_with("#X") => {
            let n = u32::from_str_radix(&ent[2..], 16)
                .map_err(|_| Error::new("xml: bad numeric entity"))?;
            char::from_u32(n).ok_or_else(|| Error::new("xml: bad numeric entity"))?
        }
        _ if ent.starts_with('#') => {
            let n = ent[1..]
                .parse::<u32>()
                .map_err(|_| Error::new("xml: bad numeric entity"))?;
            char::from_u32(n).ok_or_else(|| Error::new("xml: bad numeric entity"))?
        }
        _ => return Ok(None),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    struct Collect(Vec<String>);
    impl XmlSink for Collect {
        fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
            let mut s = format!("<{name}");
            for a in attrs {
                s.push_str(&format!(" {}={}", a.name, a.value));
            }
            s.push('>');
            self.0.push(s);
            Ok(())
        }
        fn end(&mut self, name: &str) -> Result<()> {
            self.0.push(format!("</{name}>"));
            Ok(())
        }
        fn text(&mut self, text: &str) -> Result<()> {
            self.0.push(text.to_string());
            Ok(())
        }
    }

    #[test]
    fn pulls_text_and_attrs() {
        let xml = br#"<?xml version="1.0"?><w:p w:val="Heading1"><w:t>A &amp; B</w:t></w:p>"#;
        let mut c = Collect(Vec::new());
        let stats = pull(Cursor::new(&xml[..]), &mut c).unwrap();
        assert!(stats.scratch_peak <= XML_WINDOW);
        let joined = c.0.join("|");
        assert!(joined.contains("<w:p w:val=Heading1>"), "{joined}");
        assert!(joined.contains("A & B"), "{joined}");
        assert!(joined.contains("</w:t>"), "{joined}");
    }
}
