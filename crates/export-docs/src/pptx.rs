//! pptx to HTML, and to PDF, for a generated `ppt/slides` deck of text shapes.
//! The text PDF is kept only when its text contains each slide paragraph.
//! One generated slide that embeds a PNG becomes an image XObject only when that
//! PDF is smaller than the same pixels stored uncompressed. An empty `p:pic`
//! stays a miss. LibreOffice and pandoc are not called.

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

pub fn pptx_to_pdf<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    let input_bytes = match src.seek(SeekFrom::End(0)) {
        Ok(n) => n,
        Err(err) => return Err(miss(0, &err.to_string())),
    };
    if let Err(err) = src.seek(SeekFrom::Start(0)) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    dispatch_pdf(src, out, input_bytes).map_err(|err| {
        if err.message.starts_with("MISS:") {
            err
        } else {
            miss(input_bytes, &err.message)
        }
    })
}

fn dispatch_pdf<R: Read + Seek, W: Write>(
    src: &mut R,
    out: &mut W,
    input_bytes: u64,
) -> Result<RunStats> {
    if let Some(hit) = read_raster(src)? {
        out.write_all(&hit.pdf)
            .map_err(|err| Error::new(err.to_string()))?;
        let scratch = if hit.scratch > hit.pdf.len() {
            hit.scratch
        } else {
            hit.pdf.len()
        };
        return Ok(RunStats {
            scratch_peak: scratch,
            input_bytes,
            output_bytes: hit.pdf.len() as u64,
        });
    }
    read_deck_pdf(src, out, input_bytes)
}

fn miss(input_bytes: u64, why: &str) -> Error {
    Error::new(format!("MISS: pptx {why} (input_b={input_bytes})"))
}

struct RasterHit {
    pdf: Vec<u8>,
    scratch: usize,
}

/// One stored pptx whose slide embeds a generated 80×48 RGB PNG.
pub fn pptx_raster_slide() -> Result<Vec<u8>> {
    let rgb = stripe_rgb(80, 48);
    let png = encode_png(80, 48, &rgb)?;
    Ok(pack_picture(&png))
}

/// PDF that stores the same generated pixels with no filter. Not a product conversion.
pub fn pptx_raster_stored_pdf() -> Result<Vec<u8>> {
    let rgb = stripe_rgb(80, 48);
    crate::pdf::image_xobject(80, 48, &rgb, false)
}

fn stripe_rgb(width: usize, height: usize) -> Vec<u8> {
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        let pixel = [32u8, 40 + (y as u8) * 3, 180];
        for x in 0..width {
            let at = (y * width + x) * 3;
            rgb[at] = pixel[0];
            rgb[at + 1] = pixel[1];
            rgb[at + 2] = pixel[2];
        }
    }
    rgb
}

fn pack_picture(png: &[u8]) -> Vec<u8> {
    let slide = br#"<?xml version="1.0"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
  <p:cSld><p:spTree>
    <p:pic>
      <p:nvPicPr><p:cNvPr id="2" name="Picture"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr>
      <p:blipFill><a:blip r:embed="rId1"/></p:blipFill>
      <p:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="914400" cy="548640"/></a:xfrm><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
    </p:pic>
  </p:spTree></p:cSld>
</p:sld>"#;
    let rels = br#"<?xml version="1.0"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
  <Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/image" Target="../media/image1.png"/>
</Relationships>"#;
    crate::zip::pack_stored(&[
        ("ppt/slides/slide1.xml", &slide[..]),
        ("ppt/slides/_rels/slide1.xml.rels", &rels[..]),
        ("ppt/media/image1.png", png),
    ])
}

