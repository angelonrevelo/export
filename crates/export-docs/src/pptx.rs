//! pptx to HTML for a generated `ppt/slides` deck of text shapes.
//! Anything else stays a miss. LibreOffice and pandoc are not called.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::docx::CountWrite;
use crate::error::{Error, Result};
use crate::stats::RunStats;
use crate::xml::{self, Attr, XmlSink};
use crate::zip::{self, ArchiveIndex};

pub fn pptx_to_html<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    let input_bytes = match src.seek(SeekFrom::End(0)) {
        Ok(n) => n,
        Err(err) => return Err(miss(0, &err.to_string())),
    };
    if let Err(err) = src.seek(SeekFrom::Start(0)) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    read_deck(src, out, input_bytes).map_err(|err| {
        if err.message.starts_with("MISS:") {
            err
        } else {
            miss(input_bytes, &err.message)
        }
    })
}

fn miss(input_bytes: u64, why: &str) -> Error {
    Error::new(format!("MISS: pptx {why} (input_b={input_bytes})"))
}

fn read_deck<R: Read + Seek, W: Write>(
    src: &mut R,
    out: &mut W,
    input_bytes: u64,
) -> Result<RunStats> {
    let index = ArchiveIndex::read(src)?;
    let mut slides = Vec::new();
    for entry in &index.entries {
        if let Some(n) = slide_number(&entry.name) {
            slides.push((n, entry.name.clone()));
        }
    }
    if slides.is_empty() {
        return Err(Error::new("has no ppt/slides text deck"));
    }
    slides.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut counted = CountWrite::new(out);
    counted
        .write_all(b"<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head><body>\n")
        .map_err(|e| Error::new(e.to_string()))?;
    let mut scratch = 0usize;
    for (_, name) in &slides {
        let mut reader = zip::open_entry(src, &index, name)?;
        scratch = scratch.max(reader.scratch_peak());
        let mut sink = SlideSink {
            out: &mut counted,
            stack: Vec::new(),
            title: false,
            saw_tx: false,
            in_para: false,
            in_text: false,
            bold: false,
            italic: false,
            runs: Vec::new(),
            saw_sld: false,
        };
        let stats = xml::pull(&mut reader, &mut sink)?;
        scratch = scratch.max(stats.scratch_peak);
        if !sink.saw_sld {
            return Err(Error::new("slide is not a text shape"));
        }
    }
    counted
        .write_all(b"</body></html>\n")
        .map_err(|e| Error::new(e.to_string()))?;
    Ok(RunStats {
        scratch_peak: scratch,
        input_bytes,
        output_bytes: counted.n,
    })
}

fn slide_number(name: &str) -> Option<u32> {
    let rest = name.strip_prefix("ppt/slides/slide")?;
    let rest = rest.strip_suffix(".xml")?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn allowed(local: &str) -> bool {
    matches!(
        local,
        "sld"
            | "cSld"
            | "spTree"
            | "nvGrpSpPr"
            | "cNvPr"
            | "cNvGrpSpPr"
            | "nvPr"
            | "grpSpPr"
            | "xfrm"
            | "off"
            | "ext"
            | "chOff"
            | "chExt"
            | "sp"
            | "nvSpPr"
            | "cNvSpPr"
            | "ph"
            | "spPr"
            | "prstGeom"
            | "avLst"
            | "txBody"
            | "bodyPr"
            | "lstStyle"
            | "p"
            | "r"
            | "rPr"
            | "t"
            | "br"
            | "endParaRPr"
    )
}

fn attr<'a>(attrs: &'a [Attr], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| attr.name == key || local_name(&attr.name) == key)
        .map(|attr| attr.value.as_str())
}

fn flag(value: &str) -> bool {
    matches!(value, "1" | "true" | "on")
}

struct Run {
    text: String,
    bold: bool,
    italic: bool,
    br: bool,
}

struct SlideSink<'a, W> {
    out: &'a mut CountWrite<W>,
    stack: Vec<String>,
    title: bool,
    saw_tx: bool,
    in_para: bool,
    in_text: bool,
    bold: bool,
    italic: bool,
    runs: Vec<Run>,
    saw_sld: bool,
}

impl<W: Write> SlideSink<'_, W> {
    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.out.write_all(bytes).map_err(|e| Error::new(e.to_string()))
    }

    fn inside(&self, local: &str) -> bool {
        self.stack.iter().any(|name| name == local)
    }

    fn flush_para(&mut self) -> Result<()> {
        let runs = std::mem::take(&mut self.runs);
        let any = runs.iter().any(|run| run.br || !run.text.is_empty());
        if !any {
            return Ok(());
        }
        if self.title {
            self.write(b"<h1>")?;
        } else {
            self.write(b"<p>")?;
        }
        for run in &runs {
            if run.br {
                self.write(b"<br>")?;
            }
            if run.text.is_empty() {
                continue;
            }
            if run.bold {
                self.write(b"<strong>")?;
            }
            if run.italic {
                self.write(b"<em>")?;
            }
            self.escape(&run.text)?;
            if run.italic {
                self.write(b"</em>")?;
            }
            if run.bold {
                self.write(b"</strong>")?;
            }
        }
        if self.title {
            self.write(b"</h1>\n")?;
        } else {
            self.write(b"</p>\n")?;
        }
        Ok(())
    }

    fn escape(&mut self, text: &str) -> Result<()> {
        for ch in text.chars() {
            match ch {
                '&' => self.write(b"&amp;")?,
                '<' => self.write(b"&lt;")?,
                '>' => self.write(b"&gt;")?,
                '"' => self.write(b"&quot;")?,
                _ => {
                    let mut buf = [0u8; 4];
                    let encoded = ch.encode_utf8(&mut buf);
                    self.write(encoded.as_bytes())?;
                }
            }
        }
        Ok(())
    }
}

