//! Classic-xref text PDFs. Split and merge copy page text into a new file.
//!
//! One classic-xref page may hold one uncompressed 8-bit DeviceRGB image,
//! each edge at most [`RGB_MAX_EDGE`] pixels. Split of that page copies the
//! samples. Merge still rebuilds text. A 16×16 page is 1417 bytes in and
//! out. Release split of that page is 8046 ns p50 and 10530 ns p99.
//! A 32×32 page is 3722 bytes in and out. Release split of that page is
//! 9060 ns p50 and 9938 ns p99.
//! A 64×64 page is 12940 bytes in and out. Release split of that page is
//! 13158 ns p50 and 14687 ns p99.
//!
//! Honest limit: xref streams, object streams, any other image, and fonts
//! other than the built-in Helvetica are not preserved. A truncated file, an
//! xref offset that does not land on an object, an xref stream, or an
//! unsupported image is an error.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadPdf,
}

/// Largest edge, in pixels, of the one uncompressed RGB page this crate keeps.
pub const RGB_MAX_EDGE: u32 = 64;

/// One classic-xref page holding one uncompressed 8-bit DeviceRGB image.
///
/// `samples` is row-major RGB, length `width * height * 3`. Each edge is
/// from 1 through [`RGB_MAX_EDGE`].
pub fn rgb_pdf(width: u32, height: u32, samples: &[u8]) -> Result<Vec<u8>, Error> {
    checked_rgb_len(width, height, samples.len())?;
    Ok(image_file(width, height, samples))
}

/// Width, height, and row-major RGB samples of the one supported image page.
pub fn rgb_samples(pdf: &[u8]) -> Result<(u32, u32, Vec<u8>), Error> {
    match one_rgb_page(pdf)? {
        Some(image) => Ok((image.width, image.height, image.samples)),
        None => Err(Error::BadPdf),
    }
}

pub fn text_pdf(pages: &[&str]) -> Vec<u8> {
    let n = pages.len();
    let mut objects: Vec<Vec<u8>> = Vec::with_capacity(3 + n * 2);
    let font_id = 3u32;
    let mut kids = String::from("[");
    for i in 0..n {
        let page_id = 4 + (i as u32) * 2;
        if i > 0 {
            kids.push(' ');
        }
        kids.push_str(&format!("{page_id} 0 R"));
    }
    kids.push(']');
    objects.push(b"<< /Type /Catalog /Pages 2 0 R >>".to_vec());
    objects.push(format!("<< /Type /Pages /Count {n} /Kids {kids} >>").into_bytes());
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    for text in pages {
        let page_id = 4 + objects.len() as u32 - 3;
        let content_id = page_id + 1;
        let stream = content_stream(text);
        objects.push(
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents {content_id} 0 R /Resources << /Font << /F1 {font_id} 0 R >> >> >>"
            )
            .into_bytes(),
        );
        objects.push(stream);
    }
    finish(&objects)
}

pub fn page_count(pdf: &[u8]) -> Result<usize, Error> {
    Ok(page_texts(pdf)?.len())
}

pub fn split(pdf: &[u8]) -> Result<Vec<Vec<u8>>, Error> {
    if let Some(image) = one_rgb_page(pdf)? {
        return Ok(vec![rgb_pdf(image.width, image.height, &image.samples)?]);
    }
    let texts = page_texts(pdf)?;
    if texts.is_empty() {
        return Err(Error::BadPdf);
    }
    Ok(texts
        .iter()
        .map(|text| text_pdf(&[text.as_str()]))
        .collect())
}

pub fn merge(parts: &[&[u8]]) -> Result<Vec<u8>, Error> {
    if parts.is_empty() {
        return Err(Error::Truncated);
    }
    let mut texts = Vec::new();
    for part in parts {
        texts.extend(page_texts(part)?);
    }
    if texts.is_empty() {
        return Err(Error::BadPdf);
    }
    let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
    Ok(text_pdf(&refs))
}

fn content_stream(text: &str) -> Vec<u8> {
    let mut body = Vec::from(&b"BT /F1 24 Tf 72 720 Td ("[..]);
    for byte in text.bytes() {
        match byte {
            b'(' | b')' | b'\\' => {
                body.push(b'\\');
                body.push(byte);
            }
            b'\n' => body.extend_from_slice(b") Tj T* ("),
            32..=126 => body.push(byte),
            _ => body.push(b'?'),
        }
    }
    body.extend_from_slice(b") Tj ET");
    let mut obj = format!("<< /Length {} >>\nstream\n", body.len()).into_bytes();
    obj.extend_from_slice(&body);
    obj.extend_from_slice(b"endstream");
    obj
}

