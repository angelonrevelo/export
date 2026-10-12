//! Dispatch for the document pairs export actually runs through
//! pandoc, WeasyPrint, LibreOffice (docx→pdf), pdfplumber, and pypdf.
//! Encryption and the formats with no reader here are misses.
//! PDF shrink recompresses FlateDecode streams and keeps the file only when
//! those streams inflate to the same bytes and the file is smaller.

use std::io::{Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};

use crate::container::{self, Meta};
use crate::docx::{self, CountWrite};
use crate::error::{Error, Result};
use crate::htmlpdf::{html_to_pdf, html_to_text, text_to_html};
use crate::markdown::{markdown_to_html, markdown_to_text};
use crate::pdf;
use crate::stats::RunStats;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Pair {
    pub source: &'static str,
    pub target: &'static str,
    pub ready: bool,
    pub note: &'static str,
}

/// Pairs this crate owns, plus the misses it will not pretend to run.
pub fn pairs() -> &'static [Pair] {
    PAIRS
}

const PAIRS: &[Pair] = &[
    Pair { source: "docx", target: "txt", ready: true, note: "OOXML word/document.xml, replaces pandoc" },
    Pair { source: "docx", target: "html", ready: true, note: "OOXML word/document.xml, replaces pandoc" },
    Pair { source: "docx", target: "pdf", ready: true, note: "docx→html→text PDF, replaces LibreOffice for this hop" },
    Pair { source: "odt", target: "txt", ready: true, note: "ODF content.xml, replaces pandoc" },
    Pair { source: "odt", target: "html", ready: true, note: "ODF content.xml, replaces pandoc" },
    Pair { source: "odt", target: "pdf", ready: true, note: "odt→html→text PDF" },
    Pair { source: "md", target: "html", ready: true, note: "replaces pandoc" },
    Pair { source: "md", target: "txt", ready: true, note: "replaces pandoc plain writer" },
    Pair { source: "md", target: "pdf", ready: true, note: "md→html→text PDF, replaces WeasyPrint" },
    Pair { source: "markdown", target: "html", ready: true, note: "same path as md" },
    Pair { source: "html", target: "pdf", ready: true, note: "text pages, replaces WeasyPrint" },
    Pair { source: "html", target: "txt", ready: true, note: "tag strip" },
    Pair { source: "htm", target: "pdf", ready: true, note: "same path as html" },
    Pair { source: "txt", target: "html", ready: true, note: "paragraph wrap" },
    Pair { source: "txt", target: "pdf", ready: true, note: "txt→html→text PDF" },
    Pair { source: "pdf", target: "txt", ready: true, note: "literal strings in content streams; no ToUnicode" },
    Pair { source: "pdf", target: "split", ready: true, note: "one PDF per page, classic xref rewritten" },
    Pair { source: "pdf", target: "merge", ready: true, note: "new trailer, pages concatenated" },
    Pair { source: "pdf", target: "rotate", ready: true, note: "sets /Rotate, does not rasterize" },
    Pair { source: "pdf", target: "watermark", ready: true, note: "appends a text stream when contents are uncompressed" },
    Pair { source: "pdf", target: "shrink", ready: true, note: "recompress FlateDecode only when the new stream inflates to the same bytes and the file is smaller; otherwise MISS" },
    Pair { source: "pdf", target: "encrypt", ready: false, note: "MISS: encryption is not implemented" },
    Pair { source: "pdf", target: "decrypt", ready: false, note: "MISS: decryption is not implemented" },
    Pair { source: "pdf", target: "docx", ready: false, note: "MISS: pdf2docx layout rebuild is not implemented" },
    Pair { source: "pdf", target: "png", ready: false, note: "MISS: rasterize is out of scope" },
    Pair { source: "rtf", target: "html", ready: false, note: "MISS: only a generated rtf of paragraphs and bold converts; other files stay a miss" },
    Pair { source: "epub", target: "html", ready: false, note: "MISS: only a generated epub whose spine is one, two, three, four, or five xhtml files converts; other packages stay a miss" },
    Pair { source: "pptx", target: "html", ready: false, note: "MISS: only a generated ppt/slides deck of text shapes converts; other decks stay a miss" },
    Pair { source: "pptx", target: "pdf", ready: false, note: "MISS: text-shape decks convert, and one generated PNG slide converts only when the image XObject PDF is smaller than the stored pixels; other decks stay a miss" },
    Pair { source: "doc", target: "html", ready: false, note: "MISS: only a generated Word 97 file of paragraphs in WordDocument converts; other files stay a miss" },
    Pair { source: "doc", target: "pdf", ready: false, note: "MISS: only a generated Word 97 file of one, two, or three paragraphs converts; other files stay a miss" },
    Pair { source: "ppt", target: "pdf", ready: false, note: "MISS: a generated one-shape legacy ppt converts when the PDF text contains that string, four text shapes convert when the PDF text contains all four, one text shape plus one 24-bit, 8-bit, or 4-bit DIB picture converts when the PDF text contains the string and the picture is an image, two text shapes plus one 8-bit DIB picture convert when both strings are in the PDF text and the picture is an image, one text shape plus two 8-bit DIB pictures converts when the string is in the PDF text and both pictures are images, one text shape plus three 8-bit DIB pictures converts when the string is in the PDF text and all three pictures are images, one text shape plus four 8-bit DIB pictures converts when the string is in the PDF text and all four pictures are images, and one text shape plus five 8-bit DIB pictures converts when the string is in the PDF text and all five pictures are images, and one text shape plus six 8-bit DIB pictures converts when the string is in the PDF text and all six pictures are images; other bit depths stay a miss" },
    Pair { source: "ppt", target: "html", ready: false, note: "MISS: only a generated legacy ppt with one, two, three, four, five, six, seven, eight, nine, or ten text shapes converts; an eleventh text shape and other files stay a miss" },
    Pair { source: "xls", target: "pdf", ready: false, note: "MISS: spreadsheets are not this crate" },
];