impl<W: Write> XmlSink for SlideSink<'_, W> {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        let local = local_name(name);
        if !allowed(local) {
            return Err(Error::new("slide is not a text shape"));
        }
        match local {
            "sld" => {
                self.saw_sld = true;
                self.write(b"<section>\n")?;
            }
            "sp" => {
                self.title = false;
                self.saw_tx = false;
            }
            "ph" => {
                if let Some(kind) = attr(attrs, "type") {
                    if kind == "title" || kind == "ctrTitle" {
                        self.title = true;
                    }
                }
            }
            "txBody" => self.saw_tx = true,
            "p" if self.inside("txBody") => {
                self.in_para = true;
                self.runs.clear();
            }
            "rPr" => {
                self.bold = attr(attrs, "b").is_some_and(flag);
                self.italic = attr(attrs, "i").is_some_and(flag);
            }
            "br" if self.in_para => self.runs.push(Run {
                text: String::new(),
                bold: false,
                italic: false,
                br: true,
            }),
            "t" if self.in_para => {
                self.in_text = true;
                self.runs.push(Run {
                    text: String::new(),
                    bold: self.bold,
                    italic: self.italic,
                    br: false,
                });
            }
            _ => {}
        }
        self.stack.push(local.to_string());
        Ok(())
    }

    fn end(&mut self, name: &str) -> Result<()> {
        let local = local_name(name);
        if local == "p" && self.in_para {
            self.in_para = false;
            self.flush_para()?;
        }
        if local == "t" {
            self.in_text = false;
        }
        if local == "r" {
            self.bold = false;
            self.italic = false;
        }
        if local == "sp" && !self.saw_tx {
            return Err(Error::new("slide is not a text shape"));
        }
        if local == "sld" {
            self.write(b"</section>\n")?;
        }
        if self.stack.last().map(String::as_str) == Some(local) {
            self.stack.pop();
        }
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<()> {
        if self.in_text {
            if let Some(run) = self.runs.last_mut() {
                run.text.push_str(text);
            }
            return Ok(());
        }
        if text.chars().all(char::is_whitespace) {
            return Ok(());
        }
        Err(Error::new("slide is not a text shape"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::{pack_deflated, pack_stored};
    use std::io::Cursor;

    fn slide(title: &str, body: &str) -> String {
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld>
    <p:spTree>
      <p:nvGrpSpPr>
        <p:cNvPr id="1" name=""/>
        <p:cNvGrpSpPr/>
        <p:nvPr/>
      </p:nvGrpSpPr>
      <p:grpSpPr>
        <a:xfrm>
          <a:off x="0" y="0"/>
          <a:ext cx="0" cy="0"/>
          <a:chOff x="0" y="0"/>
          <a:chExt cx="0" cy="0"/>
        </a:xfrm>
      </p:grpSpPr>
      <p:sp>
        <p:nvSpPr>
          <p:cNvPr id="2" name="Title"/>
          <p:cNvSpPr/>
          <p:nvPr><p:ph type="title"/></p:nvPr>
        </p:nvSpPr>
        <p:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
        <p:txBody>
          <a:bodyPr/>
          <a:lstStyle/>
          <a:p><a:r><a:rPr lang="en-US"/><a:t>{title}</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
      <p:sp>
        <p:nvSpPr>
          <p:cNvPr id="3" name="Body"/>
          <p:cNvSpPr/>
          <p:nvPr><p:ph type="body"/></p:nvPr>
        </p:nvSpPr>
        <p:spPr/>
        <p:txBody>
          <a:bodyPr/>
          <a:lstStyle/>
          <a:p><a:r><a:rPr b="1"/><a:t>{body}</a:t></a:r></a:p>
        </p:txBody>
      </p:sp>
    </p:spTree>
  </p:cSld>
</p:sld>"#
        )
    }

    fn html_of(bytes: &[u8]) -> Result<String> {
        let mut out = Vec::new();
        pptx_to_html(&mut Cursor::new(bytes), &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn text_shapes_become_html_in_slide_order() {
        let first = slide("Hello &amp; Co", "Bold line");
        let second = slide("Second", "Tail");
        let tenth = slide("Tenth", "Later");
        for bytes in [
            pack_stored(&[
                ("ppt/slides/slide10.xml", tenth.as_bytes()),
                ("ppt/slides/slide2.xml", second.as_bytes()),
                ("ppt/slides/slide1.xml", first.as_bytes()),
            ]),
            pack_deflated(&[
                ("ppt/slides/slide1.xml", first.as_bytes()),
                ("ppt/slides/slide2.xml", second.as_bytes()),
            ]),
        ] {
            let html = html_of(&bytes).unwrap();
            assert!(html.contains("<h1>Hello &amp; Co</h1>"), "{html}");
            assert!(html.contains("<p><strong>Bold line</strong></p>"), "{html}");
            let hello = html.find("Hello").unwrap();
            let second_at = html.find("Second").unwrap();
            assert!(hello < second_at, "{html}");
            if html.contains("Tenth") {
                assert!(second_at < html.find("Tenth").unwrap(), "{html}");
            }
        }
    }

    #[test]
    fn picture_shape_stays_a_miss() {
        let xml = br#"<?xml version="1.0"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree><p:pic/></p:spTree></p:cSld>
</p:sld>"#;
        let bytes = pack_stored(&[("ppt/slides/slide1.xml", &xml[..])]);
        let err = html_of(&bytes).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not a text shape"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", bytes.len())),
            "{err}"
        );
    }
}