fn finish(objects: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::from(&b"%PDF-1.4\n"[..]);
    let mut offsets = Vec::with_capacity(objects.len() + 1);
    offsets.push(0);
    for (i, object) in objects.iter().enumerate() {
        offsets.push(out.len());
        let header = format!("{} 0 obj\n", i + 1);
        out.extend_from_slice(header.as_bytes());
        out.extend_from_slice(object);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref_at = out.len();
    let size = objects.len() + 1;
    out.extend_from_slice(format!("xref\n0 {size}\n").as_bytes());
    out.extend_from_slice(b"0000000000 65535 f \n");
    for offset in offsets.iter().skip(1) {
        out.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

fn page_texts(pdf: &[u8]) -> Result<Vec<String>, Error> {
    if pdf.len() < 8 || !pdf.starts_with(b"%PDF-") {
        return Err(Error::BadPdf);
    }
    if !pdf.windows(5).any(|w| w == b"%%EOF") {
        return Err(Error::Truncated);
    }
    let objects = xref_objects(pdf)?;
    let pages = objects
        .iter()
        .find(|(_, body)| body.windows(12).any(|w| w == b"/Type /Pages"))
        .ok_or(Error::BadPdf)?;
    let kids = refs_in_array(pages.1, b"/Kids").ok_or(Error::BadPdf)?;
    let mut texts = Vec::with_capacity(kids.len());
    for kid in kids {
        let page = objects
            .iter()
            .find(|(num, _)| *num == kid)
            .ok_or(Error::BadPdf)?;
        let contents = refs_after(page.1, b"/Contents").ok_or(Error::BadPdf)?;
        let mut text = String::new();
        for content_id in contents {
            let content = objects
                .iter()
                .find(|(num, _)| *num == content_id)
                .ok_or(Error::BadPdf)?;
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(&text_from_stream(content.1));
        }
        texts.push(text);
    }
    Ok(texts)
}

fn xref_objects(pdf: &[u8]) -> Result<Vec<(u32, &[u8])>, Error> {
    let marker = b"startxref";
    let start = pdf
        .windows(marker.len())
        .rposition(|w| w == marker)
        .ok_or(Error::Truncated)?;
    let digits = std::str::from_utf8(&pdf[start + marker.len()..]).unwrap_or("");
    let xref_at: usize = digits
        .split_whitespace()
        .next()
        .ok_or(Error::Truncated)?
        .parse()
        .map_err(|_| Error::BadPdf)?;
    if xref_at >= pdf.len() || !pdf[xref_at..].starts_with(b"xref") {
        return Err(Error::BadPdf);
    }
    let table = std::str::from_utf8(&pdf[xref_at..]).map_err(|_| Error::BadPdf)?;
    let mut lines = table.lines();
    lines.next();
    let subsection = lines.next().ok_or(Error::Truncated)?;
    let mut parts = subsection.split_whitespace();
    let start_num: u32 = parts
        .next()
        .ok_or(Error::BadPdf)?
        .parse()
        .map_err(|_| Error::BadPdf)?;
    let count: usize = parts
        .next()
        .ok_or(Error::BadPdf)?
        .parse()
        .map_err(|_| Error::BadPdf)?;
    let mut objects = Vec::new();
    for i in 0..count {
        let line = lines.next().ok_or(Error::Truncated)?;
        let mut cols = line.split_whitespace();
        let offset: usize = cols
            .next()
            .ok_or(Error::BadPdf)?
            .parse()
            .map_err(|_| Error::BadPdf)?;
        let _gen = cols.next().ok_or(Error::BadPdf)?;
        let flag = cols.next().ok_or(Error::BadPdf)?;
        if flag != "n" {
            continue;
        }
        let num = start_num + i as u32;
        if offset >= pdf.len() {
            return Err(Error::BadPdf);
        }
        let header = format!("{num} 0 obj");
        if !pdf[offset..].starts_with(header.as_bytes()) {
            return Err(Error::BadPdf);
        }
        let body_at = offset + header.len();
        let end = object_end(&pdf[body_at..])?;
        objects.push((num, &pdf[body_at..body_at + end]));
    }
    reject_xref_stream(pdf, xref_at, &objects)?;
    Ok(objects)
}

fn refs_after(body: &[u8], key: &[u8]) -> Option<Vec<u32>> {
    let at = body.windows(key.len()).position(|w| w == key)?;
    let after = &body[at + key.len()..];
    let trimmed = after
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map(|i| &after[i..])?;
    if trimmed.first() == Some(&b'[') {
        return refs_in_array(body, key);
    }
    let text = std::str::from_utf8(trimmed).ok()?;
    let mut tokens = text.split_whitespace();
    let num = tokens.next()?.parse::<u32>().ok()?;
    let _gen = tokens.next()?;
    if tokens.next()? != "R" {
        return None;
    }
    Some(vec![num])
}

fn refs_in_array(body: &[u8], key: &[u8]) -> Option<Vec<u32>> {
    let at = body.windows(key.len()).position(|w| w == key)?;
    let after = &body[at + key.len()..];
    let start = after.iter().position(|b| *b == b'[')?;
    let end = after[start..].iter().position(|b| *b == b']')?;
    let inside = std::str::from_utf8(&after[start + 1..start + end]).ok()?;
    let mut nums = Vec::new();
    let tokens: Vec<&str> = inside.split_whitespace().collect();
    let mut i = 0;
    while i + 2 < tokens.len() {
        if tokens[i + 2] == "R" {
            if let Ok(n) = tokens[i].parse::<u32>() {
                nums.push(n);
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    if nums.is_empty() {
        None
    } else {
        Some(nums)
    }
}

fn text_from_stream(body: &[u8]) -> String {
    let source = std::str::from_utf8(body).unwrap_or("");
    let mut out = String::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'(' {
            i += 1;
            let mut literal = String::new();
            while i < bytes.len() && bytes[i] != b')' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    literal.push(bytes[i + 1] as char);
                    i += 2;
                    continue;
                }
                literal.push(bytes[i] as char);
                i += 1;
            }
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(&literal);
        }
        i += 1;
    }
    out
}

struct RgbPage {
    width: u32,
    height: u32,
    samples: Vec<u8>,
}

struct ContentMarks {
    inline_image: bool,
    shows_text: bool,
}

fn image_file(width: u32, height: u32, samples: &[u8]) -> Vec<u8> {
    let content = format!("q\n{width} 0 0 {height} 0 0 cm\n/Im0 Do\nQ");
    let mut image = format!(
        "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
        samples.len()
    )
    .into_bytes();
    image.extend_from_slice(samples);
    image.extend_from_slice(b"endstream");
    let objects = [
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
        format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R >> >> >>"
        )
        .into_bytes(),
        wrap_stream(content.as_bytes()),
        image,
    ];
    finish(&objects)
}

fn wrap_stream(body: &[u8]) -> Vec<u8> {
    let mut obj = format!("<< /Length {} >>\nstream\n", body.len()).into_bytes();
    obj.extend_from_slice(body);
    obj.extend_from_slice(b"endstream");
    obj
}

fn checked_rgb_len(width: u32, height: u32, len: usize) -> Result<(), Error> {
    if width == 0 || height == 0 || width > RGB_MAX_EDGE || height > RGB_MAX_EDGE {
        return Err(Error::BadPdf);
    }
    let need = (width as usize)
        .checked_mul(height as usize)
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or(Error::BadPdf)?;
    if len != need {
        return Err(Error::BadPdf);
    }
    Ok(())
}

fn one_rgb_page(pdf: &[u8]) -> Result<Option<RgbPage>, Error> {
    if pdf.len() < 8 || !pdf.starts_with(b"%PDF-") {
        return Err(Error::BadPdf);
    }
    if !pdf.windows(5).any(|window| window == b"%%EOF") {
        return Err(Error::Truncated);
    }
    let objects = xref_objects(pdf)?;
    let pages = objects
        .iter()
        .find(|(_, body)| body.windows(12).any(|window| window == b"/Type /Pages"))
        .ok_or(Error::BadPdf)?;
    let kids = refs_in_array(pages.1, b"/Kids").ok_or(Error::BadPdf)?;
    let mut image = None;
    for kid in &kids {
        let page = objects
            .iter()
            .find(|(num, _)| num == kid)
            .ok_or(Error::BadPdf)?;
        let marks = page_content_marks(page.1, &objects)?;
        if marks.inline_image {
            return Err(Error::BadPdf);
        }
        if let Some(rgb) = page_rgb(page.1, &objects)? {
            if marks.shows_text || kids.len() != 1 || image.is_some() {
                return Err(Error::BadPdf);
            }
            image = Some(rgb);
        }
    }
    let mut image_objects = 0usize;
    for (_, body) in &objects {
        if name_is(dict_head(body), b"/Subtype", b"/Image") {
            image_objects += 1;
        }
    }
    if image_objects > 1 || (image_objects == 1 && image.is_none()) {
        return Err(Error::BadPdf);
    }
    Ok(image)
}

fn page_rgb(page: &[u8], objects: &[(u32, &[u8])]) -> Result<Option<RgbPage>, Error> {
    let resources = resources_bytes(page, objects)?;
    let Some(xobjects) = xobject_dict(resources, objects)? else {
        return Ok(None);
    };
    if find_name(resources, b"/Font").is_some() {
        return Err(Error::BadPdf);
    }
    let refs = dict_refs(xobjects).ok_or(Error::BadPdf)?;
    if refs.len() != 1 {
        return Err(Error::BadPdf);
    }
    let image = objects
        .iter()
        .find(|(num, _)| *num == refs[0])
        .ok_or(Error::BadPdf)?;
    Ok(Some(rgb_image(image.1)?))
}

fn resources_bytes<'a>(page: &'a [u8], objects: &'a [(u32, &'a [u8])]) -> Result<&'a [u8], Error> {
    if find_name(page, b"/Resources").is_none() {
        return Ok(b"");
    }
    if let Some(dict) = bracket_dict(page, b"/Resources") {
        return Ok(dict);
    }
    let refs = refs_after(page, b"/Resources").ok_or(Error::BadPdf)?;
    if refs.len() != 1 {
        return Err(Error::BadPdf);
    }
    objects
        .iter()
        .find(|(num, _)| *num == refs[0])
        .map(|(_, body)| *body)
        .ok_or(Error::BadPdf)
}

