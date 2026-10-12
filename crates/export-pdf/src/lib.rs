//! Classic-xref text PDFs. Split and merge copy page text into a new file.
//!
//! Honest limit: xref streams, object streams, images, and fonts other than
//! the built-in Helvetica are not preserved. A truncated file or an xref
//! offset that does not land on an object is an error.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadPdf,
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
    objects.push(
        format!("<< /Type /Pages /Count {n} /Kids {kids} >>").into_bytes(),
    );
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
    let start_num: u32 = parts.next().ok_or(Error::BadPdf)?.parse().map_err(|_| Error::BadPdf)?;
    let count: usize = parts.next().ok_or(Error::BadPdf)?.parse().map_err(|_| Error::BadPdf)?;
    let mut objects = Vec::new();
    for i in 0..count {
        let line = lines.next().ok_or(Error::Truncated)?;
        let mut cols = line.split_whitespace();
        let offset: usize = cols.next().ok_or(Error::BadPdf)?.parse().map_err(|_| Error::BadPdf)?;
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
        let end = pdf[body_at..]
            .windows(6)
            .position(|w| w == b"endobj")
            .ok_or(Error::Truncated)?;
        objects.push((num, &pdf[body_at..body_at + end]));
    }
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

#[cfg(test)]
mod tests {
    use super::{merge, page_count, split, text_pdf, Error};

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
        assert_eq!(texts, vec!["Second (line)".to_string(), "Hello".to_string()]);
    }

    #[test]
    fn truncated_and_shifted_xref_error() {
        assert_eq!(page_count(b"%PDF-1.4\nno end"), Err(Error::Truncated));
        let mut pdf = text_pdf(&["Hello"]);
        let at = pdf.windows(7).position(|w| w == b"1 0 obj").unwrap();
        pdf[at] = b'9';
        assert_eq!(page_count(&pdf), Err(Error::BadPdf));
    }
}
