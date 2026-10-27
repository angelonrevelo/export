//! DOCX and ODT to text or HTML. Unzip one XML part and pull-parse it.
//! Headers, footers, and footnotes are not read.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::error::{Error, Result};
use crate::stats::RunStats;
use crate::xml::{self, Attr, XmlSink};
use crate::zip::{self, ArchiveIndex};

pub fn docx_to_text<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    convert_ooxml(src, "word/document.xml", false, out, Kind::Docx)
}

pub fn docx_to_html<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    convert_ooxml(src, "word/document.xml", true, out, Kind::Docx)
}

pub fn odt_to_text<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    convert_ooxml(src, "content.xml", false, out, Kind::Odt)
}

pub fn odt_to_html<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    convert_ooxml(src, "content.xml", true, out, Kind::Odt)
}

#[derive(Clone, Copy)]
enum Kind {
    Docx,
    Odt,
}

fn convert_ooxml<R: Read + Seek, W: Write>(
    src: &mut R,
    part: &str,
    html: bool,
    out: &mut W,
    kind: Kind,
) -> Result<RunStats> {
    let input_bytes = src
        .seek(SeekFrom::End(0))
        .map_err(|e| Error::new(format!("docx: {e}")))?;
    src.seek(SeekFrom::Start(0))
        .map_err(|e| Error::new(format!("docx: {e}")))?;
    let index = ArchiveIndex::read(src)?;
    if index.by_name(part).is_none() {
        return Err(Error::new(format!("docx: missing {part}")));
    }
    // The numbering part decides list markers, so it is read before the
    // document entry takes the reader.
    let numbering = if !html && matches!(kind, Kind::Docx) {
        match index.by_name("word/numbering.xml") {
            Some(_) => {
                let mut entry = zip::open_entry(src, &index, "word/numbering.xml")?;
                let mut bytes = Vec::new();
                entry
                    .read_to_end(&mut bytes)
                    .map_err(|e| Error::new(format!("docx: {e}")))?;
                crate::plain::parse_numbering(&bytes)
            }
            None => crate::plain::Numbering::default(),
        }
    } else {
        crate::plain::Numbering::default()
    };
    let mut reader = zip::open_entry(src, &index, part)?;
    let zip_scratch = reader.scratch_peak();
    let mut out = CountWrite::new(out);
    if html {
        out.write_all(b"<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head><body>\n")
            .map_err(|e| Error::new(e.to_string()))?;
    }
    let stats = if !html && matches!(kind, Kind::Docx) {
        let mut sink = crate::plain::TextSink::new(&mut out, numbering);
        let stats = xml::pull(&mut reader, &mut sink)?;
        sink.finish_document()?;
        stats
    } else {
        let mut sink = Sink {
        out: &mut out,
        html,
        kind,
        skip: 0,
        in_t: false,
        in_p: false,
        opened: false,
        heading: 0,
        bold: false,
        italic: false,
        strong: false,
        em: false,
        table: 0,
        cells: 0,
        };
        xml::pull(&mut reader, &mut sink)?
    };
    if html {
        out.write_all(b"\n</body></html>\n")
            .map_err(|e| Error::new(e.to_string()))?;
    }
    Ok(RunStats {
        scratch_peak: zip_scratch.saturating_add(stats.scratch_peak),
        input_bytes,
        output_bytes: out.n,
    })
}

struct Sink<'a, W> {
    out: &'a mut W,
    html: bool,
    kind: Kind,
    skip: u32,
    in_t: bool,
    in_p: bool,
    opened: bool,
    heading: u8,
    bold: bool,
    italic: bool,
    strong: bool,
    em: bool,
    table: u32,
    cells: usize,
}