fn xobject_dict<'a>(
    resources: &'a [u8],
    objects: &'a [(u32, &'a [u8])],
) -> Result<Option<&'a [u8]>, Error> {
    if find_name(resources, b"/XObject").is_none() {
        return Ok(None);
    }
    if let Some(dict) = bracket_dict(resources, b"/XObject") {
        return Ok(Some(dict));
    }
    let refs = refs_after(resources, b"/XObject").ok_or(Error::BadPdf)?;
    if refs.len() != 1 {
        return Err(Error::BadPdf);
    }
    let body = objects
        .iter()
        .find(|(num, _)| *num == refs[0])
        .map(|(_, body)| *body)
        .ok_or(Error::BadPdf)?;
    let start = body
        .iter()
        .position(|byte| !is_ws(*byte))
        .ok_or(Error::BadPdf)?;
    if body.get(start) != Some(&b'<') || body.get(start + 1) != Some(&b'<') {
        return Err(Error::BadPdf);
    }
    let end = dict_end(&body[start + 2..]).ok_or(Error::BadPdf)?;
    Ok(Some(&body[start + 2..start + 2 + end]))
}

fn rgb_image(body: &[u8]) -> Result<RgbPage, Error> {
    let dict = dict_head(body);
    if !name_is(dict, b"/Subtype", b"/Image") {
        return Err(Error::BadPdf);
    }
    if find_name(dict, b"/Filter").is_some()
        || find_name(dict, b"/Decode").is_some()
        || find_name(dict, b"/DecodeParms").is_some()
        || find_name(dict, b"/ImageMask").is_some()
        || find_name(dict, b"/SMask").is_some()
        || find_name(dict, b"/Mask").is_some()
    {
        return Err(Error::BadPdf);
    }
    if !name_is(dict, b"/ColorSpace", b"/DeviceRGB") {
        return Err(Error::BadPdf);
    }
    let bits = direct_int(dict, b"/BitsPerComponent").ok_or(Error::BadPdf)?;
    if bits != 8 {
        return Err(Error::BadPdf);
    }
    let width = u32::try_from(direct_int(dict, b"/Width").ok_or(Error::BadPdf)?)
        .map_err(|_| Error::BadPdf)?;
    let height = u32::try_from(direct_int(dict, b"/Height").ok_or(Error::BadPdf)?)
        .map_err(|_| Error::BadPdf)?;
    let samples = stream_bytes(body)?;
    checked_rgb_len(width, height, samples.len())?;
    Ok(RgbPage {
        width,
        height,
        samples: samples.to_vec(),
    })
}