fn read_raster<R: Read + Seek>(src: &mut R) -> Result<Option<RasterHit>> {
    let index = ArchiveIndex::read(src)?;
    let mut slides = Vec::new();
    for entry in &index.entries {
        if let Some(n) = slide_number(&entry.name) {
            slides.push((n, entry.name.clone()));
        }
    }
    if slides.len() != 1 {
        return Ok(None);
    }
    let slide_name = slides[0].1.clone();
    let xml = read_named(src, &index, &slide_name)?;
    if !xml.windows(6).any(|w| w == b"<p:pic") {
        return Ok(None);
    }
    let embed = picture_embed(&xml)?;
    let Some(embed) = embed else {
        return Ok(None);
    };
    let rels_name = rels_path(&slide_name)?;
    let rels = read_named(src, &index, &rels_name)?;
    let target = relationship_target(&rels, &embed)?;
    let media = join_part(&slide_name, &target)?;
    let png = read_named(src, &index, &media)?;
    let (width, height, rgb) = decode_png(&png)?;
    let stored = crate::pdf::image_xobject(width, height, &rgb, false)?;
    let pdf = crate::pdf::image_xobject(width, height, &rgb, true)?;
    if pdf.len() >= stored.len() {
        return Err(Error::new(format!(
            "picture pdf is not smaller than stored pixels (pdf_b={} stored_b={})",
            pdf.len(),
            stored.len()
        )));
    }
    Ok(Some(RasterHit {
        scratch: png.len().max(rgb.len()).max(pdf.len()),
        pdf,
    }))
}

fn read_named<R: Read + Seek>(src: &mut R, index: &ArchiveIndex, name: &str) -> Result<Vec<u8>> {
    let mut reader = zip::open_entry(src, index, name)?;
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .map_err(|err| Error::new(err.to_string()))?;
    Ok(buf)
}

fn picture_embed(xml: &[u8]) -> Result<Option<String>> {
    let mut sink = PicSink {
        pics: 0,
        embed: None,
    };
    xml::pull(&mut std::io::Cursor::new(xml), &mut sink)?;
    if sink.pics == 0 {
        return Ok(None);
    }
    if sink.pics != 1 {
        return Err(Error::new("picture slide is not one raster"));
    }
    Ok(sink.embed)
}

struct PicSink {
    pics: usize,
    embed: Option<String>,
}

impl XmlSink for PicSink {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        let local = local_name(name);
        if local == "pic" {
            self.pics += 1;
        }
        if local == "blip" {
            if self.embed.is_some() {
                return Err(Error::new("picture slide is not one raster"));
            }
            if let Some(id) = attr(attrs, "embed") {
                self.embed = Some(id.to_string());
            }
        }
        Ok(())
    }

    fn end(&mut self, _name: &str) -> Result<()> {
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<()> {
        if text.chars().all(char::is_whitespace) {
            Ok(())
        } else {
            Err(Error::new("picture slide is not one raster"))
        }
    }
}

fn relationship_target(xml: &[u8], id: &str) -> Result<String> {
    let mut sink = RelSink {
        want: id.to_string(),
        target: None,
    };
    xml::pull(&mut std::io::Cursor::new(xml), &mut sink)?;
    sink.target.ok_or_else(|| Error::new("picture has no image"))
}

struct RelSink {
    want: String,
    target: Option<String>,
}

impl XmlSink for RelSink {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        if local_name(name) != "Relationship" {
            return Ok(());
        }
        let Some(id) = attr(attrs, "Id") else {
            return Ok(());
        };
        if id != self.want {
            return Ok(());
        }
        if self.target.is_some() {
            return Err(Error::new("picture has no image"));
        }
        let Some(target) = attr(attrs, "Target") else {
            return Err(Error::new("picture has no image"));
        };
        self.target = Some(target.to_string());
        Ok(())
    }

    fn end(&mut self, _name: &str) -> Result<()> {
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<()> {
        if text.chars().all(char::is_whitespace) {
            Ok(())
        } else {
            Err(Error::new("picture has no image"))
        }
    }
}

fn rels_path(slide: &str) -> Result<String> {
    let (dir, file) = slide
        .rsplit_once('/')
        .ok_or_else(|| Error::new("picture has no image"))?;
    Ok(format!("{dir}/_rels/{file}.rels"))
}

