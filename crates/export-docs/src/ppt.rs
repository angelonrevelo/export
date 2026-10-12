//! Legacy PowerPoint 97 to HTML for one generated compound file.
//! The `PowerPoint Document` stream holds one, two, three, four, five, six, seven, eight, nine, ten, or eleven text-bytes shapes.
//! Any other file stays a miss. LibreOffice and pandoc are not called.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::docx::CountWrite;
use crate::error::{Error, Result};
use crate::stats::RunStats;

const MAX_INPUT: u64 = 32 * 1024 * 1024;
const MAX_TEXT: usize = 32 * 1024;
const SECTOR: usize = 512;
const MINI: usize = 64;
const MINI_CUTOFF: u32 = 4096;
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FATSECT: u32 = 0xFFFF_FFFD;
const FREESECT: u32 = 0xFFFF_FFFF;
const NOSTREAM: u32 = 0xFFFF_FFFF;
const SIG: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const STREAM: &str = "PowerPoint Document";

const RT_DOCUMENT: u16 = 0x03E8;
const RT_DOCUMENT_ATOM: u16 = 0x03E9;
const RT_SLIDE_LIST: u16 = 0x0FF0;
const RT_TEXT_HEADER: u16 = 0x0F9F;
const RT_TEXT_BYTES: u16 = 0x0FA8;
const RT_PICTURE: u16 = 0xF01F;
const PICTURE_INSTANCE: u16 = 0x07A8;

pub fn ppt_to_html<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    let (input_bytes, bytes, texts) = load_texts(src)?;
    let html = html_of(&texts);
    for text in &texts {
        if !html.contains(text) {
            return Err(miss(input_bytes, "html does not contain the text"));
        }
    }
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

/// A generated one-shape file, and four text shapes, become a text PDF when the
/// extracted text contains those strings. One text shape plus one 24-bit,
/// 8-bit, or 4-bit DIB picture becomes a text PDF with that picture as an
/// image. Two text shapes plus one 8-bit DIB picture become a text PDF when
/// both strings are in the text and the picture is an image. One text shape
/// plus two 8-bit DIB pictures becomes a text PDF when the string is in the
/// text and both pictures are images. One text shape plus three 8-bit DIB
/// pictures becomes a text PDF when the string is in the text and all three
/// pictures are images. One text shape plus four 8-bit DIB pictures becomes a
/// text PDF when the string is in the text and all four pictures are images.
/// One text shape plus five 8-bit DIB pictures becomes a text PDF when the
/// string is in the text and all five pictures are images. One text shape plus
/// six 8-bit DIB pictures becomes a text PDF when the string is in the text
/// and all six pictures are images. Other bit depths stay a miss. The published
/// 2,560-byte one-shape file stays a miss.
pub fn ppt_to_pdf<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    let (input_bytes, bytes) = load_file(src)?;
    let texts = match parse_ppt(&bytes) {
        Ok(texts) => texts,
        Err(err) => {
            if let Some(stats) = write_picture_pdf(input_bytes, &bytes, out)? {
                return Ok(stats);
            }
            return Err(if err.message.starts_with("MISS:") {
                err
            } else {
                miss(input_bytes, &err.message)
            });
        }
    };
    if !pdf_shape_ready(input_bytes, texts.len()) {
        return Err(miss(input_bytes, "file is not four text shapes"));
    }
    let pdf = shapes_pdf(&texts).map_err(|err| miss(input_bytes, &err.message))?;
    let extracted = crate::pdf::plain_text(&pdf).map_err(|err| miss(input_bytes, &err.message))?;
    if !texts.iter().all(|text| extracted.contains(text)) {
        return Err(miss(input_bytes, "pdf text does not contain the text"));
    }
    let mut counted = CountWrite::new(out);
    counted
        .write_all(&pdf)
        .map_err(|err| miss(input_bytes, &err.to_string()))?;
    let scratch = if bytes.len() > pdf.len() {
        bytes.len()
    } else {
        pdf.len()
    };
    Ok(RunStats {
        scratch_peak: scratch,
        input_bytes,
        output_bytes: counted.n,
    })
}

fn load_texts<R: Read + Seek>(src: &mut R) -> Result<(u64, Vec<u8>, Vec<String>)> {
    let (input_bytes, bytes) = load_file(src)?;
    let texts = parse_ppt(&bytes).map_err(|err| {
        if err.message.starts_with("MISS:") {
            err
        } else {
            miss(input_bytes, &err.message)
        }
    })?;
    Ok((input_bytes, bytes, texts))
}

fn load_file<R: Read + Seek>(src: &mut R) -> Result<(u64, Vec<u8>)> {
    let input_bytes = match src.seek(SeekFrom::End(0)) {
        Ok(n) => n,
        Err(err) => return Err(miss(0, &err.to_string())),
    };
    if input_bytes > MAX_INPUT {
        return Err(miss(input_bytes, "compound document is not readable"));
    }
    if let Err(err) = src.seek(SeekFrom::Start(0)) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    let mut bytes = Vec::new();
    if let Err(err) = src.read_to_end(&mut bytes) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    Ok((input_bytes, bytes))
}

/// The smallest one-shape compound file is the published 2,560-byte miss.
const ONE_SHAPE_PDF_MISS: u64 = 2560;

fn pdf_shape_ready(input_bytes: u64, count: usize) -> bool {
    count == 4 || (count == 1 && input_bytes != ONE_SHAPE_PDF_MISS)
}

fn shapes_pdf(texts: &[String]) -> Result<Vec<u8>> {
    crate::pdf::from_lines(&text_lines(texts))
}

fn text_lines(texts: &[String]) -> Vec<crate::pdf::Line> {
    texts
        .iter()
        .map(|text| crate::pdf::Line {
            spans: vec![crate::pdf::Span {
                text: text.clone(),
                font: 0,
                size: 11.0,
            }],
            leading: 14.0,
        })
        .collect()
}

fn write_picture_pdf<W: Write>(
    input_bytes: u64,
    bytes: &[u8],
    out: &mut W,
) -> Result<Option<RunStats>> {
    let Ok(stream) = read_stream(bytes) else {
        return Ok(None);
    };
    let Some((texts, pictures)) = texts_and_picture(&stream) else {
        return Ok(None);
    };
    let lines = text_lines(&texts);
    let pdf = if pictures.len() == 1 {
        let (width, height, rgb) = &pictures[0];
        crate::pdf::from_lines_with_image(&lines, *width, *height, rgb)
            .map_err(|err| miss(input_bytes, &err.message))?
    } else if pictures.len() == 2 && texts.len() == 1 {
        let (first_width, first_height, first_rgb) = &pictures[0];
        let (second_width, second_height, second_rgb) = &pictures[1];
        crate::pdf::from_lines_with_two_images(
            &lines,
            *first_width,
            *first_height,
            first_rgb,
            *second_width,
            *second_height,
            second_rgb,
        )
        .map_err(|err| miss(input_bytes, &err.message))?
    } else if pictures.len() == 3 && texts.len() == 1 {
        let (w0, h0, rgb0) = &pictures[0];
        let (w1, h1, rgb1) = &pictures[1];
        let (w2, h2, rgb2) = &pictures[2];
        crate::pdf::from_lines_with_three_images(
            &lines,
            [(*w0, *h0, rgb0.as_slice()), (*w1, *h1, rgb1.as_slice()), (*w2, *h2, rgb2.as_slice())],
        )
        .map_err(|err| miss(input_bytes, &err.message))?
    } else if pictures.len() == 4 && texts.len() == 1 {
        let (w0, h0, rgb0) = &pictures[0];
        let (w1, h1, rgb1) = &pictures[1];
        let (w2, h2, rgb2) = &pictures[2];
        let (w3, h3, rgb3) = &pictures[3];
        crate::pdf::from_lines_with_four_images(
            &lines,
            [
                (*w0, *h0, rgb0.as_slice()),
                (*w1, *h1, rgb1.as_slice()),
                (*w2, *h2, rgb2.as_slice()),
                (*w3, *h3, rgb3.as_slice()),
            ],
        )
        .map_err(|err| miss(input_bytes, &err.message))?
    } else if pictures.len() == 5 && texts.len() == 1 {
        let (w0, h0, rgb0) = &pictures[0];
        let (w1, h1, rgb1) = &pictures[1];
        let (w2, h2, rgb2) = &pictures[2];
        let (w3, h3, rgb3) = &pictures[3];
        let (w4, h4, rgb4) = &pictures[4];
        crate::pdf::from_lines_with_five_images(
            &lines,
            [
                (*w0, *h0, rgb0.as_slice()),
                (*w1, *h1, rgb1.as_slice()),
                (*w2, *h2, rgb2.as_slice()),
                (*w3, *h3, rgb3.as_slice()),
                (*w4, *h4, rgb4.as_slice()),
            ],
        )
        .map_err(|err| miss(input_bytes, &err.message))?
    } else if pictures.len() == 6 && texts.len() == 1 {
        let (w0, h0, rgb0) = &pictures[0];
        let (w1, h1, rgb1) = &pictures[1];
        let (w2, h2, rgb2) = &pictures[2];
        let (w3, h3, rgb3) = &pictures[3];
        let (w4, h4, rgb4) = &pictures[4];
        let (w5, h5, rgb5) = &pictures[5];
        crate::pdf::from_lines_with_six_images(
            &lines,
            [
                (*w0, *h0, rgb0.as_slice()),
                (*w1, *h1, rgb1.as_slice()),
                (*w2, *h2, rgb2.as_slice()),
                (*w3, *h3, rgb3.as_slice()),
                (*w4, *h4, rgb4.as_slice()),
                (*w5, *h5, rgb5.as_slice()),
            ],
        )
        .map_err(|err| miss(input_bytes, &err.message))?
    } else {
        return Ok(None);
    };
    let extracted = crate::pdf::plain_text(&pdf).map_err(|err| miss(input_bytes, &err.message))?;
    if !texts.iter().all(|text| extracted.contains(text)) {
        return Err(miss(input_bytes, "pdf text does not contain the text"));
    }
    for (width, height, rgb) in &pictures {
        if !crate::pdf::contains_image(&pdf, *width, *height, rgb)
            .map_err(|err| miss(input_bytes, &err.message))?
        {
            return Err(miss(input_bytes, "pdf has no picture"));
        }
    }
    let mut counted = CountWrite::new(out);
    counted
        .write_all(&pdf)
        .map_err(|err| miss(input_bytes, &err.to_string()))?;
    let mut scratch = bytes.len().max(pdf.len());
    for (_, _, rgb) in &pictures {
        scratch = scratch.max(rgb.len());
    }
    Ok(Some(RunStats {
        scratch_peak: scratch,
        input_bytes,
        output_bytes: counted.n,
    }))
}

type PictureRgb = (u32, u32, Vec<u8>);

fn texts_and_picture(stream: &[u8]) -> Option<(Vec<String>, Vec<PictureRgb>)> {
    let (doc, used) = next_record(stream).ok()?;
    if used != stream.len() || doc.ty != RT_DOCUMENT || doc.ver != 0xF || doc.instance != 0 {
        return None;
    }
    let mut at = 0usize;
    let mut saw_atom = false;
    let mut found = None;
    while at < doc.data.len() {
        let (rec, n) = next_record(&doc.data[at..]).ok()?;
        at += n;
        match rec.ty {
            RT_DOCUMENT_ATOM => {
                if saw_atom || rec.ver != 1 || rec.data.len() != 40 {
                    return None;
                }
                saw_atom = true;
            }
            RT_SLIDE_LIST => {
                if found.is_some() || rec.ver != 0xF {
                    return None;
                }
                found = Some(list_text_picture(rec.data)?);
            }
            _ => return None,
        }
    }
    if at != doc.data.len() || !saw_atom {
        return None;
    }
    found
}