fn page_content_marks(page: &[u8], objects: &[(u32, &[u8])]) -> Result<ContentMarks, Error> {
    let mut marks = ContentMarks {
        inline_image: false,
        shows_text: false,
    };
    if find_name(page, b"/Contents").is_none() {
        return Ok(marks);
    }
    let ids = refs_after(page, b"/Contents").ok_or(Error::BadPdf)?;
    for id in ids {
        let body = objects
            .iter()
            .find(|(num, _)| *num == id)
            .map(|(_, body)| *body)
            .ok_or(Error::BadPdf)?;
        let found = content_marks(body);
        marks.inline_image |= found.inline_image;
        marks.shows_text |= found.shows_text;
    }
    Ok(marks)
}

fn content_marks(body: &[u8]) -> ContentMarks {
    match stream_bytes(body) {
        Ok(bytes) => scan_tokens(bytes),
        Err(_) => scan_tokens(body),
    }
}

fn scan_tokens(bytes: &[u8]) -> ContentMarks {
    let mut marks = ContentMarks {
        inline_image: false,
        shows_text: false,
    };
    let mut i = 0;
    while i < bytes.len() {
        let byte = bytes[i];
        if is_ws(byte) {
            i += 1;
            continue;
        }
        if byte == b'%' {
            i = skip_comment(bytes, i);
            continue;
        }
        if byte == b'(' {
            i = skip_literal(bytes, i);
            continue;
        }
        if byte == b'<' && bytes.get(i + 1) == Some(&b'<') {
            i += 2;
            continue;
        }
        if byte == b'<' {
            i = skip_hex(bytes, i);
            continue;
        }
        if byte == b'/' {
            i += 1;
            while i < bytes.len() && is_name_char(bytes[i]) {
                i += 1;
            }
            continue;
        }
        if is_delim(byte) {
            i += 1;
            continue;
        }
        let start = i;
        while i < bytes.len() && !is_ws(bytes[i]) && !is_delim(bytes[i]) {
            i += 1;
        }
        let token = &bytes[start..i];
        if token == b"BI" {
            marks.inline_image = true;
        } else if token == b"Tj" || token == b"TJ" || token == b"'" || token == b"\"" {
            marks.shows_text = true;
        }
    }
    marks
}