pub fn pair_ready(source: &str, target: &str) -> bool {
    let source = key(source);
    let target = key(target);
    PAIRS
        .iter()
        .any(|p| p.ready && key(p.source) == source && key(p.target) == target)
}

pub fn convert(source_ext: &str, target: &str, input: &[u8]) -> Result<Vec<u8>> {
    let mut cur = Cursor::new(input);
    let mut out = Vec::new();
    convert_to(&mut cur, source_ext, target, &mut out)?;
    Ok(out)
}

pub fn convert_to<R: Read + Seek, W: Write>(
    input: &mut R,
    source_ext: &str,
    target: &str,
    output: &mut W,
) -> Result<RunStats> {
    let source = key(source_ext);
    let target = key(target);
    if source == "pptx" && target == "html" {
        return crate::pptx::pptx_to_html(input, output);
    }
    if source == "pptx" && target == "pdf" {
        return crate::pptx::pptx_to_pdf(input, output);
    }
    if source == "epub" && target == "html" {
        return crate::epub::epub_to_html(input, output);
    }
    if source == "rtf" && target == "html" {
        return crate::rtf::rtf_to_html(input, output);
    }
    if source == "doc" && target == "html" {
        return crate::doc::doc_to_html(input, output);
    }
    if source == "doc" && target == "pdf" {
        return crate::doc::doc_to_pdf(input, output);
    }
    if source == "ppt" && target == "html" {
        return crate::ppt::ppt_to_html(input, output);
    }
    if source == "ppt" && target == "pdf" {
        return crate::ppt::ppt_to_pdf(input, output);
    }
    if !pair_ready(&source, &target) {
        let note = PAIRS
            .iter()
            .find(|p| key(p.source) == source && key(p.target) == target)
            .map(|p| p.note)
            .unwrap_or("MISS: no bridge");
        return Err(Error::new(note));
    }
    match (source.as_str(), target.as_str()) {
        ("docx", "txt") => docx::docx_to_text(input, output),
        ("docx", "html") => docx::docx_to_html(input, output),
        ("docx", "pdf") => pipe_html_pdf(input, output, |r, w| docx::docx_to_html(r, w)),
        ("odt", "txt") => docx::odt_to_text(input, output),
        ("odt", "html") => docx::odt_to_html(input, output),
        ("odt", "pdf") => pipe_html_pdf(input, output, |r, w| docx::odt_to_html(r, w)),
        ("md", "html") => {
            let raw = read_string(input)?;
            let html = markdown_to_html(&raw);
            write_str(input, output, &html)
        }
        ("md", "txt") => {
            let raw = read_string(input)?;
            let text = markdown_to_text(&raw);
            write_str(input, output, &text)
        }
        ("md", "pdf") => {
            let raw = read_string(input)?;
            let pdf = html_to_pdf(&markdown_to_html(&raw))?;
            write_bytes(input, output, &pdf)
        }
        ("html", "txt") => {
            let raw = read_string(input)?;
            let text = html_to_text(&raw)?;
            write_str(input, output, &text)
        }
        ("html", "pdf") => {
            let raw = read_string(input)?;
            let pdf = html_to_pdf(&raw)?;
            write_bytes(input, output, &pdf)
        }
        ("txt", "html") => {
            let raw = read_string(input)?;
            let html = text_to_html(&raw);
            write_str(input, output, &html)
        }
        ("txt", "pdf") => {
            let raw = read_string(input)?;
            let pdf = html_to_pdf(&text_to_html(&raw))?;
            write_bytes(input, output, &pdf)
        }
        ("pdf", "txt") => {
            let bytes = read_bytes(input)?;
            write_str_known(bytes.len() as u64, output, &pdf::plain_text(&bytes)?)
        }
        ("pdf", "shrink") => {
            let bytes = read_bytes(input)?;
            let shrunk = pdf::shrink(&bytes)?;
            write_bytes_known(bytes.len() as u64, output, &shrunk)
        }
        ("pdf", "split") | ("pdf", "merge") | ("pdf", "rotate") | ("pdf", "watermark") => Err(Error::new(
            "pdf split, merge, rotate, and watermark are pdf_split, pdf_merge, pdf_rotate, and pdf_watermark",
        )),
        _ => Err(Error::new("MISS: no bridge")),
    }
}