fn list_text_picture(body: &[u8]) -> Option<(Vec<String>, Vec<PictureRgb>)> {
    let mut at = 0usize;
    let mut texts = Vec::new();
    let mut pictures = Vec::new();
    let mut pending = false;
    while at < body.len() {
        let (rec, n) = next_record(&body[at..]).ok()?;
        at += n;
        match rec.ty {
            RT_TEXT_HEADER => {
                if pending
                    || texts.len() == 2
                    || !pictures.is_empty()
                    || rec.ver != 0
                    || rec.data.len() != 4
                {
                    return None;
                }
                pending = true;
            }
            RT_TEXT_BYTES => {
                if !pending || !pictures.is_empty() || rec.ver != 0 {
                    return None;
                }
                if rec.data.is_empty() || !rec.data.iter().copied().all(allowed_text) {
                    return None;
                }
                let mut owned = String::with_capacity(rec.data.len());
                for &b in rec.data {
                    owned.push(char::from(b));
                }
                texts.push(owned);
                pending = false;
            }
            RT_PICTURE => {
                if pending
                    || texts.is_empty()
                    || pictures.len() == 6
                    || rec.ver != 0
                    || rec.instance != PICTURE_INSTANCE
                {
                    return None;
                }
                pictures.push(extract_blip(rec.data)?);
            }
            _ => return None,
        }
    }
    if at != body.len() || pending || texts.is_empty() || pictures.is_empty() {
        return None;
    }
    Some((texts, pictures))
}

fn extract_blip(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if data.len() < 17 || data[16] != 0xFF {
        return None;
    }
    dib_rgb(&data[17..])
}

fn dib_rgb(dib: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if dib.len() < 40 {
        return None;
    }
    let size = u32::from_le_bytes(dib[0..4].try_into().ok()?);
    if size != 40 {
        return None;
    }
    let width = i32::from_le_bytes(dib[4..8].try_into().ok()?);
    let height = i32::from_le_bytes(dib[8..12].try_into().ok()?);
    if width <= 0 || height <= 0 || width > 4096 || height > 4096 {
        return None;
    }
    let planes = u16::from_le_bytes(dib[12..14].try_into().ok()?);
    let bits = u16::from_le_bytes(dib[14..16].try_into().ok()?);
    let compression = u32::from_le_bytes(dib[16..20].try_into().ok()?);
    if planes != 1 || compression != 0 {
        return None;
    }
    if bits == 4 {
        return dib4_rgb(dib, width as usize, height as usize);
    }
    if bits == 8 {
        return dib8_rgb(dib, width as usize, height as usize);
    }
    if bits != 24 {
        return None;
    }
    let width = width as usize;
    let height = height as usize;
    let stride = (width * 3).div_ceil(4) * 4;
    if dib.len() != 40 + stride * height {
        return None;
    }
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        let src_y = height - 1 - y;
        let row = &dib[40 + src_y * stride..40 + src_y * stride + width * 3];
        for x in 0..width {
            let s = x * 3;
            let d = (y * width + x) * 3;
            rgb[d] = row[s + 2];
            rgb[d + 1] = row[s + 1];
            rgb[d + 2] = row[s];
        }
    }
    Some((width as u32, height as u32, rgb))
}

fn dib4_rgb(dib: &[u8], width: usize, height: usize) -> Option<(u32, u32, Vec<u8>)> {
    let colors = u32::from_le_bytes(dib[32..36].try_into().ok()?);
    if colors != 0 && colors != 16 {
        return None;
    }
    let stride = width.div_ceil(2).div_ceil(4) * 4;
    let pixels_at = 40 + 16 * 4;
    if dib.len() != pixels_at + stride * height {
        return None;
    }
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        let src_y = height - 1 - y;
        let row = &dib[pixels_at + src_y * stride..pixels_at + (src_y + 1) * stride];
        for x in 0..width {
            let byte = row[x / 2];
            let index = if x % 2 == 0 { byte >> 4 } else { byte & 0x0F };
            let quad = 40 + index as usize * 4;
            let d = (y * width + x) * 3;
            rgb[d] = dib[quad + 2];
            rgb[d + 1] = dib[quad + 1];
            rgb[d + 2] = dib[quad];
        }
    }
    Some((width as u32, height as u32, rgb))
}

fn dib8_rgb(dib: &[u8], width: usize, height: usize) -> Option<(u32, u32, Vec<u8>)> {
    let colors = u32::from_le_bytes(dib[32..36].try_into().ok()?);
    if colors != 0 && colors != 256 {
        return None;
    }
    let stride = width.div_ceil(4) * 4;
    let pixels_at = 40 + 256 * 4;
    if dib.len() != pixels_at + stride * height {
        return None;
    }
    let mut rgb = vec![0u8; width * height * 3];
    for y in 0..height {
        let src_y = height - 1 - y;
        let row = &dib[pixels_at + src_y * stride..pixels_at + src_y * stride + width];
        for (x, &index) in row.iter().enumerate() {
            let quad = 40 + index as usize * 4;
            let d = (y * width + x) * 3;
            rgb[d] = dib[quad + 2];
            rgb[d + 1] = dib[quad + 1];
            rgb[d + 2] = dib[quad];
        }
    }
    Some((width as u32, height as u32, rgb))
}

fn picture_rgb() -> (u32, u32, Vec<u8>) {
    let width = 80u32;
    let height = 48u32;
    let mut rgb = vec![0u8; (width * height * 3) as usize];
    for y in 0..height {
        let pixel = [32u8, 40 + (y as u8) * 3, 180];
        for x in 0..width {
            let at = ((y * width + x) * 3) as usize;
            rgb[at] = pixel[0];
            rgb[at + 1] = pixel[1];
            rgb[at + 2] = pixel[2];
        }
    }
    (width, height, rgb)
}

fn blip_dib(width: u32, height: u32, rgb: &[u8]) -> Option<Vec<u8>> {
    let dib = encode_dib(width, height, rgb)?;
    let mut out = vec![0u8; 17];
    out[0] = 0xA1;
    out[16] = 0xFF;
    out.extend(dib);
    Some(out)
}

fn encode_dib(width: u32, height: u32, rgb: &[u8]) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return None;
    }
    let pixels = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(3))?;
    if rgb.len() != pixels {
        return None;
    }
    let stride = (width as usize * 3).div_ceil(4) * 4;
    let mut dib = vec![0u8; 40 + stride * height as usize];
    dib[0..4].copy_from_slice(&40u32.to_le_bytes());
    dib[4..8].copy_from_slice(&width.to_le_bytes());
    dib[8..12].copy_from_slice(&height.to_le_bytes());
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&24u16.to_le_bytes());
    for y in 0..height as usize {
        let src_y = height as usize - 1 - y;
        for x in 0..width as usize {
            let s = (src_y * width as usize + x) * 3;
            let d = 40 + y * stride + x * 3;
            dib[d] = rgb[s + 2];
            dib[d + 1] = rgb[s + 1];
            dib[d + 2] = rgb[s];
        }
    }
    Some(dib)
}

fn palette_entry(index: u8) -> [u8; 3] {
    [
        index,
        index.wrapping_mul(3).wrapping_add(20),
        180u8.wrapping_sub(index),
    ]
}

fn encode_dib4(width: u32, height: u32) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || width > 4096 || height > 4096 {
        return None;
    }
    let width = width as usize;
    let height = height as usize;
    let stride = width.div_ceil(2).div_ceil(4) * 4;
    let pixels_at = 40 + 16 * 4;
    let mut dib = vec![0u8; pixels_at + stride * height];
    dib[0..4].copy_from_slice(&40u32.to_le_bytes());
    dib[4..8].copy_from_slice(&(width as u32).to_le_bytes());
    dib[8..12].copy_from_slice(&(height as u32).to_le_bytes());
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&4u16.to_le_bytes());
    dib[20..24].copy_from_slice(&((stride * height) as u32).to_le_bytes());
    for i in 0..16u16 {
        let [r, g, b] = palette_entry(i as u8);
        let at = 40 + i as usize * 4;
        dib[at] = b;
        dib[at + 1] = g;
        dib[at + 2] = r;
    }
    for y in 0..height {
        let file_y = height - 1 - y;
        for x in 0..width {
            let index = (x as u8).wrapping_add(y as u8) & 0x0F;
            let at = pixels_at + file_y * stride + x / 2;
            if x % 2 == 0 {
                dib[at] = index << 4;
            } else {
                dib[at] |= index;
            }
        }
    }
    Some(dib)
}

fn encode_dib8(width: u32, height: u32) -> Option<Vec<u8>> {
    encode_dib8_index(width, height, 0)
}

fn encode_dib8_index(width: u32, height: u32, add: u8) -> Option<Vec<u8>> {
    if width == 0 || height == 0 || width > 4096 || height > 255 {
        return None;
    }
    let width = width as usize;
    let height = height as usize;
    let stride = width.div_ceil(4) * 4;
    let pixels_at = 40 + 256 * 4;
    let mut dib = vec![0u8; pixels_at + stride * height];
    dib[0..4].copy_from_slice(&40u32.to_le_bytes());
    dib[4..8].copy_from_slice(&(width as u32).to_le_bytes());
    dib[8..12].copy_from_slice(&(height as u32).to_le_bytes());
    dib[12..14].copy_from_slice(&1u16.to_le_bytes());
    dib[14..16].copy_from_slice(&8u16.to_le_bytes());
    dib[20..24].copy_from_slice(&((stride * height) as u32).to_le_bytes());
    for i in 0..256u16 {
        let [r, g, b] = palette_entry(i as u8);
        let at = 40 + i as usize * 4;
        dib[at] = b;
        dib[at + 1] = g;
        dib[at + 2] = r;
    }
    for y in 0..height {
        let src_y = height - 1 - y;
        let index = (y as u8).wrapping_add(add);
        for x in 0..width {
            dib[pixels_at + src_y * stride + x] = index;
        }
    }
    Some(dib)
}

fn document_with_picture(texts: &[&[u8]], blip: &[u8]) -> Vec<u8> {
    document_with_pictures(texts, &[blip])
}

fn document_with_pictures(texts: &[&[u8]], blips: &[&[u8]]) -> Vec<u8> {
    let mut atom = vec![0u8; 40];
    put_u32(&mut atom, 0, 5760);
    put_u32(&mut atom, 4, 4320);
    let mut list_body = Vec::new();
    for text in texts {
        list_body.extend(record(0, 0, RT_TEXT_HEADER, &4u32.to_le_bytes()));
        list_body.extend(record(0, 0, RT_TEXT_BYTES, text));
    }
    for blip in blips {
        list_body.extend(record(0, PICTURE_INSTANCE, RT_PICTURE, blip));
    }
    let list = record(0xF, 0, RT_SLIDE_LIST, &list_body);
    let atom_rec = record(1, 0, RT_DOCUMENT_ATOM, &atom);
    let mut body = atom_rec;
    body.extend(list);
    record(0xF, 0, RT_DOCUMENT, &body)
}

/// Compound file whose `PowerPoint Document` stream is one text-bytes shape.
pub fn ppt_text_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[text.as_bytes()]))
}

/// Compound file whose `PowerPoint Document` stream is two text-bytes shapes.
pub fn ppt_two_text_document(first: &str, second: &str) -> Result<Vec<u8>> {
    if !plain_shape(first) || !plain_shape(second) {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[first.as_bytes(), second.as_bytes()]))
}

/// Compound file whose `PowerPoint Document` stream is three text-bytes shapes.
pub fn ppt_three_text_document(first: &str, second: &str, third: &str) -> Result<Vec<u8>> {
    if !plain_shape(first) || !plain_shape(second) || !plain_shape(third) {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
    ]))
}

/// Compound file whose `PowerPoint Document` stream is four text-bytes shapes.
pub fn ppt_four_text_document(
    first: &str,
    second: &str,
    third: &str,
    fourth: &str,
) -> Result<Vec<u8>> {
    if !plain_shape(first) || !plain_shape(second) || !plain_shape(third) || !plain_shape(fourth) {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
        fourth.as_bytes(),
    ]))
}

/// Compound file whose `PowerPoint Document` stream is five text-bytes shapes.
pub fn ppt_five_text_document(
    first: &str,
    second: &str,
    third: &str,
    fourth: &str,
    fifth: &str,
) -> Result<Vec<u8>> {
    if !plain_shape(first)
        || !plain_shape(second)
        || !plain_shape(third)
        || !plain_shape(fourth)
        || !plain_shape(fifth)
    {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
        fourth.as_bytes(),
        fifth.as_bytes(),
    ]))
}

