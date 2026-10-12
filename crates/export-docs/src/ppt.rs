//! Legacy PowerPoint 97 to HTML for one generated compound file.
//! The `PowerPoint Document` stream holds one, two, or three text-bytes shapes.
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

pub fn ppt_to_html<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
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
    let texts = parse_ppt(&bytes).map_err(|err| {
        if err.message.starts_with("MISS:") {
            err
        } else {
            miss(input_bytes, &err.message)
        }
    })?;
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
    Error::new("file is not one, two, or three text shapes")
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
                if pending_header || texts.len() >= 3 || rec.ver != 0 || rec.data.len() != 4 {
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
    fn four_text_shapes_stay_a_miss() {
        let stream = document_stream(&[
            b"One shape.",
            b"Two shape.",
            b"Three shape.",
            b"Four shape.",
        ]);
        let file = pack_cfb(&stream).unwrap();
        let err = html(&file).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("not one, two, or three text shapes"),
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
