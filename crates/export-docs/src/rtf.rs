//! RTF to HTML for a generated file of paragraphs and bold.
//! Any other file stays a miss. Pandoc is not called.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::docx::CountWrite;
use crate::error::{Error, Result};
use crate::stats::RunStats;

const MAX_INPUT: u64 = 32 * 1024 * 1024;

pub fn rtf_to_html<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    let input_bytes = match src.seek(SeekFrom::End(0)) {
        Ok(n) => n,
        Err(err) => return Err(miss(0, &err.to_string())),
    };
    if input_bytes > MAX_INPUT {
        return Err(miss(input_bytes, "file is not paragraphs and bold"));
    }
    if let Err(err) = src.seek(SeekFrom::Start(0)) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    let mut bytes = Vec::new();
    if let Err(err) = src.read_to_end(&mut bytes) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    if bytes.len() as u64 > MAX_INPUT {
        return Err(miss(input_bytes, "file is not paragraphs and bold"));
    }
    let html = render(&bytes).map_err(|err| {
        if err.message.starts_with("MISS:") {
            err
        } else {
            miss(input_bytes, &err.message)
        }
    })?;
    let mut counted = CountWrite::new(out);
    counted
        .write_all(html.as_bytes())
        .map_err(|err| miss(input_bytes, &err.to_string()))?;
    Ok(RunStats {
        scratch_peak: bytes.len(),
        input_bytes,
        output_bytes: counted.n,
    })
}

fn miss(input_bytes: u64, why: &str) -> Error {
    Error::new(format!("MISS: rtf {why} (input_b={input_bytes})"))
}

fn shape() -> Error {
    Error::new("file is not paragraphs and bold")
}

fn render(bytes: &[u8]) -> Result<String> {
    let mut parser = Parser {
        bytes,
        i: 0,
        bold: false,
        stack: Vec::new(),
        para: String::new(),
        body: String::new(),
        strong: false,
        saw: false,
    };
    parser.parse()?;
    let mut html = String::with_capacity(parser.body.len() + 64);
    html.push_str("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>");
    html.push_str(&parser.body);
    html.push_str("</body></html>");
    Ok(html)
}

struct Word {
    start: usize,
    end: usize,
    arg: Option<i32>,
}

struct Parser<'a> {
    bytes: &'a [u8],
    i: usize,
    bold: bool,
    stack: Vec<bool>,
    para: String,
    body: String,
    strong: bool,
    saw: bool,
}

enum Flow {
    Cont,
    Done,
}