/// Compound file whose `PowerPoint Document` stream is six text-bytes shapes.
pub fn ppt_six_text_document(
    first: &str,
    second: &str,
    third: &str,
    fourth: &str,
    fifth: &str,
    sixth: &str,
) -> Result<Vec<u8>> {
    if !plain_shape(first)
        || !plain_shape(second)
        || !plain_shape(third)
        || !plain_shape(fourth)
        || !plain_shape(fifth)
        || !plain_shape(sixth)
    {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
        fourth.as_bytes(),
        fifth.as_bytes(),
        sixth.as_bytes(),
    ]))
}

/// Compound file whose `PowerPoint Document` stream is seven text-bytes shapes.
pub fn ppt_seven_text_document(
    first: &str,
    second: &str,
    third: &str,
    fourth: &str,
    fifth: &str,
    sixth: &str,
    seventh: &str,
) -> Result<Vec<u8>> {
    if !plain_shape(first)
        || !plain_shape(second)
        || !plain_shape(third)
        || !plain_shape(fourth)
        || !plain_shape(fifth)
        || !plain_shape(sixth)
        || !plain_shape(seventh)
    {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
        fourth.as_bytes(),
        fifth.as_bytes(),
        sixth.as_bytes(),
        seventh.as_bytes(),
    ]))
}

/// Compound file whose `PowerPoint Document` stream is eight text-bytes shapes.
#[allow(clippy::too_many_arguments)]
pub fn ppt_eight_text_document(
    first: &str,
    second: &str,
    third: &str,
    fourth: &str,
    fifth: &str,
    sixth: &str,
    seventh: &str,
    eighth: &str,
) -> Result<Vec<u8>> {
    if !plain_shape(first)
        || !plain_shape(second)
        || !plain_shape(third)
        || !plain_shape(fourth)
        || !plain_shape(fifth)
        || !plain_shape(sixth)
        || !plain_shape(seventh)
        || !plain_shape(eighth)
    {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
        fourth.as_bytes(),
        fifth.as_bytes(),
        sixth.as_bytes(),
        seventh.as_bytes(),
        eighth.as_bytes(),
    ]))
}

/// Compound file whose `PowerPoint Document` stream is nine text-bytes shapes.
#[allow(clippy::too_many_arguments)]
pub fn ppt_nine_text_document(
    first: &str,
    second: &str,
    third: &str,
    fourth: &str,
    fifth: &str,
    sixth: &str,
    seventh: &str,
    eighth: &str,
    ninth: &str,
) -> Result<Vec<u8>> {
    if !plain_shape(first)
        || !plain_shape(second)
        || !plain_shape(third)
        || !plain_shape(fourth)
        || !plain_shape(fifth)
        || !plain_shape(sixth)
        || !plain_shape(seventh)
        || !plain_shape(eighth)
        || !plain_shape(ninth)
    {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
        fourth.as_bytes(),
        fifth.as_bytes(),
        sixth.as_bytes(),
        seventh.as_bytes(),
        eighth.as_bytes(),
        ninth.as_bytes(),
    ]))
}

/// Compound file whose `PowerPoint Document` stream is ten text-bytes shapes.
#[allow(clippy::too_many_arguments)]
pub fn ppt_ten_text_document(
    first: &str,
    second: &str,
    third: &str,
    fourth: &str,
    fifth: &str,
    sixth: &str,
    seventh: &str,
    eighth: &str,
    ninth: &str,
    tenth: &str,
) -> Result<Vec<u8>> {
    if !plain_shape(first)
        || !plain_shape(second)
        || !plain_shape(third)
        || !plain_shape(fourth)
        || !plain_shape(fifth)
        || !plain_shape(sixth)
        || !plain_shape(seventh)
        || !plain_shape(eighth)
        || !plain_shape(ninth)
        || !plain_shape(tenth)
    {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
        fourth.as_bytes(),
        fifth.as_bytes(),
        sixth.as_bytes(),
        seventh.as_bytes(),
        eighth.as_bytes(),
        ninth.as_bytes(),
        tenth.as_bytes(),
    ]))
}

/// Compound file whose `PowerPoint Document` stream is eleven text-bytes shapes.
#[allow(clippy::too_many_arguments)]
pub fn ppt_eleven_text_document(
    first: &str,
    second: &str,
    third: &str,
    fourth: &str,
    fifth: &str,
    sixth: &str,
    seventh: &str,
    eighth: &str,
    ninth: &str,
    tenth: &str,
    eleventh: &str,
) -> Result<Vec<u8>> {
    if !plain_shape(first)
        || !plain_shape(second)
        || !plain_shape(third)
        || !plain_shape(fourth)
        || !plain_shape(fifth)
        || !plain_shape(sixth)
        || !plain_shape(seventh)
        || !plain_shape(eighth)
        || !plain_shape(ninth)
        || !plain_shape(tenth)
        || !plain_shape(eleventh)
    {
        return Err(Error::new("text shape is not plain text"));
    }
    pack_cfb(&document_stream(&[
        first.as_bytes(),
        second.as_bytes(),
        third.as_bytes(),
        fourth.as_bytes(),
        fifth.as_bytes(),
        sixth.as_bytes(),
        seventh.as_bytes(),
        eighth.as_bytes(),
        ninth.as_bytes(),
        tenth.as_bytes(),
        eleventh.as_bytes(),
    ]))
}

/// Compound file with one text shape and one 24-bit DIB picture.
pub fn ppt_text_picture_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    let (width, height, rgb) = picture_rgb();
    let blip = blip_dib(width, height, &rgb).ok_or_else(|| Error::new("picture is not a dib"))?;
    pack_cfb(&document_with_picture(&[text.as_bytes()], &blip))
}

/// Compound file with one text shape and one 4-bit DIB picture.
pub fn ppt_text_nibble_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    let dib = encode_dib4(80, 160).ok_or_else(|| Error::new("picture is not a dib"))?;
    let mut blip = vec![0u8; 17];
    blip[0] = 0xA1;
    blip[16] = 0xFF;
    blip.extend(dib);
    pack_cfb(&document_with_picture(&[text.as_bytes()], &blip))
}

/// Compound file with one text shape and one 8-bit DIB picture.
pub fn ppt_text_indexed_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    let dib = encode_dib8(80, 64).ok_or_else(|| Error::new("picture is not a dib"))?;
    let mut blip = vec![0u8; 17];
    blip[0] = 0xA1;
    blip[16] = 0xFF;
    blip.extend(dib);
    pack_cfb(&document_with_picture(&[text.as_bytes()], &blip))
}

/// Compound file with two text shapes and one 8-bit DIB picture.
pub fn ppt_two_text_indexed_document(first: &str, second: &str) -> Result<Vec<u8>> {
    if !plain_shape(first) || !plain_shape(second) {
        return Err(Error::new("text shape is not plain text"));
    }
    let dib = encode_dib8(80, 64).ok_or_else(|| Error::new("picture is not a dib"))?;
    let mut blip = vec![0u8; 17];
    blip[0] = 0xA1;
    blip[16] = 0xFF;
    blip.extend(dib);
    pack_cfb(&document_with_picture(
        &[first.as_bytes(), second.as_bytes()],
        &blip,
    ))
}

/// Compound file with one text shape and two 8-bit DIB pictures.
pub fn ppt_text_two_indexed_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    let first = encode_dib8_index(80, 64, 0).ok_or_else(|| Error::new("picture is not a dib"))?;
    let second = encode_dib8_index(80, 64, 1).ok_or_else(|| Error::new("picture is not a dib"))?;
    let mut first_blip = vec![0u8; 17];
    first_blip[0] = 0xA1;
    first_blip[16] = 0xFF;
    first_blip.extend(first);
    let mut second_blip = vec![0u8; 17];
    second_blip[0] = 0xA1;
    second_blip[16] = 0xFF;
    second_blip.extend(second);
    pack_cfb(&document_with_pictures(
        &[text.as_bytes()],
        &[&first_blip, &second_blip],
    ))
}

/// Compound file with one text shape and three 8-bit DIB pictures.
pub fn ppt_text_three_indexed_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    let mut blips = Vec::with_capacity(3);
    for add in [0u8, 1, 2] {
        let dib = encode_dib8_index(80, 64, add).ok_or_else(|| Error::new("picture is not a dib"))?;
        let mut blip = vec![0u8; 17];
        blip[0] = 0xA1;
        blip[16] = 0xFF;
        blip.extend(dib);
        blips.push(blip);
    }
    pack_cfb(&document_with_pictures(
        &[text.as_bytes()],
        &[&blips[0], &blips[1], &blips[2]],
    ))
}

/// Compound file with one text shape and four 8-bit DIB pictures.
pub fn ppt_text_four_indexed_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    let mut blips = Vec::with_capacity(4);
    for add in [0u8, 1, 2, 3] {
        let dib = encode_dib8_index(80, 64, add).ok_or_else(|| Error::new("picture is not a dib"))?;
        let mut blip = vec![0u8; 17];
        blip[0] = 0xA1;
        blip[16] = 0xFF;
        blip.extend(dib);
        blips.push(blip);
    }
    pack_cfb(&document_with_pictures(
        &[text.as_bytes()],
        &[&blips[0], &blips[1], &blips[2], &blips[3]],
    ))
}

/// Compound file with one text shape and five 8-bit DIB pictures.
pub fn ppt_text_five_indexed_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    let mut blips = Vec::with_capacity(5);
    for add in [0u8, 1, 2, 3, 4] {
        let dib = encode_dib8_index(80, 64, add).ok_or_else(|| Error::new("picture is not a dib"))?;
        let mut blip = vec![0u8; 17];
        blip[0] = 0xA1;
        blip[16] = 0xFF;
        blip.extend(dib);
        blips.push(blip);
    }
    pack_cfb(&document_with_pictures(
        &[text.as_bytes()],
        &[&blips[0], &blips[1], &blips[2], &blips[3], &blips[4]],
    ))
}

/// Compound file with one text shape and six 8-bit DIB pictures.
pub fn ppt_text_six_indexed_document(text: &str) -> Result<Vec<u8>> {
    if !plain_shape(text) {
        return Err(Error::new("text shape is not plain text"));
    }
    let mut blips = Vec::with_capacity(6);
    for add in [0u8, 1, 2, 3, 4, 5] {
        let dib = encode_dib8_index(80, 64, add).ok_or_else(|| Error::new("picture is not a dib"))?;
        let mut blip = vec![0u8; 17];
        blip[0] = 0xA1;
        blip[16] = 0xFF;
        blip.extend(dib);
        blips.push(blip);
    }
    pack_cfb(&document_with_pictures(
        &[text.as_bytes()],
        &[
            &blips[0],
            &blips[1],
            &blips[2],
            &blips[3],
            &blips[4],
            &blips[5],
        ],
    ))
}

fn plain_shape(text: &str) -> bool {
    !text.is_empty() && text.len() <= MAX_TEXT && text.bytes().all(allowed_text)
}

fn miss(input_bytes: u64, why: &str) -> Error {
    Error::new(format!("MISS: ppt {why} (input_b={input_bytes})"))
}

fn unreadable() -> Error {
    Error::new("compound document is not readable")
}

fn shape() -> Error {
    Error::new("file is not one, two, three, or four text shapes")
}

fn allowed_text(b: u8) -> bool {
    (0x20..=0x7E).contains(&b) && !matches!(b, b'&' | b'<' | b'>' | b'"')
}

fn html_of(texts: &[String]) -> String {
    let mut out = String::from("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body>");
    for text in texts {
        out.push_str("<p>");
        out.push_str(text);
        out.push_str("</p>");
    }
    out.push_str("</body></html>");
    out
}

fn document_stream(texts: &[&[u8]]) -> Vec<u8> {
    let mut atom = vec![0u8; 40];
    put_u32(&mut atom, 0, 5760);
    put_u32(&mut atom, 4, 4320);
    let mut list_body = Vec::new();
    for text in texts {
        list_body.extend(record(0, 0, RT_TEXT_HEADER, &4u32.to_le_bytes()));
        list_body.extend(record(0, 0, RT_TEXT_BYTES, text));
    }
    let list = record(0xF, 0, RT_SLIDE_LIST, &list_body);
    let atom_rec = record(1, 0, RT_DOCUMENT_ATOM, &atom);
    let mut body = atom_rec;
    body.extend(list);
    record(0xF, 0, RT_DOCUMENT, &body)
}