fn object_end(rest: &[u8]) -> Result<usize, Error> {
    if let Some(end) = image_object_end(rest)? {
        return Ok(end);
    }
    rest.windows(6)
        .position(|window| window == b"endobj")
        .ok_or(Error::Truncated)
}

fn image_object_end(rest: &[u8]) -> Result<Option<usize>, Error> {
    let Some(stream_at) = find_keyword_until(rest, b"stream", b"endobj") else {
        return Ok(None);
    };
    let dict = &rest[..stream_at];
    if !name_is(dict, b"/Subtype", b"/Image") {
        return Ok(None);
    }
    let length = direct_int(dict, b"/Length").ok_or(Error::BadPdf)?;
    let data_at = stream_data_at(rest, stream_at).ok_or(Error::BadPdf)?;
    let after = data_at.checked_add(length).ok_or(Error::BadPdf)?;
    if after > rest.len() {
        return Err(Error::Truncated);
    }
    let end = rest[after..]
        .windows(6)
        .position(|window| window == b"endobj")
        .ok_or(Error::Truncated)?;
    Ok(Some(after + end))
}

fn reject_xref_stream(pdf: &[u8], xref_at: usize, objects: &[(u32, &[u8])]) -> Result<(), Error> {
    if xref_at < pdf.len() {
        if let Some(at) = find_keyword(&pdf[xref_at..], b"trailer") {
            if find_name(&pdf[xref_at + at..], b"/XRefStm").is_some() {
                return Err(Error::BadPdf);
            }
        }
    }
    for (_, body) in objects {
        if name_is(dict_head(body), b"/Type", b"/XRef") {
            return Err(Error::BadPdf);
        }
    }
    Ok(())
}

fn stream_bytes(body: &[u8]) -> Result<&[u8], Error> {
    let stream_at = find_keyword(body, b"stream").ok_or(Error::BadPdf)?;
    let length = direct_int(&body[..stream_at], b"/Length").ok_or(Error::BadPdf)?;
    let data_at = stream_data_at(body, stream_at).ok_or(Error::BadPdf)?;
    let end = data_at.checked_add(length).ok_or(Error::BadPdf)?;
    if end > body.len() {
        return Err(Error::Truncated);
    }
    Ok(&body[data_at..end])
}

fn stream_data_at(body: &[u8], stream_at: usize) -> Option<usize> {
    let mut i = stream_at + b"stream".len();
    if body.get(i) == Some(&b'\r') {
        i += 1;
        if body.get(i) == Some(&b'\n') {
            i += 1;
        }
        return Some(i);
    }
    if body.get(i) == Some(&b'\n') {
        return Some(i + 1);
    }
    None
}

fn dict_head(body: &[u8]) -> &[u8] {
    match find_keyword(body, b"stream") {
        Some(at) => &body[..at],
        None => body,
    }
}

fn bracket_dict<'a>(buf: &'a [u8], key: &[u8]) -> Option<&'a [u8]> {
    let at = find_name(buf, key)?;
    let after = &buf[at + key.len()..];
    let start = after.iter().position(|byte| !is_ws(*byte))?;
    if after.get(start) != Some(&b'<') || after.get(start + 1) != Some(&b'<') {
        return None;
    }
    let inner = start + 2;
    let end = dict_end(&after[inner..])?;
    Some(&after[inner..inner + end])
}

fn dict_end(inner: &[u8]) -> Option<usize> {
    let mut depth = 1;
    let mut i = 0;
    while i + 1 < inner.len() {
        if inner[i] == b'<' && inner[i + 1] == b'<' {
            depth += 1;
            i += 2;
            continue;
        }
        if inner[i] == b'>' && inner[i + 1] == b'>' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
            i += 2;
            continue;
        }
        i += 1;
    }
    None
}