impl<W: Write> Sink<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.out
            .write_all(bytes)
            .map_err(|e| Error::new(e.to_string()))
    }

    fn write_str(&mut self, s: &str) -> Result<()> {
        self.write(s.as_bytes())
    }

    fn escape(&mut self, s: &str) -> Result<()> {
        if !self.html {
            return self.write_str(s);
        }
        for ch in s.chars() {
            match ch {
                '&' => self.write(b"&amp;")?,
                '<' => self.write(b"&lt;")?,
                '>' => self.write(b"&gt;")?,
                '"' => self.write(b"&quot;")?,
                _ => {
                    let mut buf = [0u8; 4];
                    self.write(ch.encode_utf8(&mut buf).as_bytes())?;
                }
            }
        }
        Ok(())
    }

    fn open_block(&mut self) -> Result<()> {
        if self.opened {
            return Ok(());
        }
        self.opened = true;
        if !self.html || self.table > 0 {
            return Ok(());
        }
        if self.heading > 0 {
            self.write_str(&format!("<h{}>", self.heading))?;
        } else {
            self.write(b"<p>")?;
        }
        Ok(())
    }

    fn close_inline(&mut self) -> Result<()> {
        if self.em {
            self.write(b"</em>")?;
            self.em = false;
        }
        if self.strong {
            self.write(b"</strong>")?;
            self.strong = false;
        }
        Ok(())
    }

    fn apply_inline(&mut self) -> Result<()> {
        if !self.html {
            return Ok(());
        }
        if self.bold && !self.strong {
            self.write(b"<strong>")?;
            self.strong = true;
        }
        if !self.bold && self.strong {
            self.write(b"</strong>")?;
            self.strong = false;
        }
        if self.italic && !self.em {
            self.write(b"<em>")?;
            self.em = true;
        }
        if !self.italic && self.em {
            self.write(b"</em>")?;
            self.em = false;
        }
        Ok(())
    }

    fn close_block(&mut self) -> Result<()> {
        self.close_inline()?;
        if self.html && self.table == 0 && self.opened {
            if self.heading > 0 {
                self.write_str(&format!("</h{}>\n", self.heading))?;
            } else {
                self.write(b"</p>\n")?;
            }
        } else if !self.html && self.table == 0 && self.in_p {
            self.write(b"\n")?;
        }
        self.in_p = false;
        self.opened = false;
        self.heading = 0;
        self.bold = false;
        self.italic = false;
        Ok(())
    }
}

impl<W: Write> XmlSink for Sink<'_, W> {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        if self.skip > 0 {
            self.skip += 1;
            return Ok(());
        }
        match self.kind {
            Kind::Docx => self.start_docx(name, attrs),
            Kind::Odt => self.start_odt(name, attrs),
        }
    }

    fn end(&mut self, name: &str) -> Result<()> {
        if self.skip > 0 {
            self.skip -= 1;
            return Ok(());
        }
        match self.kind {
            Kind::Docx => self.end_docx(name),
            Kind::Odt => self.end_odt(name),
        }
    }

    fn text(&mut self, text: &str) -> Result<()> {
        if self.skip > 0 || !self.in_t {
            return Ok(());
        }
        self.open_block()?;
        self.apply_inline()?;
        self.escape(text)
    }
}