fn record(ver: u8, instance: u16, ty: u16, data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + data.len());
    let bits = u16::from(ver) | (instance << 4);
    out.extend_from_slice(&bits.to_le_bytes());
    out.extend_from_slice(&ty.to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(data);
    out
}

fn parse_ppt(file: &[u8]) -> Result<Vec<String>> {
    let stream = read_stream(file)?;
    text_shape(&stream)
}

fn text_shape(stream: &[u8]) -> Result<Vec<String>> {
    let (doc, used) = next_record(stream)?;
    if used != stream.len() || doc.ty != RT_DOCUMENT || doc.ver != 0xF || doc.instance != 0 {
        return Err(shape());
    }
    let mut at = 0usize;
    let mut saw_atom = false;
    let mut texts = None;
    while at < doc.data.len() {
        let (rec, n) = next_record(&doc.data[at..])?;
        at += n;
        match rec.ty {
            RT_DOCUMENT_ATOM => {
                if saw_atom || rec.ver != 1 || rec.data.len() != 40 {
                    return Err(shape());
                }
                saw_atom = true;
            }
            RT_SLIDE_LIST => {
                if texts.is_some() || rec.ver != 0xF {
                    return Err(shape());
                }
                texts = Some(list_text(rec.data)?);
            }
            _ => return Err(shape()),
        }
    }
    if at != doc.data.len() || !saw_atom {
        return Err(shape());
    }
    texts.ok_or_else(shape)
}

fn list_text(body: &[u8]) -> Result<Vec<String>> {
    let mut at = 0usize;
    let mut pending_header = false;
    let mut texts = Vec::new();
    while at < body.len() {
        let (rec, n) = next_record(&body[at..])?;
        at += n;
        match rec.ty {
            RT_TEXT_HEADER => {
                if pending_header || texts.len() >= 11 || rec.ver != 0 || rec.data.len() != 4 {
                    return Err(shape());
                }
                pending_header = true;
            }
            RT_TEXT_BYTES => {
                if !pending_header || rec.ver != 0 {
                    return Err(shape());
                }
                if rec.data.is_empty() || !rec.data.iter().copied().all(allowed_text) {
                    return Err(shape());
                }
                let mut owned = String::with_capacity(rec.data.len());
                for &b in rec.data {
                    owned.push(char::from(b));
                }
                texts.push(owned);
                pending_header = false;
            }
            _ => return Err(shape()),
        }
    }
    if at != body.len() || pending_header || texts.is_empty() {
        return Err(shape());
    }
    Ok(texts)
}

struct Rec<'a> {
    ver: u8,
    instance: u16,
    ty: u16,
    data: &'a [u8],
}

fn next_record(buf: &[u8]) -> Result<(Rec<'_>, usize)> {
    if buf.len() < 8 {
        return Err(shape());
    }
    let bits = u16::from_le_bytes([buf[0], buf[1]]);
    let len = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    let end = 8usize
        .checked_add(len)
        .ok_or_else(shape)?;
    if end > buf.len() {
        return Err(shape());
    }
    Ok((
        Rec {
            ver: (bits & 0xF) as u8,
            instance: bits >> 4,
            ty: u16::from_le_bytes([buf[2], buf[3]]),
            data: &buf[8..end],
        },
        end,
    ))
}

fn pack_cfb(stream: &[u8]) -> Result<Vec<u8>> {
    if stream.len() < MINI_CUTOFF as usize {
        pack_mini(stream)
    } else {
        pack_regular(stream)
    }
}

fn pack_regular(stream: &[u8]) -> Result<Vec<u8>> {
    let nsec = stream.len().div_ceil(SECTOR);
    if nsec + 2 > 128 {
        return Err(Error::new("text shape is too large"));
    }
    let mut fat_next = vec![(0u32, FATSECT), (1, ENDOFCHAIN)];
    for i in 0..nsec {
        let sid = 2 + i as u32;
        let next = if i + 1 == nsec { ENDOFCHAIN } else { sid + 1 };
        fat_next.push((sid, next));
    }
    let mut file = Vec::new();
    file.extend_from_slice(&cfb_header(ENDOFCHAIN, 0, 0));
    file.extend_from_slice(&fat_sector(&fat_next));
    file.extend_from_slice(&dir_sector(ENDOFCHAIN, 0, 2, stream.len() as u32));
    file.extend_from_slice(stream);
    file.resize(file.len() + nsec * SECTOR - stream.len(), 0);
    Ok(file)
}

fn pack_mini(stream: &[u8]) -> Result<Vec<u8>> {
    let mini_count = stream.len().div_ceil(MINI);
    let mini_bytes = mini_count * MINI;
    let data_secs = mini_bytes.div_ceil(SECTOR);
    if data_secs + 3 > 128 {
        return Err(Error::new("text shape is too large"));
    }
    let mut fat_next = vec![(0u32, FATSECT), (1, ENDOFCHAIN), (2, ENDOFCHAIN)];
    for i in 0..data_secs {
        let sid = 3 + i as u32;
        let next = if i + 1 == data_secs {
            ENDOFCHAIN
        } else {
            sid + 1
        };
        fat_next.push((sid, next));
    }
    let mut mini_fat = [0xFFu8; SECTOR];
    for i in 0..mini_count {
        let next = if i + 1 == mini_count {
            ENDOFCHAIN
        } else {
            (i as u32) + 1
        };
        put_u32(&mut mini_fat, i * 4, next);
    }
    let mut file = Vec::new();
    file.extend_from_slice(&cfb_header(2, 1, 0));
    file.extend_from_slice(&fat_sector(&fat_next));
    file.extend_from_slice(&dir_sector(3, mini_bytes as u32, 0, stream.len() as u32));
    file.extend_from_slice(&mini_fat);
    file.extend_from_slice(stream);
    file.resize(file.len() + data_secs * SECTOR - stream.len(), 0);
    Ok(file)
}

fn cfb_header(mini_fat_start: u32, csect_mini: u32, fat_sid: u32) -> [u8; SECTOR] {
    let mut h = [0u8; SECTOR];
    h[..8].copy_from_slice(&SIG);
    put_u16(&mut h, 0x18, 0x003E);
    put_u16(&mut h, 0x1A, 3);
    put_u16(&mut h, 0x1C, 0xFFFE);
    put_u16(&mut h, 0x1E, 9);
    put_u16(&mut h, 0x20, 6);
    put_u32(&mut h, 0x2C, 1);
    put_u32(&mut h, 0x30, 1);
    put_u32(&mut h, 0x38, MINI_CUTOFF);
    put_u32(&mut h, 0x3C, mini_fat_start);
    put_u32(&mut h, 0x40, csect_mini);
    put_u32(&mut h, 0x44, ENDOFCHAIN);
    put_u32(&mut h, 0x4C, fat_sid);
    for i in 1..109 {
        put_u32(&mut h, 0x4C + i * 4, FREESECT);
    }
    h
}

fn fat_sector(next: &[(u32, u32)]) -> [u8; SECTOR] {
    let mut fat = [0xFFu8; SECTOR];
    for (sid, value) in next {
        put_u32(&mut fat, *sid as usize * 4, *value);
    }
    fat
}

fn dir_sector(root_start: u32, root_size: u32, stream_start: u32, stream_size: u32) -> [u8; SECTOR] {
    let mut dir = [0u8; SECTOR];
    let root = dir_entry("Root Entry", 5, 1, root_start, root_size);
    let stream = dir_entry(STREAM, 2, NOSTREAM, stream_start, stream_size);
    dir[..128].copy_from_slice(&root);
    dir[128..256].copy_from_slice(&stream);
    dir
}

fn dir_entry(name: &str, kind: u8, child: u32, start: u32, size: u32) -> [u8; 128] {
    let mut e = [0u8; 128];
    let mut at = 0;
    for unit in name.encode_utf16() {
        put_u16(&mut e, at, unit);
        at += 2;
    }
    put_u16(&mut e, 0x40, (at + 2) as u16);
    e[0x42] = kind;
    e[0x43] = 1;
    put_u32(&mut e, 0x44, NOSTREAM);
    put_u32(&mut e, 0x48, NOSTREAM);
    put_u32(&mut e, 0x4C, child);
    put_u32(&mut e, 0x74, start);
    put_u32(&mut e, 0x78, size);
    e
}