fn pipe_html_pdf<R, W, F>(input: &mut R, output: &mut W, mut to_html: F) -> Result<RunStats>
where
    R: Read + Seek,
    W: Write,
    F: FnMut(&mut R, &mut Vec<u8>) -> Result<RunStats>,
{
    let input_bytes = stream_len(input)?;
    let mut html = Vec::new();
    let stats = to_html(input, &mut html)?;
    let text = String::from_utf8_lossy(&html);
    let pdf = html_to_pdf(&text)?;
    output.write_all(&pdf)?;
    Ok(RunStats {
        scratch_peak: stats.scratch_peak,
        input_bytes,
        output_bytes: pdf.len() as u64,
    })
}

fn read_string<R: Read + Seek>(input: &mut R) -> Result<String> {
    let bytes = read_bytes(input)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn read_bytes<R: Read + Seek>(input: &mut R) -> Result<Vec<u8>> {
    input.seek(std::io::SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    input.read_to_end(&mut buf)?;
    Ok(buf)
}

fn stream_len<R: Seek>(input: &mut R) -> Result<u64> {
    let n = input.seek(std::io::SeekFrom::End(0))?;
    input.seek(std::io::SeekFrom::Start(0))?;
    Ok(n)
}

fn write_str<R: Read + Seek, W: Write>(input: &mut R, output: &mut W, text: &str) -> Result<RunStats> {
    let input_bytes = stream_len(input)?;
    let mut counted = CountWrite::new(output);
    counted.write_all(text.as_bytes())?;
    Ok(RunStats {
        scratch_peak: text.len(),
        input_bytes,
        output_bytes: counted.n,
    })
}

fn write_bytes_known<W: Write>(input_bytes: u64, output: &mut W, bytes: &[u8]) -> Result<RunStats> {
    output.write_all(bytes)?;
    Ok(RunStats {
        scratch_peak: bytes.len(),
        input_bytes,
        output_bytes: bytes.len() as u64,
    })
}

fn write_str_known<W: Write>(input_bytes: u64, output: &mut W, text: &str) -> Result<RunStats> {
    output.write_all(text.as_bytes())?;
    Ok(RunStats {
        scratch_peak: text.len(),
        input_bytes,
        output_bytes: text.len() as u64,
    })
}

fn write_bytes<R: Read + Seek, W: Write>(input: &mut R, output: &mut W, bytes: &[u8]) -> Result<RunStats> {
    let input_bytes = stream_len(input)?;
    output.write_all(bytes)?;
    Ok(RunStats {
        scratch_peak: bytes.len(),
        input_bytes,
        output_bytes: bytes.len() as u64,
    })
}

pub fn pdf_split(input: &[u8]) -> Result<Vec<Vec<u8>>> {
    pdf::split(input)
}

pub fn pdf_merge(inputs: &[&[u8]]) -> Result<Vec<u8>> {
    pdf::merge(inputs)
}

pub fn pdf_rotate(input: &[u8], degrees: i32) -> Result<Vec<u8>> {
    pdf::rotate(input, degrees)
}

pub fn pdf_watermark(input: &[u8], text: &str) -> Result<Vec<u8>> {
    pdf::watermark(input, text)
}

pub fn pdf_page_count(input: &[u8]) -> Result<usize> {
    pdf::page_count(input)
}

pub fn pdf_text(input: &[u8]) -> Result<String> {
    pdf::plain_text(input)
}

pub fn pdf_image_matches(input: &[u8], width: u32, height: u32, rgb: &[u8]) -> Result<bool> {
    pdf::contains_image(input, width, height, rgb)
}

pub fn pdf_shrink(input: &[u8]) -> Result<Vec<u8>> {
    pdf::shrink(input)
}

pub fn pdf_flate_fixture(input: &[u8]) -> Result<Vec<u8>> {
    pdf::flate_fixture(input)
}

fn kind_code(ext: &str) -> u16 {
    match key(ext).as_str() {
        "docx" => 1,
        "md" => 2,
        "html" => 3,
        "txt" => 4,
        "pdf" => 5,
        "odt" => 6,
        "rtf" => 7,
        "epub" => 8,
        "pptx" => 9,
        "split" => 10,
        "merge" => 11,
        _ => 0,
    }
}

/// Encode the converted body as one `EXDOC001` segment and rename it into place.
pub fn save_conversion(dir: &Path, index: u32, source: &str, target: &str, input: &[u8]) -> Result<PathBuf> {
    let body = convert(source, target, input)?;
    let pages = if key(target) == "pdf" {
        (pdf::page_count(&body).unwrap_or(0) as u32).to_le_bytes().to_vec()
    } else {
        Vec::new()
    };
    let meta = Meta {
        source: kind_code(source),
        target: kind_code(target),
        flags: 0,
        input_bytes: input.len() as u64,
        output_bytes: body.len() as u64,
    };
    let bytes = container::encode(&meta, &body, &pages)?;
    container::write_segment(dir, index, &bytes)
}

fn key(ext: &str) -> String {
    let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    match ext.as_str() {
        "markdown" => "md".to_string(),
        "htm" => "html".to_string(),
        _ => ext,
    }
}