impl<W: Write> Sink<'_, W> {
    fn start_docx(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        let Some(loc) = word_local(name) else {
            return Ok(());
        };
        match loc {
            "del" | "delText" | "instrText" => {
                self.skip = 1;
            }
            "p" => {
                self.in_p = true;
                self.opened = false;
                self.heading = 0;
            }
            "pStyle" => {
                if let Some(v) = attr(attrs, "val") {
                    self.heading = heading_level(v);
                }
            }
            "b" => self.bold = !is_off(attr(attrs, "val")),
            "i" => self.italic = !is_off(attr(attrs, "val")),
            "t" => self.in_t = true,
            "tab" => {
                self.open_block()?;
                self.write(b"\t")?;
            }
            "br" | "cr" => {
                self.open_block()?;
                self.write(b"\n")?;
            }
            "tbl" => {
                self.table += 1;
                if self.html {
                    self.write(b"<table>\n")?;
                }
            }
            "tr" => {
                self.cells = 0;
                if self.html {
                    self.write(b"<tr>")?;
                }
            }
            "tc" => {
                if self.html {
                    self.write(b"<td>")?;
                } else if self.cells > 0 {
                    self.write(b"\t")?;
                }
                self.cells += 1;
            }
            _ => {}
        }
        Ok(())
    }

    fn end_docx(&mut self, name: &str) -> Result<()> {
        let Some(loc) = word_local(name) else {
            return Ok(());
        };
        match loc {
            "t" => self.in_t = false,
            "r" => {
                self.close_inline()?;
                self.bold = false;
                self.italic = false;
            }
            "p" => self.close_block()?,
            "tc" => {
                if self.html {
                    self.write(b"</td>")?;
                }
            }
            "tr" => {
                if self.html {
                    self.write(b"</tr>\n")?;
                } else {
                    self.write(b"\n")?;
                }
                self.cells = 0;
            }
            "tbl" => {
                self.table = self.table.saturating_sub(1);
                if self.html {
                    self.write(b"</table>\n")?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn start_odt(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        let Some(loc) = text_local(name) else {
            return Ok(());
        };
        match loc {
            "p" | "h" => {
                self.in_p = true;
                self.in_t = true;
                self.opened = false;
                self.heading = 0;
                if loc == "h" {
                    self.heading = attr(attrs, "outline-level")
                        .and_then(|v| v.parse().ok())
                        .unwrap_or(1u8)
                        .clamp(1, 6);
                }
            }
            "span" => {
                // ODT bold/italic lives in styles we do not resolve. Keep the text.
            }
            "tab" => {
                self.open_block()?;
                self.write(b"\t")?;
            }
            "line-break" => {
                self.open_block()?;
                self.write(b"\n")?;
            }
            "s" => {
                // text:s is a run of spaces; c= count.
                let n = attr(attrs, "c").and_then(|v| v.parse().ok()).unwrap_or(1usize);
                self.open_block()?;
                for _ in 0..n.min(256) {
                    self.write(b" ")?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn end_odt(&mut self, name: &str) -> Result<()> {
        let Some(loc) = text_local(name) else {
            return Ok(());
        };
        if loc == "p" || loc == "h" {
            self.in_t = false;
            self.close_block()?;
        }
        Ok(())
    }
}

pub(crate) fn word_local(name: &str) -> Option<&str> {
    prefixed(name, "w", "wordprocessingml/2006/main")
}

fn text_local(name: &str) -> Option<&str> {
    prefixed(name, "text", "opendocument")
}

fn prefixed<'a>(name: &'a str, prefix: &str, ns_tail: &str) -> Option<&'a str> {
    match name.rsplit_once(':') {
        Some((pre, loc)) => {
            if pre == prefix || pre.ends_with(ns_tail) {
                Some(loc)
            } else {
                None
            }
        }
        None => Some(name),
    }
}

fn attr<'a>(attrs: &'a [Attr], local: &str) -> Option<&'a str> {
    let suffix = format!(":{local}");
    attrs
        .iter()
        .find(|a| a.name == local || a.name.ends_with(&suffix))
        .map(|a| a.value.as_str())
}

fn is_off(val: Option<&str>) -> bool {
    matches!(
        val.map(|v| v.eq_ignore_ascii_case("0")
            || v.eq_ignore_ascii_case("false")
            || v.eq_ignore_ascii_case("off")),
        Some(true)
    )
}

fn heading_level(val: &str) -> u8 {
    let v = val.to_ascii_lowercase();
    if v == "title" {
        return 1;
    }
    if !v.contains("heading") {
        return 0;
    }
    v.chars()
        .filter(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse::<u8>()
        .unwrap_or(1)
        .clamp(1, 6)
}

/// Byte counter so callers can record `output_bytes` without a second buffer.
pub struct CountWrite<W> {
    inner: W,
    pub n: u64,
}

impl<W> CountWrite<W> {
    pub fn new(inner: W) -> Self {
        Self { inner, n: 0 }
    }
}

impl<W: Write> Write for CountWrite<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.n += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}