fn join_part(slide: &str, target: &str) -> Result<String> {
    if target.starts_with('/') || target.contains('\\') || target.contains(':') || target.contains('#') || target.contains('?') {
        return Err(Error::new("picture has no image"));
    }
    let mut stack: Vec<&str> = slide
        .rsplit_once('/')
        .map(|(dir, _)| dir.split('/').filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    for part in target.split('/') {
        if part.is_empty() || part == "." {
            return Err(Error::new("picture has no image"));
        }
        if part == ".." {
            if stack.pop().is_none() {
                return Err(Error::new("picture has no image"));
            }
            continue;
        }
        stack.push(part);
    }
    if stack.is_empty() {
        return Err(Error::new("picture has no image"));
    }
    Ok(stack.join("/"))
}

fn encode_png(width: u32, height: u32, rgb: &[u8]) -> Result<Vec<u8>> {
    let width_n = width as usize;
    let height_n = height as usize;
    let row = width_n
        .checked_mul(3)
        .ok_or_else(|| Error::new("image size"))?;
    if rgb.len() != row * height_n {
        return Err(Error::new("image size"));
    }
    let mut raw = Vec::with_capacity((row + 1) * height_n);
    for y in 0..height_n {
        raw.push(0);
        raw.extend_from_slice(&rgb[y * row..(y + 1) * row]);
    }
    let deflated = crate::deflate::deflate_lz77(&raw);
    if crate::deflate::inflate_vec(&deflated)? != raw {
        return Err(Error::new("png deflate did not round-trip"));
    }
    let zlib = zlib_wrap(&deflated, &raw);
    let mut ihdr = Vec::with_capacity(13);
    ihdr.extend_from_slice(&width.to_be_bytes());
    ihdr.extend_from_slice(&height.to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    let mut png = vec![137, 80, 78, 71, 13, 10, 26, 10];
    png.extend(png_chunk(b"IHDR", &ihdr));
    png.extend(png_chunk(b"IDAT", &zlib));
    png.extend(png_chunk(b"IEND", b""));
    Ok(png)
}

fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
    let mut body = Vec::with_capacity(4 + data.len());
    body.extend_from_slice(kind);
    body.extend_from_slice(data);
    let crc = crate::crc::crc32(&body);
    let mut out = Vec::with_capacity(12 + data.len());
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(&body);
    out.extend_from_slice(&crc.to_be_bytes());
    out
}

fn zlib_wrap(deflate: &[u8], raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(deflate.len() + 6);
    out.push(0x78);
    out.push(0x01);
    out.extend_from_slice(deflate);
    out.extend_from_slice(&adler32(raw).to_be_bytes());
    out
}

fn adler32(data: &[u8]) -> u32 {
    let mut a = 1u32;
    let mut b = 0u32;
    for &byte in data {
        a = (a + u32::from(byte)) % 65521;
        b = (b + a) % 65521;
    }
    (b << 16) | a
}

fn decode_png(png: &[u8]) -> Result<(u32, u32, Vec<u8>)> {
    if png.len() < 8 || png[..8] != [137, 80, 78, 71, 13, 10, 26, 10] {
        return Err(Error::new("picture is not a png raster"));
    }
    let mut i = 8usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut saw_ihdr = false;
    let mut idat = Vec::new();
    while i + 12 <= png.len() {
        let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
        let kind = png.get(i + 4..i + 8).ok_or_else(|| Error::new("picture is not a png raster"))?;
        let data_at = i + 8;
        let data_end = data_at
            .checked_add(len)
            .ok_or_else(|| Error::new("picture is not a png raster"))?;
        let crc_end = data_end
            .checked_add(4)
            .ok_or_else(|| Error::new("picture is not a png raster"))?;
        if crc_end > png.len() {
            return Err(Error::new("picture is not a png raster"));
        }
        let body = &png[i + 4..data_end];
        let expect = u32::from_be_bytes(png[data_end..crc_end].try_into().unwrap());
        if crate::crc::crc32(body) != expect {
            return Err(Error::new("picture is not a png raster"));
        }
        let data = &png[data_at..data_end];
        match kind {
            b"IHDR" => {
                if saw_ihdr || data.len() != 13 {
                    return Err(Error::new("picture is not a png raster"));
                }
                width = u32::from_be_bytes(data[0..4].try_into().unwrap());
                height = u32::from_be_bytes(data[4..8].try_into().unwrap());
                if data[8] != 8 || data[9] != 2 || data[10] != 0 || data[11] != 0 || data[12] != 0 {
                    return Err(Error::new("picture is not a png raster"));
                }
                saw_ihdr = true;
            }
            b"IDAT" => {
                if !saw_ihdr {
                    return Err(Error::new("picture is not a png raster"));
                }
                idat.extend_from_slice(data);
            }
            b"IEND" => break,
            _ => return Err(Error::new("picture is not a png raster")),
        }
        i = crc_end;
    }
    if !saw_ihdr || width == 0 || height == 0 {
        return Err(Error::new("picture is not a png raster"));
    }
    let raw = zlib_inflate(&idat)?;
    let row = (width as usize)
        .checked_mul(3)
        .and_then(|n| n.checked_add(1))
        .ok_or_else(|| Error::new("picture is not a png raster"))?;
    if raw.len() != row * height as usize {
        return Err(Error::new("picture is not a png raster"));
    }
    let mut rgb = Vec::with_capacity(raw.len() - height as usize);
    for y in 0..height as usize {
        let line = &raw[y * row..(y + 1) * row];
        if line[0] != 0 {
            return Err(Error::new("picture is not a png raster"));
        }
        rgb.extend_from_slice(&line[1..]);
    }
    Ok((width, height, rgb))
}

fn zlib_inflate(data: &[u8]) -> Result<Vec<u8>> {
    if data.len() < 6 || data[0] != 0x78 {
        return Err(Error::new("picture is not a png raster"));
    }
    let raw = crate::deflate::inflate_vec(&data[2..data.len() - 4])?;
    let expect = u32::from_be_bytes(data[data.len() - 4..].try_into().unwrap());
    if adler32(&raw) != expect {
        return Err(Error::new("picture is not a png raster"));
    }
    Ok(raw)
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

fn read_deck_pdf<R: Read + Seek, W: Write>(
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
    let mut paras = Vec::new();
    let mut scratch = 0usize;
    for (_, name) in &slides {
        let mut reader = zip::open_entry(src, &index, name)?;
        scratch = scratch.max(reader.scratch_peak());
        let mut sink = TextSink {
            stack: Vec::new(),
            saw_tx: false,
            in_para: false,
            in_text: false,
            bold: false,
            italic: false,
            runs: Vec::new(),
            saw_sld: false,
            paras: Vec::new(),
        };
        let stats = xml::pull(&mut reader, &mut sink)?;
        scratch = scratch.max(stats.scratch_peak);
        if !sink.saw_sld {
            return Err(Error::new("slide is not a text shape"));
        }
        paras.extend(sink.paras);
    }
    if paras.is_empty() {
        return Err(Error::new("slide is not a text shape"));
    }
    let pdf = deck_pdf(&paras)?;
    let extracted = crate::pdf::plain_text(&pdf)?;
    if !paras.iter().all(|text| extracted.contains(text)) {
        return Err(Error::new("pdf text does not match"));
    }
    out.write_all(&pdf).map_err(|e| Error::new(e.to_string()))?;
    let peak = if scratch > pdf.len() { scratch } else { pdf.len() };
    Ok(RunStats {
        scratch_peak: peak,
        input_bytes,
        output_bytes: pdf.len() as u64,
    })
}

fn deck_pdf(paras: &[String]) -> Result<Vec<u8>> {
    let lines: Vec<crate::pdf::Line> = paras
        .iter()
        .map(|text| crate::pdf::Line {
            spans: vec![crate::pdf::Span {
                text: text.clone(),
                font: 0,
                size: 11.0,
            }],
            leading: 14.0,
        })
        .collect();
    crate::pdf::from_lines(&lines)
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

struct TextSink {
    stack: Vec<String>,
    saw_tx: bool,
    in_para: bool,
    in_text: bool,
    bold: bool,
    italic: bool,
    runs: Vec<Run>,
    saw_sld: bool,
    paras: Vec<String>,
}

impl TextSink {
    fn inside(&self, local: &str) -> bool {
        self.stack.iter().any(|name| name == local)
    }

    fn flush_para(&mut self) {
        let runs = std::mem::take(&mut self.runs);
        let mut text = String::new();
        for run in &runs {
            text.push_str(&run.text);
        }
        if !text.is_empty() {
            self.paras.push(text);
        }
    }
}

impl XmlSink for TextSink {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        let local = local_name(name);
        if !allowed(local) {
            return Err(Error::new("slide is not a text shape"));
        }
        match local {
            "sld" => self.saw_sld = true,
            "sp" => self.saw_tx = false,
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
            self.flush_para();
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
        let err = crate::convert::convert("pptx", "pdf", &bytes).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not a text shape"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", bytes.len())),
            "{err}"
        );
        assert_eq!(bytes.len(), 296);
    }

    #[test]
    fn text_shapes_become_pdf_with_the_slide_text() {
        let first = slide("Hello &amp; Co", "Bold line");
        let second = slide("Second", "alpha beta gamma delta text shape");
        let bytes = pack_stored(&[
            ("ppt/slides/slide2.xml", second.as_bytes()),
            ("ppt/slides/slide1.xml", first.as_bytes()),
        ]);
        let pdf = crate::convert::convert("pptx", "pdf", &bytes).unwrap();
        let text = crate::pdf::plain_text(&pdf).unwrap();
        let hello = text.find("Hello & Co").unwrap();
        let bold = text.find("Bold line").unwrap();
        let second_at = text.find("Second").unwrap();
        let body = text.find("alpha beta gamma delta text shape").unwrap();
        assert!(hello < bold && bold < second_at && second_at < body, "{text}");
    }

    #[test]
    fn forty_slide_text_pdf_stays_the_published_size() {
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        for n in 1..=40 {
            let xml = format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/><a:t>Slide {n} title</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Body"/><p:cNvSpPr/><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr b="1"/><a:t>alpha beta gamma delta text shape</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#
            );
            files.push((format!("ppt/slides/slide{n}.xml"), xml.into_bytes()));
        }
        let refs: Vec<(&str, &[u8])> = files
            .iter()
            .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
            .collect();
        let deck = pack_stored(&refs);
        assert_eq!(deck.len(), 46515);
        let pdf = crate::convert::convert("pptx", "pdf", &deck).unwrap();
        assert_eq!(pdf.len(), 6725);
        let text = crate::pdf::plain_text(&pdf).unwrap();
        assert!(text.contains("Slide 1 title"));
        assert!(text.contains("Slide 40 title"));
        assert!(text.contains("alpha beta gamma delta text shape"));
    }

    #[test]
    fn raster_slide_becomes_a_smaller_image_xobject() {
        let bytes = pptx_raster_slide().unwrap();
        assert!(bytes.windows(8).any(|w| w == [137, 80, 78, 71, 13, 10, 26, 10]));
        assert!(bytes.windows(6).any(|w| w == *b"<p:pic"));
        assert!(bytes.windows(14).any(|w| w == *b"r:embed=\"rId1\""));
        let pdf = crate::convert::convert("pptx", "pdf", &bytes).unwrap();
        assert!(pdf.windows(15).any(|w| w == b"/Subtype /Image"));
        assert!(pdf.windows(12).any(|w| w == b"/FlateDecode"));
        assert!(pdf.windows(8).any(|w| w == b"/Im1 Do "));
        assert!(pdf.windows(9).any(|w| w == b"/Width 80"));
        assert!(pdf.windows(10).any(|w| w == b"/Height 48"));
        let stored = pptx_raster_stored_pdf().unwrap();
        assert!(
            !stored.windows(12).any(|w| w == b"/FlateDecode"),
            "stored copy must keep the raw pixels"
        );
        assert!(
            pdf.len() < stored.len(),
            "pdf_b={} stored_b={}",
            pdf.len(),
            stored.len()
        );
    }

    #[test]
    fn incompressible_raster_reports_both_sizes() {
        let mut rgb = vec![0u8; 24 * 24 * 3];
        let mut state = 0xA5A5_1234u32;
        for byte in &mut rgb {
            state = state.wrapping_mul(1664525).wrapping_add(1013904223);
            *byte = (state >> 24) as u8;
        }
        let png = encode_png(24, 24, &rgb).unwrap();
        let bytes = pack_picture(&png);
        let err = crate::convert::convert("pptx", "pdf", &bytes).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("pdf_b="), "{err}");
        assert!(err.message.contains("stored_b="), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", bytes.len())),
            "{err}"
        );
    }
}