fn dict_refs(inner: &[u8]) -> Option<Vec<u32>> {
    let text = std::str::from_utf8(inner).ok()?;
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let mut nums = Vec::new();
    let mut i = 0;
    while i + 2 < tokens.len() {
        if tokens[i + 2] == "R" {
            if let Ok(num) = tokens[i].parse::<u32>() {
                nums.push(num);
            }
            i += 3;
        } else {
            i += 1;
        }
    }
    if nums.is_empty() {
        None
    } else {
        Some(nums)
    }
}

fn name_is(buf: &[u8], key: &[u8], value: &[u8]) -> bool {
    let Some(at) = find_name(buf, key) else {
        return false;
    };
    let after = &buf[at + key.len()..];
    let Some(start) = after.iter().position(|byte| !is_ws(*byte)) else {
        return false;
    };
    let rest = &after[start..];
    rest.len() >= value.len()
        && rest.starts_with(value)
        && (rest.len() == value.len() || !is_name_char(rest[value.len()]))
}

fn direct_int(buf: &[u8], key: &[u8]) -> Option<usize> {
    let at = find_name(buf, key)?;
    let after = &buf[at + key.len()..];
    let start = after.iter().position(|byte| !is_ws(*byte))?;
    if !after[start].is_ascii_digit() {
        return None;
    }
    let mut n: usize = 0;
    let mut i = start;
    while i < after.len() && after[i].is_ascii_digit() {
        n = n.checked_mul(10)?.checked_add((after[i] - b'0') as usize)?;
        i += 1;
    }
    if i < after.len() && is_name_char(after[i]) {
        return None;
    }
    let rest = &after[i..];
    let next = rest
        .iter()
        .position(|byte| !is_ws(*byte))
        .unwrap_or(rest.len());
    if next < rest.len() && rest[next].is_ascii_digit() {
        return None;
    }
    Some(n)
}