impl Parser<'_> {
    fn parse(&mut self) -> Result<()> {
        self.skip_newlines();
        if self.i >= self.bytes.len() || self.bytes[self.i] != b'{' {
            return Err(shape());
        }
        self.i += 1;
        self.stack.push(self.bold);
        self.skip_newlines();
        self.expect_rtf_header()?;
        self.parse_group()?;
        self.skip_newlines();
        if self.i != self.bytes.len() || !self.saw {
            return Err(shape());
        }
        Ok(())
    }

    fn expect_rtf_header(&mut self) -> Result<()> {
        if self.i >= self.bytes.len() || self.bytes[self.i] != b'\\' {
            return Err(shape());
        }
        self.i += 1;
        let word = self.read_word()?;
        if &self.bytes[word.start..word.end] != b"rtf" || word.arg.is_none() {
            return Err(shape());
        }
        Ok(())
    }

    fn parse_group(&mut self) -> Result<()> {
        loop {
            if self.i >= self.bytes.len() {
                return Err(shape());
            }
            let b = self.bytes[self.i];
            match b {
                b'{' => {
                    self.i += 1;
                    self.stack.push(self.bold);
                    if self.opens_skip()? {
                        self.skip_rest()?;
                        self.pop_bold()?;
                    }
                }
                b'}' => {
                    self.i += 1;
                    self.pop_bold()?;
                    if self.stack.is_empty() {
                        self.end_para();
                        return Ok(());
                    }
                }
                b'\\' => {
                    self.i += 1;
                    if matches!(self.on_control()?, Flow::Done) {
                        return Ok(());
                    }
                }
                b'\n' | b'\r' => self.i += 1,
                c if c.is_ascii_control() => return Err(shape()),
                c => {
                    self.i += 1;
                    self.emit(c);
                }
            }
        }
    }

    fn on_control(&mut self) -> Result<Flow> {
        if self.i >= self.bytes.len() {
            return Err(shape());
        }
        let c = self.bytes[self.i];
        if c.is_ascii_alphabetic() {
            let word = self.read_word()?;
            let w = &self.bytes[word.start..word.end];
            if w == b"par" {
                self.end_para();
                return Ok(Flow::Cont);
            }
            if w == b"b" {
                self.bold = !matches!(word.arg, Some(0));
                return Ok(Flow::Cont);
            }
            if w == b"plain" {
                self.bold = false;
                return Ok(Flow::Cont);
            }
            if ignored(w) {
                return Ok(Flow::Cont);
            }
            return Err(shape());
        }
        self.i += 1;
        match c {
            b'\\' => self.emit(b'\\'),
            b'{' => self.emit(b'{'),
            b'}' => self.emit(b'}'),
            b'\'' => {
                let hi = self.hex_digit()?;
                let lo = self.hex_digit()?;
                self.emit((hi << 4) | lo);
            }
            b'*' => {
                if self.i < self.bytes.len() && self.bytes[self.i] == b' ' {
                    self.i += 1;
                }
                self.skip_rest()?;
                self.pop_bold()?;
                if self.stack.is_empty() {
                    self.end_para();
                    return Ok(Flow::Done);
                }
                return Ok(Flow::Cont);
            }
            _ => return Err(shape()),
        }
        Ok(Flow::Cont)
    }

    fn opens_skip(&mut self) -> Result<bool> {
        let save = self.i;
        self.skip_newlines();
        if self.i >= self.bytes.len() || self.bytes[self.i] != b'\\' {
            self.i = save;
            return Ok(false);
        }
        let mark = self.i + 1;
        if mark >= self.bytes.len() {
            self.i = save;
            return Ok(false);
        }
        let c = self.bytes[mark];
        if c == b'*' {
            self.i = mark + 1;
            if self.i < self.bytes.len() && self.bytes[self.i] == b' ' {
                self.i += 1;
            }
            return Ok(true);
        }
        if !c.is_ascii_alphabetic() {
            self.i = save;
            return Ok(false);
        }
        self.i = mark;
        let word = self.read_word()?;
        let skip = is_skip_dest(&self.bytes[word.start..word.end]);
        if !skip {
            self.i = save;
        }
        Ok(skip)
    }

    fn skip_rest(&mut self) -> Result<()> {
        let mut depth = 1u32;
        while depth > 0 {
            if self.i >= self.bytes.len() {
                return Err(shape());
            }
            let b = self.bytes[self.i];
            self.i += 1;
            match b {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                b'\\' => self.skip_escaped()?,
                _ => {}
            }
        }
        Ok(())
    }

    fn skip_escaped(&mut self) -> Result<()> {
        if self.i >= self.bytes.len() {
            return Err(shape());
        }
        let c = self.bytes[self.i];
        if c.is_ascii_alphabetic() {
            let _ = self.read_word()?;
            return Ok(());
        }
        self.i += 1;
        if c == b'\'' {
            let _ = self.hex_digit()?;
            let _ = self.hex_digit()?;
        }
        Ok(())
    }

    fn read_word(&mut self) -> Result<Word> {
        let start = self.i;
        if start >= self.bytes.len() || !self.bytes[start].is_ascii_alphabetic() {
            return Err(shape());
        }
        self.i += 1;
        while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_alphabetic() {
            self.i += 1;
        }
        let end = self.i;
        let mut neg = false;
        if self.i < self.bytes.len() && self.bytes[self.i] == b'-' {
            let after = self.i + 1;
            if after < self.bytes.len() && self.bytes[after].is_ascii_digit() {
                neg = true;
                self.i += 1;
            }
        }
        let num_start = self.i;
        while self.i < self.bytes.len() && self.bytes[self.i].is_ascii_digit() {
            self.i += 1;
        }
        let arg = if self.i == num_start {
            None
        } else {
            let mut n: i32 = 0;
            for &d in &self.bytes[num_start..self.i] {
                n = n.saturating_mul(10).saturating_add((d - b'0') as i32);
            }
            if neg {
                Some(n.saturating_neg())
            } else {
                Some(n)
            }
        };
        if self.i < self.bytes.len() && self.bytes[self.i] == b' ' {
            self.i += 1;
        }
        Ok(Word { start, end, arg })
    }

    fn hex_digit(&mut self) -> Result<u8> {
        if self.i >= self.bytes.len() {
            return Err(shape());
        }
        let b = self.bytes[self.i];
        self.i += 1;
        match b {
            b'0'..=b'9' => Ok(b - b'0'),
            b'a'..=b'f' => Ok(b - b'a' + 10),
            b'A'..=b'F' => Ok(b - b'A' + 10),
            _ => Err(shape()),
        }
    }

    fn pop_bold(&mut self) -> Result<()> {
        self.bold = match self.stack.pop() {
            Some(saved) => saved,
            None => return Err(shape()),
        };
        Ok(())
    }

    fn skip_newlines(&mut self) {
        while self.i < self.bytes.len() && matches!(self.bytes[self.i], b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn emit(&mut self, b: u8) {
        if self.bold && !self.strong {
            self.para.push_str("<strong>");
            self.strong = true;
        } else if !self.bold && self.strong {
            self.para.push_str("</strong>");
            self.strong = false;
        }
        match b {
            b'&' => self.para.push_str("&amp;"),
            b'<' => self.para.push_str("&lt;"),
            b'>' => self.para.push_str("&gt;"),
            b'"' => self.para.push_str("&quot;"),
            _ => self.para.push(char::from(b)),
        }
    }

    fn end_para(&mut self) {
        if self.strong {
            self.para.push_str("</strong>");
            self.strong = false;
        }
        if self.para.is_empty() {
            return;
        }
        self.body.push_str("<p>");
        self.body.push_str(&self.para);
        self.body.push_str("</p>");
        self.para.clear();
        self.saw = true;
    }
}

fn ignored(w: &[u8]) -> bool {
    w == b"ansi"
        || w == b"mac"
        || w == b"pc"
        || w == b"pca"
        || w == b"ansicpg"
        || w == b"deff"
        || w == b"deflang"
        || w == b"adeflang"
        || w == b"f"
        || w == b"fs"
        || w == b"lang"
        || w == b"cf"
        || w == b"cb"
        || w == b"pard"
}

fn is_skip_dest(w: &[u8]) -> bool {
    w == b"fonttbl" || w == b"colortbl" || w == b"stylesheet" || w == b"info" || w == b"generator"
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run(input: &[u8]) -> Result<Vec<u8>> {
        let mut cur = Cursor::new(input);
        let mut out = Vec::new();
        rtf_to_html(&mut cur, &mut out)?;
        Ok(out)
    }

    #[test]
    fn paragraphs_and_bold() {
        let rtf = b"{\\rtf1\\ansi{\\fonttbl{\\f0 Arial;}}{\\*\\generator LibreOffice}First & line.\\par\nSecond {\\b mid} line.\\par\nThird \\b Bold\\b0 tail.\\par\ncaf\\'e9.\\par}";
        let html = String::from_utf8(run(rtf).unwrap()).unwrap();
        assert!(html.contains("<p>First &amp; line.</p>"), "{html}");
        assert!(
            html.contains("<p>Second <strong>mid</strong> line.</p>"),
            "{html}"
        );
        assert!(
            html.contains("<p>Third <strong>Bold</strong>tail.</p>"),
            "{html}"
        );
        assert!(html.contains("<p>caf\u{e9}.</p>"), "{html}");
        assert!(!html.contains("Arial"), "{html}");
        assert!(!html.contains("LibreOffice"), "{html}");
    }

    #[test]
    fn pict_stays_a_miss() {
        let rtf = b"{\\rtf1\\ansi{\\pict hello}\\par}";
        let err = run(rtf).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{}", err.message);
        assert!(err.message.contains("not"), "{}", err.message);
        assert!(
            err.message.contains(&format!("input_b={}", rtf.len())),
            "{}",
            err.message
        );
    }

    #[test]
    fn bare_rtf_and_empty_stay_errors() {
        assert!(run(br"{\rtf}").is_err());
        assert!(run(br"{\rtf1}").is_err());
    }
}