fn put_u16(buf: &mut [u8], at: usize, v: u16) {
    buf[at..at + 2].copy_from_slice(&v.to_le_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, v: u32) {
    buf[at..at + 4].copy_from_slice(&v.to_le_bytes());
}

fn get_u16(buf: &[u8], at: usize) -> Result<u16> {
    if at + 2 > buf.len() {
        return Err(unreadable());
    }
    Ok(u16::from_le_bytes([buf[at], buf[at + 1]]))
}

fn get_u32(buf: &[u8], at: usize) -> Result<u32> {
    if at + 4 > buf.len() {
        return Err(unreadable());
    }
    Ok(u32::from_le_bytes([
        buf[at],
        buf[at + 1],
        buf[at + 2],
        buf[at + 3],
    ]))
}

struct StreamLoc {
    start: u32,
    size: u32,
}

fn read_stream(file: &[u8]) -> Result<Vec<u8>> {
    if file.len() < SECTOR || file[..8] != SIG {
        return Err(unreadable());
    }
    let major = get_u16(file, 0x1A)?;
    let order = get_u16(file, 0x1C)?;
    let sec_shift = get_u16(file, 0x1E)?;
    let mini_shift = get_u16(file, 0x20)?;
    if major != 3 || order != 0xFFFE || sec_shift != 9 || mini_shift != 6 {
        return Err(unreadable());
    }
    let csect_fat = get_u32(file, 0x2C)?;
    let dir_start = get_u32(file, 0x30)?;
    let cutoff = get_u32(file, 0x38)?;
    let mini_fat_start = get_u32(file, 0x3C)?;
    let csect_mini = get_u32(file, 0x40)?;
    let csect_dif = get_u32(file, 0x48)?;
    if cutoff != MINI_CUTOFF || csect_dif != 0 || csect_fat == 0 || csect_fat > 109 || csect_mini > 8
    {
        return Err(unreadable());
    }
    let fat = load_fat(file, csect_fat)?;
    let dir = read_chain(file, &fat, dir_start, None)?;
    if dir.len() % 128 != 0 {
        return Err(unreadable());
    }
    let (root, stream) = pick_streams(&dir)?;
    if stream.size == 0 || stream.size as u64 > MAX_INPUT {
        return Err(unreadable());
    }
    if stream.size < MINI_CUTOFF {
        if csect_mini == 0 || root.size == 0 {
            return Err(unreadable());
        }
        let mini_fat_bytes = read_chain(
            file,
            &fat,
            mini_fat_start,
            Some(csect_mini as usize * SECTOR),
        )?;
        let mini_fat = words_of(&mini_fat_bytes)?;
        let mini_stream = read_chain(file, &fat, root.start, Some(root.size as usize))?;
        read_mini(&mini_fat, &mini_stream, stream.start, stream.size as usize)
    } else {
        read_chain(file, &fat, stream.start, Some(stream.size as usize))
    }
}

fn load_fat(file: &[u8], csect_fat: u32) -> Result<Vec<u32>> {
    let mut fat = Vec::new();
    for i in 0..csect_fat as usize {
        let sid = get_u32(file, 0x4C + i * 4)?;
        if sid >= 0xFFFF_FFFA {
            return Err(unreadable());
        }
        let sector = sector(file, sid)?;
        fat.extend(words_of(sector)?);
    }
    Ok(fat)
}

fn words_of(bytes: &[u8]) -> Result<Vec<u32>> {
    if bytes.len() % 4 != 0 {
        return Err(unreadable());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks(4) {
        out.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

fn pick_streams(dir: &[u8]) -> Result<(StreamLoc, StreamLoc)> {
    let mut root = None;
    let mut stream = None;
    for entry in dir.chunks(128) {
        let kind = entry[0x42];
        if kind == 0 {
            continue;
        }
        if entry[0x7C..0x80] != [0, 0, 0, 0] {
            return Err(unreadable());
        }
        let loc = StreamLoc {
            start: get_u32(entry, 0x74)?,
            size: get_u32(entry, 0x78)?,
        };
        match kind {
            5 => {
                if root.is_some() {
                    return Err(unreadable());
                }
                root = Some(loc);
            }
            2 => {
                let name = entry_name(entry)?;
                if name != STREAM || stream.is_some() {
                    return Err(Error::new(
                        "compound document has no PowerPoint Document stream",
                    ));
                }
                stream = Some(loc);
            }
            _ => return Err(unreadable()),
        }
    }
    match (root, stream) {
        (Some(root), Some(stream)) => Ok((root, stream)),
        (Some(_), None) => Err(Error::new(
            "compound document has no PowerPoint Document stream",
        )),
        _ => Err(unreadable()),
    }
}

fn entry_name(entry: &[u8]) -> Result<String> {
    let n = get_u16(entry, 0x40)? as usize;
    if !(2..=64).contains(&n) || n % 2 != 0 {
        return Err(unreadable());
    }
    let chars = n / 2 - 1;
    let mut name = String::new();
    for i in 0..chars {
        let unit = get_u16(entry, i * 2)?;
        if unit > 0x7F {
            return Err(unreadable());
        }
        name.push(char::from(unit as u8));
    }
    Ok(name)
}

fn read_chain(file: &[u8], fat: &[u32], start: u32, size: Option<usize>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut sid = start;
    let mut guard = 0usize;
    loop {
        if let Some(size) = size {
            if out.len() >= size {
                out.truncate(size);
                return Ok(out);
            }
        }
        if sid == ENDOFCHAIN {
            break;
        }
        if sid >= 0xFFFF_FFFA || sid as usize >= fat.len() || guard > fat.len() {
            return Err(unreadable());
        }
        guard += 1;
        let chunk = sector(file, sid)?;
        out.extend_from_slice(chunk);
        sid = fat[sid as usize];
    }
    if let Some(size) = size {
        if out.len() < size {
            return Err(unreadable());
        }
        out.truncate(size);
    }
    Ok(out)
}

fn read_mini(mini_fat: &[u32], mini_stream: &[u8], start: u32, size: usize) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(size);
    let mut sid = start;
    let mut guard = 0usize;
    while out.len() < size {
        if sid >= 0xFFFF_FFFA || sid as usize >= mini_fat.len() || guard > mini_fat.len() {
            return Err(unreadable());
        }
        guard += 1;
        let off = sid as usize * MINI;
        if off >= mini_stream.len() {
            return Err(unreadable());
        }
        let end = (off + MINI).min(mini_stream.len());
        let chunk = &mini_stream[off..end];
        let need = size - out.len();
        if chunk.len() >= need {
            out.extend_from_slice(&chunk[..need]);
        } else {
            out.extend_from_slice(chunk);
        }
        sid = mini_fat[sid as usize];
    }
    Ok(out)
}

fn sector(file: &[u8], sid: u32) -> Result<&[u8]> {
    let off = (sid as usize)
        .checked_add(1)
        .and_then(|n| n.checked_mul(SECTOR))
        .ok_or_else(unreadable)?;
    if off + SECTOR > file.len() {
        return Err(unreadable());
    }
    Ok(&file[off..off + SECTOR])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn html(bytes: &[u8]) -> Result<String> {
        let mut out = Vec::new();
        ppt_to_html(&mut Cursor::new(bytes), &mut out)?;
        Ok(String::from_utf8(out).unwrap())
    }

    #[test]
    fn one_text_shape_becomes_html() {
        let text = "Generated legacy PowerPoint text shape.";
        let file = ppt_text_document(text).unwrap();
        assert_eq!(&file[..8], &SIG);
        let got = html(&file).unwrap();
        assert!(got.contains(text), "{got}");
        assert!(got.contains("<p>Generated legacy PowerPoint text shape.</p>"), "{got}");
    }

    #[test]
    fn one_shape_stays_the_published_size() {
        let mut text = String::new();
        for i in 0..60 {
            text.push_str(&format!(
                "Sentence {i} of the generated PowerPoint text shape. "
            ));
        }
        let file = ppt_text_document(&text).unwrap();
        assert_eq!(file.len(), 5632);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 3193);
        assert!(got.contains(&text), "{got}");
    }

    #[test]
    fn one_shape_becomes_pdf_with_the_text() {
        let mut text = String::new();
        for i in 0..60 {
            text.push_str(&format!(
                "Sentence {i} of the generated PowerPoint text shape. "
            ));
        }
        let file = ppt_text_document(&text).unwrap();
        assert_eq!(file.len(), 5632);
        assert_ne!(file.len() as u64, ONE_SHAPE_PDF_MISS);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 3981);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
    }

    #[test]
    fn one_text_and_picture_become_pdf() {
        let text = picture_slide_text();
        let file = ppt_text_picture_document(&text).unwrap();
        assert_eq!(file.len(), 14336);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 1991);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
        let (width, height, rgb) = picture_rgb();
        assert!(crate::pdf::contains_image(&pdf, width, height, &rgb).unwrap());
        assert!(pdf.windows(15).any(|w| w == b"/Subtype /Image"));
    }

    #[test]
    fn one_text_and_eight_bit_picture_become_pdf() {
        let text = indexed_slide_text();
        let file = ppt_text_indexed_document(&text).unwrap();
        assert_eq!(file.len(), 8704);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 2199);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
        let (width, height, rgb) = expected_indexed_rgb();
        assert!(crate::pdf::contains_image(&pdf, width, height, &rgb).unwrap());
        assert!(pdf.windows(15).any(|w| w == b"/Subtype /Image"));
        let twenty_four = ppt_text_picture_document(&picture_slide_text()).unwrap();
        let twenty_four_pdf = crate::convert::convert("ppt", "pdf", &twenty_four).unwrap();
        assert_eq!(twenty_four.len(), 14336);
        assert_eq!(twenty_four_pdf.len(), 1991);
    }

    #[test]
    fn one_text_and_four_bit_picture_become_pdf() {
        let text = nibble_slide_text();
        let file = ppt_text_nibble_document(&text).unwrap();
        assert_eq!(file.len(), 9216);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 2469);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
        let (width, height, rgb) = expected_nibble_rgb();
        assert!(crate::pdf::contains_image(&pdf, width, height, &rgb).unwrap());
        assert!(pdf.windows(15).any(|w| w == b"/Subtype /Image"));
        assert!(pdf.windows(9).any(|w| w == b"/Width 80"));
        assert!(pdf.windows(11).any(|w| w == b"/Height 160"));
        let indexed = ppt_text_indexed_document(&indexed_slide_text()).unwrap();
        let indexed_pdf = crate::convert::convert("ppt", "pdf", &indexed).unwrap();
        assert_eq!(indexed.len(), 8704);
        assert_eq!(indexed_pdf.len(), 2199);
        let twenty_four = ppt_text_picture_document(&picture_slide_text()).unwrap();
        let twenty_four_pdf = crate::convert::convert("ppt", "pdf", &twenty_four).unwrap();
        assert_eq!(twenty_four.len(), 14336);
        assert_eq!(twenty_four_pdf.len(), 1991);
    }

    #[test]
    fn four_bit_picture_stays_a_miss() {
        let mut dib = vec![0u8; 40];
        dib[0..4].copy_from_slice(&40u32.to_le_bytes());
        dib[4..8].copy_from_slice(&8u32.to_le_bytes());
        dib[8..12].copy_from_slice(&8u32.to_le_bytes());
        dib[12..14].copy_from_slice(&1u16.to_le_bytes());
        dib[14..16].copy_from_slice(&4u16.to_le_bytes());
        let mut blip = vec![0u8; 17];
        blip[16] = 0xFF;
        blip.extend(dib);
        let stream = document_with_picture(&[b"Four bit picture stays a miss."], &blip);
        let file = pack_cfb(&stream).unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("not one, two, three, or four text shapes"),
            "{err}"
        );
    }

    #[test]
    fn broken_picture_stays_the_old_miss() {
        let stream = document_with_picture(&[b"Generated legacy PowerPoint picture."], &[0u8; 8]);
        let file = pack_cfb(&stream).unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("not one, two, three, or four text shapes"),
            "{err}"
        );
    }

    fn picture_slide_text() -> String {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!(
                "Picture shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        text
    }

    fn expected_indexed_rgb() -> (u32, u32, Vec<u8>) {
        let width = 80u32;
        let height = 64u32;
        let mut rgb = vec![0u8; (width * height * 3) as usize];
        for y in 0..height as usize {
            let [r, g, b] = palette_entry(y as u8);
            for x in 0..width as usize {
                let d = (y * width as usize + x) * 3;
                rgb[d] = r;
                rgb[d + 1] = g;
                rgb[d + 2] = b;
            }
        }
        (width, height, rgb)
    }

    fn expected_nibble_rgb() -> (u32, u32, Vec<u8>) {
        let width = 80u32;
        let height = 160u32;
        let mut rgb = vec![0u8; (width * height * 3) as usize];
        for y in 0..height as usize {
            for x in 0..width as usize {
                let index = (x as u8).wrapping_add(y as u8) & 0x0F;
                let [r, g, b] = palette_entry(index);
                let d = (y * width as usize + x) * 3;
                rgb[d] = r;
                rgb[d + 1] = g;
                rgb[d + 2] = b;
            }
        }
        (width, height, rgb)
    }

    fn nibble_slide_text() -> String {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!(
                "Nibble picture sentence {i} of the generated PowerPoint file. "
            ));
        }
        text
    }

    fn indexed_slide_text() -> String {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!(
                "Indexed picture sentence {i} of the generated PowerPoint file. "
            ));
        }
        text
    }

    fn two_indexed_slide_text() -> (String, String) {
        let mut first = String::new();
        let mut second = String::new();
        for i in 0..12 {
            first.push_str(&format!(
                "First indexed shape sentence {i} of the generated PowerPoint file. "
            ));
            second.push_str(&format!(
                "Second indexed shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (first, second)
    }

    #[test]
    fn two_text_and_eight_bit_picture_become_pdf() {
        let (first, second) = two_indexed_slide_text();
        let file = ppt_two_text_indexed_document(&first, &second).unwrap();
        assert_eq!(file.len(), 9728);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 3085);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&first), "{got}");
        assert!(got.contains(&second), "{got}");
        assert!(got.find(&first).unwrap() < got.find(&second).unwrap());
        let (width, height, rgb) = expected_indexed_rgb();
        assert!(crate::pdf::contains_image(&pdf, width, height, &rgb).unwrap());
        assert!(pdf.windows(15).any(|w| w == b"/Subtype /Image"));
        assert!(pdf.windows(9).any(|w| w == b"/Width 80"));
        assert!(pdf.windows(10).any(|w| w == b"/Height 64"));
        let indexed = ppt_text_indexed_document(&indexed_slide_text()).unwrap();
        let indexed_pdf = crate::convert::convert("ppt", "pdf", &indexed).unwrap();
        assert_eq!(indexed.len(), 8704);
        assert_eq!(indexed_pdf.len(), 2199);
        let twenty_four = ppt_text_picture_document(&picture_slide_text()).unwrap();
        let twenty_four_pdf = crate::convert::convert("ppt", "pdf", &twenty_four).unwrap();
        assert_eq!(twenty_four.len(), 14336);
        assert_eq!(twenty_four_pdf.len(), 1991);
        let nibble = ppt_text_nibble_document(&nibble_slide_text()).unwrap();
        let nibble_pdf = crate::convert::convert("ppt", "pdf", &nibble).unwrap();
        assert_eq!(nibble.len(), 9216);
        assert_eq!(nibble_pdf.len(), 2469);
    }

    #[test]
    fn one_text_and_two_eight_bit_pictures_become_pdf() {
        let text = two_indexed_picture_text();
        let file = ppt_text_two_indexed_document(&text).unwrap();
        assert_eq!(file.len(), 14848);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 2816);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
        let (width, height, first_rgb) = expected_indexed_rgb_add(0);
        let (_, _, second_rgb) = expected_indexed_rgb_add(1);
        assert_ne!(first_rgb, second_rgb);
        assert!(crate::pdf::contains_image(&pdf, width, height, &first_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &second_rgb).unwrap());
        assert!(pdf.windows(8).any(|w| w == b"/Im1 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im2 Do "));
        assert!(pdf.windows(9).any(|w| w == b"/Width 80"));
        assert!(pdf.windows(10).any(|w| w == b"/Height 64"));
        let two = ppt_two_text_indexed_document(
            &two_indexed_slide_text().0,
            &two_indexed_slide_text().1,
        )
        .unwrap();
        let two_pdf = crate::convert::convert("ppt", "pdf", &two).unwrap();
        assert_eq!(two.len(), 9728);
        assert_eq!(two_pdf.len(), 3085);
        let indexed = ppt_text_indexed_document(&indexed_slide_text()).unwrap();
        let indexed_pdf = crate::convert::convert("ppt", "pdf", &indexed).unwrap();
        assert_eq!(indexed.len(), 8704);
        assert_eq!(indexed_pdf.len(), 2199);
        let nibble = ppt_text_nibble_document(&nibble_slide_text()).unwrap();
        let nibble_pdf = crate::convert::convert("ppt", "pdf", &nibble).unwrap();
        assert_eq!(nibble.len(), 9216);
        assert_eq!(nibble_pdf.len(), 2469);
    }

    fn two_indexed_picture_text() -> String {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!(
                "Pair indexed shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        text
    }

    fn expected_indexed_rgb_add(add: u8) -> (u32, u32, Vec<u8>) {
        let width = 80u32;
        let height = 64u32;
        let mut rgb = vec![0u8; (width * height * 3) as usize];
        for y in 0..height as usize {
            let index = (y as u8).wrapping_add(add);
            let [r, g, b] = palette_entry(index);
            for x in 0..width as usize {
                let d = (y * width as usize + x) * 3;
                rgb[d] = r;
                rgb[d + 1] = g;
                rgb[d + 2] = b;
            }
        }
        (width, height, rgb)
    }

    #[test]
    fn one_text_and_three_eight_bit_pictures_become_pdf() {
        let text = three_indexed_picture_text();
        let file = ppt_text_three_indexed_document(&text).unwrap();
        assert_eq!(file.len(), 21504);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 3421);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
        let (width, height, first_rgb) = expected_indexed_rgb_add(0);
        let (_, _, second_rgb) = expected_indexed_rgb_add(1);
        let (_, _, third_rgb) = expected_indexed_rgb_add(2);
        assert_ne!(first_rgb, second_rgb);
        assert_ne!(second_rgb, third_rgb);
        assert_ne!(first_rgb, third_rgb);
        assert!(crate::pdf::contains_image(&pdf, width, height, &first_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &second_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &third_rgb).unwrap());
        assert!(pdf.windows(8).any(|w| w == b"/Im1 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im2 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im3 Do "));
        let two = ppt_text_two_indexed_document(&two_indexed_picture_text()).unwrap();
        let two_pdf = crate::convert::convert("ppt", "pdf", &two).unwrap();
        assert_eq!(two.len(), 14848);
        assert_eq!(two_pdf.len(), 2816);
        let (first, second) = two_indexed_slide_text();
        let shapes = ppt_two_text_indexed_document(&first, &second).unwrap();
        let shapes_pdf = crate::convert::convert("ppt", "pdf", &shapes).unwrap();
        assert_eq!(shapes.len(), 9728);
        assert_eq!(shapes_pdf.len(), 3085);
        let nibble = ppt_text_nibble_document(&nibble_slide_text()).unwrap();
        let nibble_pdf = crate::convert::convert("ppt", "pdf", &nibble).unwrap();
        assert_eq!(nibble.len(), 9216);
        assert_eq!(nibble_pdf.len(), 2469);
    }

    fn three_indexed_picture_text() -> String {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!(
                "Triple indexed shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        text
    }

    #[test]
    fn one_text_and_four_eight_bit_pictures_become_pdf() {
        let text = four_indexed_picture_text();
        let file = ppt_text_four_indexed_document(&text).unwrap();
        assert_eq!(file.len(), 27648);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 3978);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
        let (width, height, first_rgb) = expected_indexed_rgb_add(0);
        let (_, _, second_rgb) = expected_indexed_rgb_add(1);
        let (_, _, third_rgb) = expected_indexed_rgb_add(2);
        let (_, _, fourth_rgb) = expected_indexed_rgb_add(3);
        assert_ne!(first_rgb, second_rgb);
        assert_ne!(second_rgb, third_rgb);
        assert_ne!(third_rgb, fourth_rgb);
        assert!(crate::pdf::contains_image(&pdf, width, height, &first_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &second_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &third_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &fourth_rgb).unwrap());
        assert!(pdf.windows(8).any(|w| w == b"/Im1 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im2 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im3 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im4 Do "));
        let three = ppt_text_three_indexed_document(&three_indexed_picture_text()).unwrap();
        let three_pdf = crate::convert::convert("ppt", "pdf", &three).unwrap();
        assert_eq!(three.len(), 21504);
        assert_eq!(three_pdf.len(), 3421);
        let two = ppt_text_two_indexed_document(&two_indexed_picture_text()).unwrap();
        let two_pdf = crate::convert::convert("ppt", "pdf", &two).unwrap();
        assert_eq!(two.len(), 14848);
        assert_eq!(two_pdf.len(), 2816);
        let nibble = ppt_text_nibble_document(&nibble_slide_text()).unwrap();
        let nibble_pdf = crate::convert::convert("ppt", "pdf", &nibble).unwrap();
        assert_eq!(nibble.len(), 9216);
        assert_eq!(nibble_pdf.len(), 2469);
    }

    fn four_indexed_picture_text() -> String {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!(
                "Quad indexed shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        text
    }

    #[test]
    fn one_text_and_five_eight_bit_pictures_become_pdf() {
        let text = five_indexed_picture_text();
        let file = ppt_text_five_indexed_document(&text).unwrap();
        assert_eq!(file.len(), 33792);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 4574);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
        let (width, height, first_rgb) = expected_indexed_rgb_add(0);
        let (_, _, second_rgb) = expected_indexed_rgb_add(1);
        let (_, _, third_rgb) = expected_indexed_rgb_add(2);
        let (_, _, fourth_rgb) = expected_indexed_rgb_add(3);
        let (_, _, fifth_rgb) = expected_indexed_rgb_add(4);
        assert_ne!(first_rgb, second_rgb);
        assert_ne!(second_rgb, third_rgb);
        assert_ne!(third_rgb, fourth_rgb);
        assert_ne!(fourth_rgb, fifth_rgb);
        assert!(crate::pdf::contains_image(&pdf, width, height, &first_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &second_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &third_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &fourth_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &fifth_rgb).unwrap());
        assert!(pdf.windows(8).any(|w| w == b"/Im1 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im2 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im3 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im4 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im5 Do "));
        let four = ppt_text_four_indexed_document(&four_indexed_picture_text()).unwrap();
        let four_pdf = crate::convert::convert("ppt", "pdf", &four).unwrap();
        assert_eq!(four.len(), 27648);
        assert_eq!(four_pdf.len(), 3978);
        let three = ppt_text_three_indexed_document(&three_indexed_picture_text()).unwrap();
        let three_pdf = crate::convert::convert("ppt", "pdf", &three).unwrap();
        assert_eq!(three.len(), 21504);
        assert_eq!(three_pdf.len(), 3421);
        let nibble = ppt_text_nibble_document(&nibble_slide_text()).unwrap();
        let nibble_pdf = crate::convert::convert("ppt", "pdf", &nibble).unwrap();
        assert_eq!(nibble.len(), 9216);
        assert_eq!(nibble_pdf.len(), 2469);
    }

    fn five_indexed_picture_text() -> String {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!(
                "Quint indexed shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        text
    }

    #[test]
    fn one_text_and_six_eight_bit_pictures_become_pdf() {
        let text = six_indexed_picture_text();
        let file = ppt_text_six_indexed_document(&text).unwrap();
        assert_eq!(file.len(), 39936);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 5145);
        let got = crate::pdf::plain_text(&pdf).unwrap();
        assert!(got.contains(&text), "{got}");
        let (width, height, first_rgb) = expected_indexed_rgb_add(0);
        let (_, _, second_rgb) = expected_indexed_rgb_add(1);
        let (_, _, third_rgb) = expected_indexed_rgb_add(2);
        let (_, _, fourth_rgb) = expected_indexed_rgb_add(3);
        let (_, _, fifth_rgb) = expected_indexed_rgb_add(4);
        let (_, _, sixth_rgb) = expected_indexed_rgb_add(5);
        assert_ne!(first_rgb, second_rgb);
        assert_ne!(second_rgb, third_rgb);
        assert_ne!(third_rgb, fourth_rgb);
        assert_ne!(fourth_rgb, fifth_rgb);
        assert_ne!(fifth_rgb, sixth_rgb);
        assert!(crate::pdf::contains_image(&pdf, width, height, &first_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &second_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &third_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &fourth_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &fifth_rgb).unwrap());
        assert!(crate::pdf::contains_image(&pdf, width, height, &sixth_rgb).unwrap());
        assert!(pdf.windows(8).any(|w| w == b"/Im1 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im2 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im3 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im4 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im5 Do "));
        assert!(pdf.windows(8).any(|w| w == b"/Im6 Do "));
        let five = ppt_text_five_indexed_document(&five_indexed_picture_text()).unwrap();
        let five_pdf = crate::convert::convert("ppt", "pdf", &five).unwrap();
        assert_eq!(five.len(), 33792);
        assert_eq!(five_pdf.len(), 4574);
        let four = ppt_text_four_indexed_document(&four_indexed_picture_text()).unwrap();
        let four_pdf = crate::convert::convert("ppt", "pdf", &four).unwrap();
        assert_eq!(four.len(), 27648);
        assert_eq!(four_pdf.len(), 3978);
        let nibble = ppt_text_nibble_document(&nibble_slide_text()).unwrap();
        let nibble_pdf = crate::convert::convert("ppt", "pdf", &nibble).unwrap();
        assert_eq!(nibble.len(), 9216);
        assert_eq!(nibble_pdf.len(), 2469);
    }

    fn six_indexed_picture_text() -> String {
        let mut text = String::new();
        for i in 0..12 {
            text.push_str(&format!(
                "Sext indexed shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        text
    }

    #[test]
    fn seven_pictures_stay_a_miss() {
        let blip = tiny_indexed_blip(0);
        let stream = document_with_pictures(
            &[b"One shape."],
            &[&blip, &blip, &blip, &blip, &blip, &blip, &blip],
        );
        let file = pack_cfb(&stream).unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("not one, two, three, or four text shapes"),
            "{err}"
        );
    }

    #[test]
    fn two_texts_and_two_pictures_stay_a_miss() {
        let first = tiny_indexed_blip(0);
        let second = tiny_indexed_blip(1);
        let stream = document_with_pictures(
            &[b"First shape.", b"Second shape."],
            &[&first, &second],
        );
        let file = pack_cfb(&stream).unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("not one, two, three, or four text shapes"),
            "{err}"
        );
    }

    fn tiny_indexed_blip(add: u8) -> Vec<u8> {
        let dib = encode_dib8_index(8, 8, add).unwrap();
        let mut blip = vec![0u8; 17];
        blip[0] = 0xA1;
        blip[16] = 0xFF;
        blip.extend(dib);
        blip
    }

    #[test]
    fn three_text_and_picture_stays_a_miss() {
        let stream = document_with_picture(
            &[b"First shape.", b"Second shape.", b"Third shape."],
            &[0u8; 8],
        );
        let file = pack_cfb(&stream).unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("not one, two, three, or four text shapes"),
            "{err}"
        );
    }

    #[test]
    fn two_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let file = ppt_two_text_document(first, second).unwrap();
        let got = html(&file).unwrap();
        let marked = format!("<p>{first}</p><p>{second}</p>");
        assert!(got.contains(&marked), "{got}");
        assert!(got.find(first).unwrap() < got.find(second).unwrap());
    }

    #[test]
    fn two_shape_stays_the_published_size() {
        let mut first = String::new();
        let mut second = String::new();
        for i in 0..40 {
            first.push_str(&format!(
                "First shape sentence {i} of the generated PowerPoint file. "
            ));
            second.push_str(&format!(
                "Second shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        let file = ppt_two_text_document(&first, &second).unwrap();
        assert_eq!(file.len(), 6656);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 4750);
        assert!(got.contains(&first) && got.contains(&second), "{got}");
    }

    #[test]
    fn three_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let file = ppt_three_text_document(first, second, third).unwrap();
        let got = html(&file).unwrap();
        let marked = format!("<p>{first}</p><p>{second}</p><p>{third}</p>");
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        assert!(a < b && b < c);
    }

    #[test]
    fn three_shape_stays_the_published_size() {
        let mut alpha = String::new();
        let mut beta = String::new();
        let mut gamma = String::new();
        for i in 0..30 {
            alpha.push_str(&format!(
                "Alpha shape sentence {i} of the generated PowerPoint file. "
            ));
            beta.push_str(&format!(
                "Beta shape sentence {i} of the generated PowerPoint file. "
            ));
            gamma.push_str(&format!(
                "Gamma shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        let file = ppt_three_text_document(&alpha, &beta, &gamma).unwrap();
        assert_eq!(file.len(), 7168);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 5257);
        assert!(got.contains(&alpha) && got.contains(&beta) && got.contains(&gamma), "{got}");
    }

    #[test]
    fn four_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let fourth = "Fourth generated legacy PowerPoint text shape.";
        let file = ppt_four_text_document(first, second, third, fourth).unwrap();
        let got = html(&file).unwrap();
        let marked = format!("<p>{first}</p><p>{second}</p><p>{third}</p><p>{fourth}</p>");
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        let d = got.find(fourth).unwrap();
        assert!(a < b && b < c && c < d);
    }

    #[test]
    fn four_shape_stays_the_published_html_size() {
        let (north, east, south, west) = four_shape_fixture();
        let file = ppt_four_text_document(&north, &east, &south, &west).unwrap();
        assert_eq!(file.len(), 7680);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 5814);
        assert!(
            got.contains(&north) && got.contains(&east) && got.contains(&south) && got.contains(&west),
            "{got}"
        );
    }

    #[test]
    fn four_text_shapes_become_pdf_with_the_text() {
        let (north, east, south, west) = four_shape_fixture();
        let file = ppt_four_text_document(&north, &east, &south, &west).unwrap();
        assert_eq!(file.len(), 7680);
        let pdf = crate::convert::convert("ppt", "pdf", &file).unwrap();
        assert_eq!(pdf.len(), 6710);
        let text = crate::pdf::plain_text(&pdf).unwrap();
        assert!(text.contains(&north), "{text}");
        assert!(text.contains(&east), "{text}");
        assert!(text.contains(&south), "{text}");
        assert!(text.contains(&west), "{text}");
        let n = text.find(&north).unwrap();
        let e = text.find(&east).unwrap();
        let s = text.find(&south).unwrap();
        let w = text.find(&west).unwrap();
        assert!(n < e && e < s && s < w);
    }

    #[test]
    fn fewer_shapes_stay_a_pdf_miss() {
        let file = ppt_text_document("Only one generated text shape.").unwrap();
        assert_eq!(file.len() as u64, ONE_SHAPE_PDF_MISS);
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not four text shapes"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    fn four_shape_fixture() -> (String, String, String, String) {
        let mut north = String::new();
        let mut east = String::new();
        let mut south = String::new();
        let mut west = String::new();
        for i in 0..25 {
            north.push_str(&format!(
                "North shape sentence {i} of the generated PowerPoint file. "
            ));
            east.push_str(&format!(
                "East shape sentence {i} of the generated PowerPoint file. "
            ));
            south.push_str(&format!(
                "South shape sentence {i} of the generated PowerPoint file. "
            ));
            west.push_str(&format!(
                "West shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (north, east, south, west)
    }

    #[test]
    fn five_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let fourth = "Fourth generated legacy PowerPoint text shape.";
        let fifth = "Fifth generated legacy PowerPoint text shape.";
        let file = ppt_five_text_document(first, second, third, fourth, fifth).unwrap();
        let got = html(&file).unwrap();
        let marked = format!("<p>{first}</p><p>{second}</p><p>{third}</p><p>{fourth}</p><p>{fifth}</p>");
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        let d = got.find(fourth).unwrap();
        let e = got.find(fifth).unwrap();
        assert!(a < b && b < c && c < d && d < e);
    }

    #[test]
    fn five_shape_strings_are_in_the_html() {
        let (north, east, south, west, center) = five_shape_fixture();
        let file = ppt_five_text_document(&north, &east, &south, &west, &center).unwrap();
        assert_eq!(file.len(), 9216);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 7286);
        let marked = format!("<p>{north}</p><p>{east}</p><p>{south}</p><p>{west}</p><p>{center}</p>");
        assert!(got.contains(&marked), "{got}");
        let n = got.find(&north).unwrap();
        let e = got.find(&east).unwrap();
        let s = got.find(&south).unwrap();
        let w = got.find(&west).unwrap();
        let c = got.find(&center).unwrap();
        assert!(n < e && e < s && s < w && w < c);
    }

    #[test]
    fn five_text_shapes_stay_a_pdf_miss() {
        let (north, east, south, west, center) = five_shape_fixture();
        let file = ppt_five_text_document(&north, &east, &south, &west, &center).unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not four text shapes"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    fn five_shape_fixture() -> (String, String, String, String, String) {
        let (north, east, south, west) = four_shape_fixture();
        let mut center = String::new();
        for i in 0..25 {
            center.push_str(&format!(
                "Center shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (north, east, south, west, center)
    }

    #[test]
    fn six_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let fourth = "Fourth generated legacy PowerPoint text shape.";
        let fifth = "Fifth generated legacy PowerPoint text shape.";
        let sixth = "Sixth generated legacy PowerPoint text shape.";
        let file = ppt_six_text_document(first, second, third, fourth, fifth, sixth).unwrap();
        let got = html(&file).unwrap();
        let marked = format!(
            "<p>{first}</p><p>{second}</p><p>{third}</p><p>{fourth}</p><p>{fifth}</p><p>{sixth}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        let d = got.find(fourth).unwrap();
        let e = got.find(fifth).unwrap();
        let f = got.find(sixth).unwrap();
        assert!(a < b && b < c && c < d && d < e && e < f);
    }

    #[test]
    fn six_shape_strings_are_in_the_html() {
        let (north, east, south, west, center, inner) = six_shape_fixture();
        let file = ppt_six_text_document(&north, &east, &south, &west, &center, &inner).unwrap();
        assert_eq!(file.len(), 10752);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 8733);
        let marked = format!(
            "<p>{north}</p><p>{east}</p><p>{south}</p><p>{west}</p><p>{center}</p><p>{inner}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let n = got.find(&north).unwrap();
        let e = got.find(&east).unwrap();
        let s = got.find(&south).unwrap();
        let w = got.find(&west).unwrap();
        let c = got.find(&center).unwrap();
        let i = got.find(&inner).unwrap();
        assert!(n < e && e < s && s < w && w < c && c < i);
    }

    #[test]
    fn six_text_shapes_stay_a_pdf_miss() {
        let (north, east, south, west, center, inner) = six_shape_fixture();
        let file = ppt_six_text_document(&north, &east, &south, &west, &center, &inner).unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not four text shapes"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    fn six_shape_fixture() -> (String, String, String, String, String, String) {
        let (north, east, south, west, center) = five_shape_fixture();
        let mut inner = String::new();
        for i in 0..25 {
            inner.push_str(&format!(
                "Inner shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (north, east, south, west, center, inner)
    }

    #[test]
    fn seven_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let fourth = "Fourth generated legacy PowerPoint text shape.";
        let fifth = "Fifth generated legacy PowerPoint text shape.";
        let sixth = "Sixth generated legacy PowerPoint text shape.";
        let seventh = "Seventh generated legacy PowerPoint text shape.";
        let file =
            ppt_seven_text_document(first, second, third, fourth, fifth, sixth, seventh).unwrap();
        let got = html(&file).unwrap();
        let marked = format!(
            "<p>{first}</p><p>{second}</p><p>{third}</p><p>{fourth}</p><p>{fifth}</p><p>{sixth}</p><p>{seventh}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        let d = got.find(fourth).unwrap();
        let e = got.find(fifth).unwrap();
        let f = got.find(sixth).unwrap();
        let g = got.find(seventh).unwrap();
        assert!(a < b && b < c && c < d && d < e && e < f && f < g);
    }

    #[test]
    fn seven_shape_strings_are_in_the_html() {
        let (north, east, south, west, center, inner, outer) = seven_shape_fixture();
        let file = ppt_seven_text_document(
            &north, &east, &south, &west, &center, &inner, &outer,
        )
        .unwrap();
        assert_eq!(file.len(), 12288);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 10180);
        let marked = format!(
            "<p>{north}</p><p>{east}</p><p>{south}</p><p>{west}</p><p>{center}</p><p>{inner}</p><p>{outer}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let n = got.find(&north).unwrap();
        let e = got.find(&east).unwrap();
        let s = got.find(&south).unwrap();
        let w = got.find(&west).unwrap();
        let c = got.find(&center).unwrap();
        let i = got.find(&inner).unwrap();
        let o = got.find(&outer).unwrap();
        assert!(n < e && e < s && s < w && w < c && c < i && i < o);
    }

    #[test]
    fn seven_text_shapes_stay_a_pdf_miss() {
        let (north, east, south, west, center, inner, outer) = seven_shape_fixture();
        let file = ppt_seven_text_document(
            &north, &east, &south, &west, &center, &inner, &outer,
        )
        .unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not four text shapes"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    fn seven_shape_fixture() -> (String, String, String, String, String, String, String) {
        let (north, east, south, west, center, inner) = six_shape_fixture();
        let mut outer = String::new();
        for i in 0..25 {
            outer.push_str(&format!(
                "Outer shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (north, east, south, west, center, inner, outer)
    }

    #[test]
    fn eight_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let fourth = "Fourth generated legacy PowerPoint text shape.";
        let fifth = "Fifth generated legacy PowerPoint text shape.";
        let sixth = "Sixth generated legacy PowerPoint text shape.";
        let seventh = "Seventh generated legacy PowerPoint text shape.";
        let eighth = "Eighth generated legacy PowerPoint text shape.";
        let file = ppt_eight_text_document(
            first, second, third, fourth, fifth, sixth, seventh, eighth,
        )
        .unwrap();
        let got = html(&file).unwrap();
        let marked = format!(
            "<p>{first}</p><p>{second}</p><p>{third}</p><p>{fourth}</p><p>{fifth}</p><p>{sixth}</p><p>{seventh}</p><p>{eighth}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        let d = got.find(fourth).unwrap();
        let e = got.find(fifth).unwrap();
        let f = got.find(sixth).unwrap();
        let g = got.find(seventh).unwrap();
        let h = got.find(eighth).unwrap();
        assert!(a < b && b < c && c < d && d < e && e < f && f < g && g < h);
    }

    #[test]
    fn eight_shape_strings_are_in_the_html() {
        let (north, east, south, west, center, inner, outer, rim) = eight_shape_fixture();
        let file = ppt_eight_text_document(
            &north, &east, &south, &west, &center, &inner, &outer, &rim,
        )
        .unwrap();
        assert_eq!(file.len(), 13312);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 11577);
        let marked = format!(
            "<p>{north}</p><p>{east}</p><p>{south}</p><p>{west}</p><p>{center}</p><p>{inner}</p><p>{outer}</p><p>{rim}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let n = got.find(&north).unwrap();
        let e = got.find(&east).unwrap();
        let s = got.find(&south).unwrap();
        let w = got.find(&west).unwrap();
        let c = got.find(&center).unwrap();
        let i = got.find(&inner).unwrap();
        let o = got.find(&outer).unwrap();
        let r = got.find(&rim).unwrap();
        assert!(n < e && e < s && s < w && w < c && c < i && i < o && o < r);
    }

    #[test]
    fn eight_text_shapes_stay_a_pdf_miss() {
        let (north, east, south, west, center, inner, outer, rim) = eight_shape_fixture();
        let file = ppt_eight_text_document(
            &north, &east, &south, &west, &center, &inner, &outer, &rim,
        )
        .unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not four text shapes"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    fn eight_shape_fixture() -> (String, String, String, String, String, String, String, String) {
        let (north, east, south, west, center, inner, outer) = seven_shape_fixture();
        let mut rim = String::new();
        for i in 0..25 {
            rim.push_str(&format!(
                "Rim shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (north, east, south, west, center, inner, outer, rim)
    }

    #[test]
    fn nine_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let fourth = "Fourth generated legacy PowerPoint text shape.";
        let fifth = "Fifth generated legacy PowerPoint text shape.";
        let sixth = "Sixth generated legacy PowerPoint text shape.";
        let seventh = "Seventh generated legacy PowerPoint text shape.";
        let eighth = "Eighth generated legacy PowerPoint text shape.";
        let ninth = "Ninth generated legacy PowerPoint text shape.";
        let file = ppt_nine_text_document(
            first, second, third, fourth, fifth, sixth, seventh, eighth, ninth,
        )
        .unwrap();
        let got = html(&file).unwrap();
        let marked = format!(
            "<p>{first}</p><p>{second}</p><p>{third}</p><p>{fourth}</p><p>{fifth}</p><p>{sixth}</p><p>{seventh}</p><p>{eighth}</p><p>{ninth}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        let d = got.find(fourth).unwrap();
        let e = got.find(fifth).unwrap();
        let f = got.find(sixth).unwrap();
        let g = got.find(seventh).unwrap();
        let h = got.find(eighth).unwrap();
        let i = got.find(ninth).unwrap();
        assert!(a < b && b < c && c < d && d < e && e < f && f < g && g < h && h < i);
    }

    #[test]
    fn nine_shape_strings_are_in_the_html() {
        let (north, east, south, west, center, inner, outer, rim, band) = nine_shape_fixture();
        let file = ppt_nine_text_document(
            &north, &east, &south, &west, &center, &inner, &outer, &rim, &band,
        )
        .unwrap();
        assert_eq!(file.len(), 14848);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 12999);
        let marked = format!(
            "<p>{north}</p><p>{east}</p><p>{south}</p><p>{west}</p><p>{center}</p><p>{inner}</p><p>{outer}</p><p>{rim}</p><p>{band}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let n = got.find(&north).unwrap();
        let e = got.find(&east).unwrap();
        let s = got.find(&south).unwrap();
        let w = got.find(&west).unwrap();
        let c = got.find(&center).unwrap();
        let inn = got.find(&inner).unwrap();
        let o = got.find(&outer).unwrap();
        let r = got.find(&rim).unwrap();
        let band_at = got.find(&band).unwrap();
        assert!(n < e && e < s && s < w && w < c && c < inn && inn < o && o < r && r < band_at);
    }

    #[test]
    fn nine_text_shapes_stay_a_pdf_miss() {
        let (north, east, south, west, center, inner, outer, rim, band) = nine_shape_fixture();
        let file = ppt_nine_text_document(
            &north, &east, &south, &west, &center, &inner, &outer, &rim, &band,
        )
        .unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not four text shapes"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    fn nine_shape_fixture() -> (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    ) {
        let (north, east, south, west, center, inner, outer, rim) = eight_shape_fixture();
        let mut band = String::new();
        for i in 0..25 {
            band.push_str(&format!(
                "Band shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (
            north, east, south, west, center, inner, outer, rim, band,
        )
    }

    #[test]
    fn ten_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let fourth = "Fourth generated legacy PowerPoint text shape.";
        let fifth = "Fifth generated legacy PowerPoint text shape.";
        let sixth = "Sixth generated legacy PowerPoint text shape.";
        let seventh = "Seventh generated legacy PowerPoint text shape.";
        let eighth = "Eighth generated legacy PowerPoint text shape.";
        let ninth = "Ninth generated legacy PowerPoint text shape.";
        let tenth = "Tenth generated legacy PowerPoint text shape.";
        let file = ppt_ten_text_document(
            first, second, third, fourth, fifth, sixth, seventh, eighth, ninth, tenth,
        )
        .unwrap();
        let got = html(&file).unwrap();
        let marked = format!(
            "<p>{first}</p><p>{second}</p><p>{third}</p><p>{fourth}</p><p>{fifth}</p><p>{sixth}</p><p>{seventh}</p><p>{eighth}</p><p>{ninth}</p><p>{tenth}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        let d = got.find(fourth).unwrap();
        let e = got.find(fifth).unwrap();
        let f = got.find(sixth).unwrap();
        let g = got.find(seventh).unwrap();
        let h = got.find(eighth).unwrap();
        let ni = got.find(ninth).unwrap();
        let te = got.find(tenth).unwrap();
        assert!(a < b && b < c && c < d && d < e && e < f && f < g && g < h && h < ni && ni < te);
    }

    #[test]
    fn ten_shape_strings_are_in_the_html() {
        let (north, east, south, west, center, inner, outer, rim, band, arc) = ten_shape_fixture();
        let file = ppt_ten_text_document(
            &north, &east, &south, &west, &center, &inner, &outer, &rim, &band, &arc,
        )
        .unwrap();
        assert_eq!(file.len(), 16384);
        let got = html(&file).unwrap();
        assert_eq!(got.len(), 14396);
        let marked = format!(
            "<p>{north}</p><p>{east}</p><p>{south}</p><p>{west}</p><p>{center}</p><p>{inner}</p><p>{outer}</p><p>{rim}</p><p>{band}</p><p>{arc}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let n = got.find(&north).unwrap();
        let e = got.find(&east).unwrap();
        let s = got.find(&south).unwrap();
        let w = got.find(&west).unwrap();
        let c = got.find(&center).unwrap();
        let inn = got.find(&inner).unwrap();
        let o = got.find(&outer).unwrap();
        let r = got.find(&rim).unwrap();
        let band_at = got.find(&band).unwrap();
        let arc_at = got.find(&arc).unwrap();
        assert!(
            n < e && e < s && s < w && w < c && c < inn && inn < o && o < r && r < band_at
                && band_at < arc_at
        );
    }

    #[test]
    fn ten_text_shapes_stay_a_pdf_miss() {
        let (north, east, south, west, center, inner, outer, rim, band, arc) = ten_shape_fixture();
        let file = ppt_ten_text_document(
            &north, &east, &south, &west, &center, &inner, &outer, &rim, &band, &arc,
        )
        .unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not four text shapes"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    fn ten_shape_fixture() -> (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    ) {
        let (north, east, south, west, center, inner, outer, rim, band) = nine_shape_fixture();
        let mut arc = String::new();
        for i in 0..25 {
            arc.push_str(&format!(
                "Arc shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (
            north, east, south, west, center, inner, outer, rim, band, arc,
        )
    }

    #[test]
    fn eleven_text_shapes_become_html() {
        let first = "First generated legacy PowerPoint text shape.";
        let second = "Second generated legacy PowerPoint text shape.";
        let third = "Third generated legacy PowerPoint text shape.";
        let fourth = "Fourth generated legacy PowerPoint text shape.";
        let fifth = "Fifth generated legacy PowerPoint text shape.";
        let sixth = "Sixth generated legacy PowerPoint text shape.";
        let seventh = "Seventh generated legacy PowerPoint text shape.";
        let eighth = "Eighth generated legacy PowerPoint text shape.";
        let ninth = "Ninth generated legacy PowerPoint text shape.";
        let tenth = "Tenth generated legacy PowerPoint text shape.";
        let eleventh = "Eleventh generated legacy PowerPoint text shape.";
        let file = ppt_eleven_text_document(
            first, second, third, fourth, fifth, sixth, seventh, eighth, ninth, tenth, eleventh,
        )
        .unwrap();
        let got = html(&file).unwrap();
        let marked = format!(
            "<p>{first}</p><p>{second}</p><p>{third}</p><p>{fourth}</p><p>{fifth}</p><p>{sixth}</p><p>{seventh}</p><p>{eighth}</p><p>{ninth}</p><p>{tenth}</p><p>{eleventh}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let a = got.find(first).unwrap();
        let b = got.find(second).unwrap();
        let c = got.find(third).unwrap();
        let d = got.find(fourth).unwrap();
        let e = got.find(fifth).unwrap();
        let f = got.find(sixth).unwrap();
        let g = got.find(seventh).unwrap();
        let h = got.find(eighth).unwrap();
        let ni = got.find(ninth).unwrap();
        let te = got.find(tenth).unwrap();
        let el = got.find(eleventh).unwrap();
        assert!(
            a < b && b < c && c < d && d < e && e < f && f < g && g < h && h < ni && ni < te
                && te < el
        );
    }

    #[test]
    fn eleven_shape_strings_are_in_the_html() {
        let (north, east, south, west, center, inner, outer, rim, band, arc, cove) =
            eleven_shape_fixture();
        let file = ppt_eleven_text_document(
            &north, &east, &south, &west, &center, &inner, &outer, &rim, &band, &arc, &cove,
        )
        .unwrap();
        let got = html(&file).unwrap();
        let marked = format!(
            "<p>{north}</p><p>{east}</p><p>{south}</p><p>{west}</p><p>{center}</p><p>{inner}</p><p>{outer}</p><p>{rim}</p><p>{band}</p><p>{arc}</p><p>{cove}</p>"
        );
        assert!(got.contains(&marked), "{got}");
        let n = got.find(&north).unwrap();
        let e = got.find(&east).unwrap();
        let s = got.find(&south).unwrap();
        let w = got.find(&west).unwrap();
        let c = got.find(&center).unwrap();
        let inn = got.find(&inner).unwrap();
        let o = got.find(&outer).unwrap();
        let r = got.find(&rim).unwrap();
        let band_at = got.find(&band).unwrap();
        let arc_at = got.find(&arc).unwrap();
        let cove_at = got.find(&cove).unwrap();
        assert!(
            n < e && e < s && s < w && w < c && c < inn && inn < o && o < r && r < band_at
                && band_at < arc_at
                && arc_at < cove_at
        );
    }

    #[test]
    fn eleven_text_shapes_stay_a_pdf_miss() {
        let (north, east, south, west, center, inner, outer, rim, band, arc, cove) =
            eleven_shape_fixture();
        let file = ppt_eleven_text_document(
            &north, &east, &south, &west, &center, &inner, &outer, &rim, &band, &arc, &cove,
        )
        .unwrap();
        let err = crate::convert::convert("ppt", "pdf", &file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(err.message.contains("not four text shapes"), "{err}");
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    fn eleven_shape_fixture() -> (
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
        String,
    ) {
        let (north, east, south, west, center, inner, outer, rim, band, arc) = ten_shape_fixture();
        let mut cove = String::new();
        for i in 0..25 {
            cove.push_str(&format!(
                "Cove shape sentence {i} of the generated PowerPoint file. "
            ));
        }
        (
            north, east, south, west, center, inner, outer, rim, band, arc, cove,
        )
    }

    #[test]
    fn twelve_text_shapes_stay_a_miss() {
        let stream = document_stream(&[
            b"One shape.",
            b"Two shape.",
            b"Three shape.",
            b"Four shape.",
            b"Five shape.",
            b"Six shape.",
            b"Seven shape.",
            b"Eight shape.",
            b"Nine shape.",
            b"Ten shape.",
            b"Eleven shape.",
            b"Twelve shape.",
        ]);
        let file = pack_cfb(&stream).unwrap();
        let err = html(&file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("not one, two, three, or four text shapes"),
            "{err}"
        );
        assert!(
            err.message.contains(&format!("input_b={}", file.len())),
            "{err}"
        );
    }

    #[test]
    fn long_text_shape_uses_the_fat() {
        let text = "A".repeat(5000);
        let file = ppt_text_document(&text).unwrap();
        assert!(text.len() >= MINI_CUTOFF as usize);
        let got = html(&file).unwrap();
        assert!(got.contains(&text), "{got}");
    }

    #[test]
    fn unreadable_compound_document_stays_a_miss() {
        let err = html(b"not a ppt").unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("compound document is not readable"),
            "{err}"
        );
        assert!(err.message.contains("input_b=9"), "{err}");
        let word = crate::doc::word97_document(&["Hi"]).unwrap();
        let err = html(&word).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("has no PowerPoint Document stream"),
            "{err}"
        );
        assert!(
            err.message.contains(&format!("input_b={}", word.len())),
            "{err}"
        );
    }
}