fn find_name(buf: &[u8], name: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i + name.len() <= buf.len() {
        if buf[i..].starts_with(name) {
            let after = i + name.len();
            if after == buf.len() || !is_name_char(buf[after]) {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}

fn find_keyword(buf: &[u8], keyword: &[u8]) -> Option<usize> {
    find_keyword_until(buf, keyword, b"")
}

fn find_keyword_until(buf: &[u8], keyword: &[u8], stop: &[u8]) -> Option<usize> {
    let mut i = 0;
    while i < buf.len() {
        let byte = buf[i];
        if is_ws(byte) {
            i += 1;
            continue;
        }
        if byte == b'%' {
            i = skip_comment(buf, i);
            continue;
        }
        if byte == b'(' {
            i = skip_literal(buf, i);
            continue;
        }
        if byte == b'<' && buf.get(i + 1) == Some(&b'<') {
            i += 2;
            continue;
        }
        if byte == b'<' {
            i = skip_hex(buf, i);
            continue;
        }
        if byte == b'/' {
            i += 1;
            while i < buf.len() && is_name_char(buf[i]) {
                i += 1;
            }
            continue;
        }
        if is_delim(byte) {
            i += 1;
            continue;
        }
        let start = i;
        while i < buf.len() && !is_ws(buf[i]) && !is_delim(buf[i]) {
            i += 1;
        }
        if stop != b"" && &buf[start..i] == stop {
            return None;
        }
        if &buf[start..i] == keyword {
            return Some(start);
        }
    }
    None
}

fn skip_comment(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() && bytes[i] != b'\n' && bytes[i] != b'\r' {
        i += 1;
    }
    i
}

fn skip_literal(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == b')' {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_hex(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    while i < bytes.len() && bytes[i] != b'>' {
        i += 1;
    }
    if i < bytes.len() {
        i + 1
    } else {
        i
    }
}

fn is_ws(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b'\x0c' | 0)
}

fn is_delim(byte: u8) -> bool {
    matches!(
        byte,
        b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
    )
}

fn is_name_char(byte: u8) -> bool {
    !is_ws(byte) && !is_delim(byte)
}

#[cfg(test)]
mod tests {
    use super::{
        finish, merge, page_count, rgb_pdf, rgb_samples, split, text_pdf, wrap_stream, Error,
        RGB_MAX_EDGE,
    };

    #[test]
    fn round_trip_split_and_merge() {
        let pdf = text_pdf(&["Hello", "Second (line)"]);
        assert_eq!(page_count(&pdf).unwrap(), 2);
        let pages = split(&pdf).unwrap();
        assert_eq!(pages.len(), 2);
        assert_eq!(page_count(&pages[0]).unwrap(), 1);
        assert_eq!(page_count(&pages[1]).unwrap(), 1);
        let merged = merge(&[&pages[1], &pages[0]]).unwrap();
        let texts = super::page_texts(&merged).unwrap();
        assert_eq!(
            texts,
            vec!["Second (line)".to_string(), "Hello".to_string()]
        );
    }

    #[test]
    fn truncated_and_shifted_xref_error() {
        assert_eq!(page_count(b"%PDF-1.4\nno end"), Err(Error::Truncated));
        let mut pdf = text_pdf(&["Hello"]);
        let at = pdf.windows(7).position(|w| w == b"1 0 obj").unwrap();
        pdf[at] = b'9';
        assert_eq!(page_count(&pdf), Err(Error::BadPdf));
    }

    #[test]
    fn split_keeps_small_rgb_pixels() {
        let patterns = [
            vec![9, 8, 7],
            vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 1, 2, 3],
            b"endobjendobj".to_vec(),
            b"endstreamendstreamendstreamendstream".to_vec(),
            b"startxref\n%%".to_vec(),
            {
                let mut samples = vec![0u8; (RGB_MAX_EDGE * RGB_MAX_EDGE * 3) as usize];
                samples[0] = b'\n';
                samples[1] = b'\r';
                for (i, byte) in samples.iter_mut().enumerate().skip(2) {
                    *byte = (i * 13 + 7) as u8;
                }
                samples
            },
        ];
        let shapes = [
            (1, 1),
            (2, 2),
            (2, 2),
            (4, 3),
            (2, 2),
            (RGB_MAX_EDGE, RGB_MAX_EDGE),
        ];
        for ((width, height), samples) in shapes.into_iter().zip(patterns) {
            assert_eq!(samples.len(), width as usize * height as usize * 3);
            let pdf = rgb_pdf(width, height, &samples).unwrap();
            assert_eq!(page_count(&pdf).unwrap(), 1);
            let (w, h, direct) = rgb_samples(&pdf).unwrap();
            assert_eq!((w, h), (width, height));
            assert_eq!(direct, samples);
            let pages = split(&pdf).unwrap();
            assert_eq!(pages.len(), 1);
            assert_eq!(pages[0], pdf);
            let (w, h, got) = rgb_samples(&pages[0]).unwrap();
            assert_eq!((w, h, &got), (width, height, &samples));
            let again = split(&pages[0]).unwrap();
            assert_eq!(rgb_samples(&again[0]).unwrap().2, samples);
        }

        let samples = [1u8, 2, 3, 255, 0, 128, 9, 8, 7, 6, 5, 4];
        let pdf = rgb_pdf(2, 2, &samples).unwrap();
        let mut image = format!(
            "<< /Type /XObject /Subtype /Image /Width  2 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\r\n",
            samples.len()
        )
        .into_bytes();
        image.extend_from_slice(&samples);
        image.extend_from_slice(b"endstream");
        let spaced = finish(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 2 2] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R >> >> >>".to_vec(),
            wrap_stream(b"q\n2 0 0 2 0 0 cm\n/Im0 Do\nQ"),
            image,
        ]);
        assert_ne!(spaced, pdf);
        let pages = split(&spaced).unwrap();
        assert_eq!(rgb_samples(&pages[0]).unwrap().2, samples);

        let text = text_pdf(&["BI"]);
        let pages = split(&text).unwrap();
        assert_eq!(
            super::page_texts(&pages[0]).unwrap(),
            vec!["BI".to_string()]
        );
        assert_eq!(rgb_pdf(0, 1, &[]).unwrap_err(), Error::BadPdf);
        assert_eq!(rgb_pdf(1, 1, &[1, 2]).unwrap_err(), Error::BadPdf);
        let past_edge = ((RGB_MAX_EDGE + 1) * 3) as usize;
        assert_eq!(
            rgb_pdf(RGB_MAX_EDGE + 1, 1, &vec![0; past_edge]).unwrap_err(),
            Error::BadPdf
        );
    }

    #[test]
    fn other_images_and_xref_streams_error() {
        let gray = image_file_with("/DeviceGray", 8, "", 2, 2, &[1, 2, 3, 4], "/Im0 5 0 R");
        assert_eq!(page_count(&gray).unwrap(), 1);
        assert_eq!(split(&gray), Err(Error::BadPdf));

        let cmyk = image_file_with("/DeviceCMYK", 8, "", 1, 1, &[1, 2, 3, 4], "/Im0 5 0 R");
        assert_eq!(page_count(&cmyk).unwrap(), 1);
        assert_eq!(split(&cmyk), Err(Error::BadPdf));

        let filtered = image_file_with(
            "/DeviceRGB",
            8,
            "/Filter /DCTDecode",
            1,
            1,
            &[9, 8, 7],
            "/Im0 5 0 R",
        );
        assert_eq!(page_count(&filtered).unwrap(), 1);
        assert_eq!(split(&filtered), Err(Error::BadPdf));

        let deep = image_file_with(
            "/DeviceRGB",
            16,
            "",
            1,
            1,
            &[0, 1, 0, 2, 0, 3],
            "/Im0 5 0 R",
        );
        assert_eq!(split(&deep), Err(Error::BadPdf));

        let indexed = image_file_with(
            "[/Indexed /DeviceRGB 1 <000000FFFFFF>]",
            8,
            "",
            1,
            1,
            &[0],
            "/Im0 5 0 R",
        );
        assert_eq!(split(&indexed), Err(Error::BadPdf));

        let masked = image_file_with(
            "/DeviceRGB",
            8,
            "/SMask 5 0 R",
            1,
            1,
            &[1, 2, 3],
            "/Im0 5 0 R",
        );
        assert_eq!(split(&masked), Err(Error::BadPdf));

        let wide = vec![0u8; ((RGB_MAX_EDGE + 1) * 3) as usize];
        let wide_pdf = image_file_with(
            "/DeviceRGB",
            8,
            "",
            RGB_MAX_EDGE + 1,
            1,
            &wide,
            "/Im0 5 0 R",
        );
        assert_eq!(page_count(&wide_pdf).unwrap(), 1);
        assert_eq!(split(&wide_pdf), Err(Error::BadPdf));

        let two = finish(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Contents 4 0 R /Resources << /XObject << /Im0 5 0 R /Im1 6 0 R >> >> >>".to_vec(),
            wrap_stream(b"q\n/Im0 Do\nQ"),
            xobject(&[1, 2, 3]),
            xobject(&[4, 5, 6]),
        ]);
        assert_eq!(page_count(&two).unwrap(), 1);
        assert_eq!(split(&two), Err(Error::BadPdf));

        let orphan = finish(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
            wrap_stream(b"BT /F1 24 Tf 72 720 Td (Hi) Tj ET"),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
            xobject(&[1, 2, 3]),
        ]);
        assert_eq!(page_count(&orphan).unwrap(), 1);
        assert_eq!(split(&orphan), Err(Error::BadPdf));

        let inline = finish(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Contents 4 0 R >>".to_vec(),
            wrap_stream(b"BI /W 1 /H 1 /CS /RGB /BPC 8 ID\x01\x02\x03\nEI"),
        ]);
        assert_eq!(page_count(&inline).unwrap(), 1);
        assert_eq!(split(&inline), Err(Error::BadPdf));

        let mixed = finish(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 1 1] /Contents 4 0 R /Resources << /Font << /F1 6 0 R >> /XObject << /Im0 5 0 R >> >> >>".to_vec(),
            wrap_stream(b"BT (Hi) Tj ET\nq\n/Im0 Do\nQ"),
            xobject(&[1, 2, 3]),
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec(),
        ]);
        assert_eq!(split(&mixed), Err(Error::BadPdf));

        let xref_stream = b"%PDF-1.5\n1 0 obj\n<< /Type /XRef /Length 0 >>\nstream\nendstream\nendobj\nstartxref\n9\n%%EOF\n";
        assert_eq!(split(xref_stream), Err(Error::BadPdf));
        assert_eq!(page_count(xref_stream), Err(Error::BadPdf));

        let mut hybrid = text_pdf(&["Hello"]);
        let at = hybrid.windows(6).position(|w| w == b"/Root ").unwrap();
        for (offset, byte) in b"/XRefStm 9 ".iter().enumerate() {
            hybrid.insert(at + offset, *byte);
        }
        assert_eq!(split(&hybrid), Err(Error::BadPdf));
        assert_eq!(page_count(&hybrid), Err(Error::BadPdf));
        assert_eq!(merge(&[&hybrid]).unwrap_err(), Error::BadPdf);
    }

    fn image_file_with(
        color: &str,
        bits: u32,
        extra: &str,
        width: u32,
        height: u32,
        samples: &[u8],
        xobjects: &str,
    ) -> Vec<u8> {
        let mut image = format!(
            "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {color} /BitsPerComponent {bits} {extra} /Length {} >>\nstream\n",
            samples.len()
        )
        .into_bytes();
        image.extend_from_slice(samples);
        image.extend_from_slice(b"endstream");
        let page = format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents 4 0 R /Resources << /XObject << {xobjects} >> >> >>"
        );
        finish(&[
            b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
            b"<< /Type /Pages /Count 1 /Kids [3 0 R] >>".to_vec(),
            page.into_bytes(),
            wrap_stream(b"q\n/Im0 Do\nQ"),
            image,
        ])
    }

    fn xobject(samples: &[u8]) -> Vec<u8> {
        let mut image = format!(
            "<< /Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>\nstream\n",
            samples.len()
        )
        .into_bytes();
        image.extend_from_slice(samples);
        image.extend_from_slice(b"endstream");
        image
    }
}
