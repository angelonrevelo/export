//! Word 97 to HTML when `WordDocument` holds one 8-bit paragraph.
//! Any other file stays a miss. LibreOffice and pandoc are not called.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::docx::CountWrite;
use crate::error::{Error, Result};
use crate::stats::RunStats;

const MAX_INPUT: u64 = 32 * 1024 * 1024;
const MAX_PARA: usize = 32 * 1024;
const SECTOR: usize = 512;
const MINI: usize = 64;
const MINI_CUTOFF: u32 = 4096;
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FATSECT: u32 = 0xFFFF_FFFD;
const FREESECT: u32 = 0xFFFF_FFFF;
const NOSTREAM: u32 = 0xFFFF_FFFF;
const FIB_MAGIC: u16 = 0xA5EC;
const NFIB_WORD97: u16 = 0x00C1;
const FC_MIN: usize = 0x40;
/// fComplex, fHasPic, cQuickSaves, fEncrypted, fExtChar, fObfuscated.
const FIB_REJECT: u16 = 0x91FC;

const SIG: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];

pub fn doc_to_html<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    let input_bytes = match src.seek(SeekFrom::End(0)) {
        Ok(n) => n,
        Err(err) => return Err(miss(0, &err.to_string())),
    };
    if input_bytes > MAX_INPUT {
        return Err(miss(input_bytes, "file is not one paragraph"));
    }
    if let Err(err) = src.seek(SeekFrom::Start(0)) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    let mut bytes = Vec::new();
    if let Err(err) = src.read_to_end(&mut bytes) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    let text = parse_doc(&bytes).map_err(|err| {
        if err.message.starts_with("MISS:") {
            err
        } else {
            miss(input_bytes, &err.message)
        }
    })?;
    let html = html_of(&text);
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

/// Compound file whose `WordDocument` text is `paragraphs`, each ended by a paragraph mark.
/// The converter accepts only a single plain paragraph.
pub fn word97_document(paragraphs: &[&str]) -> Result<Vec<u8>> {
    if paragraphs.is_empty() {
        return Err(Error::new("need a paragraph"));
    }
    let mut text = Vec::new();
    for para in paragraphs {
        if para.is_empty() || !para.bytes().all(allowed_text) {
            return Err(Error::new("paragraph is not plain text"));
        }
        text.extend_from_slice(para.as_bytes());
        text.push(0x0D);
    }
    if text.len() > MAX_PARA {
        return Err(Error::new("paragraph is too large"));
    }
    let word = word_document(&text);
    pack_cfb(&word)
}

fn miss(input_bytes: u64, why: &str) -> Error {
    Error::new(format!("MISS: doc {why} (input_b={input_bytes})"))
}

fn shape() -> Error {
    Error::new("file is not one paragraph")
}

fn allowed_text(b: u8) -> bool {
    b == 0x09 || (0x20..=0x7E).contains(&b)
}

fn html_of(text: &str) -> String {
    let mut out =
        String::from("<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body><p>");
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out.push_str("</p></body></html>");
    out
}

fn word_document(text_with_mark: &[u8]) -> Vec<u8> {
    let fc_min = FC_MIN as u32;
    let fc_mac = fc_min + text_with_mark.len() as u32;
    let mut buf = vec![0u8; fc_mac as usize];
    put_u16(&mut buf, 0, FIB_MAGIC);
    put_u16(&mut buf, 2, NFIB_WORD97);
    put_u16(&mut buf, 0x0C, 0x00BF);
    put_u32(&mut buf, 0x18, fc_min);
    put_u32(&mut buf, 0x1C, fc_mac);
    buf[FC_MIN..].copy_from_slice(text_with_mark);
    buf
}

fn pack_cfb(word: &[u8]) -> Result<Vec<u8>> {
    if word.len() < MINI_CUTOFF as usize {
        pack_mini(word)
    } else {
        pack_regular(word)
    }
}

fn pack_regular(word: &[u8]) -> Result<Vec<u8>> {
    let nsec = word.len().div_ceil(SECTOR);
    if nsec + 2 > 128 {
        return Err(Error::new("paragraph is too large"));
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
    file.extend_from_slice(&dir_sector(ENDOFCHAIN, 0, 2, word.len() as u32));
    file.extend_from_slice(word);
    file.resize(file.len() + nsec * SECTOR - word.len(), 0);
    Ok(file)
}

fn pack_mini(word: &[u8]) -> Result<Vec<u8>> {
    let mini_count = word.len().div_ceil(MINI);
    let mini_bytes = mini_count * MINI;
    let data_secs = mini_bytes.div_ceil(SECTOR);
    if data_secs + 3 > 128 {
        return Err(Error::new("paragraph is too large"));
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
    file.extend_from_slice(&dir_sector(3, mini_bytes as u32, 0, word.len() as u32));
    file.extend_from_slice(&mini_fat);
    file.extend_from_slice(word);
    file.resize(file.len() + data_secs * SECTOR - word.len(), 0);
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

struct DirSpec<'a> {
    name: &'a str,
    kind: u8,
    child: u32,
    start: u32,
    size: u32,
}

fn dir_sector(root_start: u32, root_size: u32, word_start: u32, word_size: u32) -> [u8; SECTOR] {
    let mut dir = [0u8; SECTOR];
    let root = dir_entry(&DirSpec {
        name: "Root Entry",
        kind: 5,
        child: 1,
        start: root_start,
        size: root_size,
    });
    let word = dir_entry(&DirSpec {
        name: "WordDocument",
        kind: 2,
        child: NOSTREAM,
        start: word_start,
        size: word_size,
    });
    dir[..128].copy_from_slice(&root);
    dir[128..256].copy_from_slice(&word);
    dir
}

fn dir_entry(spec: &DirSpec<'_>) -> [u8; 128] {
    let mut e = [0u8; 128];
    let mut at = 0;
    for unit in spec.name.encode_utf16() {
        put_u16(&mut e, at, unit);
        at += 2;
    }
    put_u16(&mut e, 0x40, (at + 2) as u16);
    e[0x42] = spec.kind;
    e[0x43] = 1;
    put_u32(&mut e, 0x44, NOSTREAM);
    put_u32(&mut e, 0x48, NOSTREAM);
    put_u32(&mut e, 0x4C, spec.child);
    put_u32(&mut e, 0x74, spec.start);
    put_u32(&mut e, 0x78, spec.size);
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
        return Err(shape());
    }
    Ok(u16::from_le_bytes([buf[at], buf[at + 1]]))
}

fn get_u32(buf: &[u8], at: usize) -> Result<u32> {
    if at + 4 > buf.len() {
        return Err(shape());
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

fn parse_doc(file: &[u8]) -> Result<String> {
    if file.len() < SECTOR || file[..8] != SIG {
        return Err(shape());
    }
    let major = get_u16(file, 0x1A)?;
    let order = get_u16(file, 0x1C)?;
    let sec_shift = get_u16(file, 0x1E)?;
    let mini_shift = get_u16(file, 0x20)?;
    if major != 3 || order != 0xFFFE || sec_shift != 9 || mini_shift != 6 {
        return Err(shape());
    }
    let csect_fat = get_u32(file, 0x2C)?;
    let dir_start = get_u32(file, 0x30)?;
    let cutoff = get_u32(file, 0x38)?;
    let mini_fat_start = get_u32(file, 0x3C)?;
    let csect_mini = get_u32(file, 0x40)?;
    let csect_dif = get_u32(file, 0x48)?;
    if cutoff != MINI_CUTOFF
        || csect_dif != 0
        || csect_fat == 0
        || csect_fat > 109
        || csect_mini > 8
    {
        return Err(shape());
    }
    let fat = load_fat(file, csect_fat)?;
    let dir = read_chain(file, &fat, dir_start, None)?;
    if dir.len() % 128 != 0 {
        return Err(shape());
    }
    let (root, word) = pick_streams(&dir)?;
    if word.size == 0 || word.size as u64 > MAX_INPUT {
        return Err(shape());
    }
    let bytes = if word.size < MINI_CUTOFF {
        if csect_mini == 0 || root.size == 0 {
            return Err(shape());
        }
        let mini_fat_bytes = read_chain(
            file,
            &fat,
            mini_fat_start,
            Some(csect_mini as usize * SECTOR),
        )?;
        let mini_fat = words_of(&mini_fat_bytes)?;
        let mini_stream = read_chain(file, &fat, root.start, Some(root.size as usize))?;
        read_mini(&mini_fat, &mini_stream, word.start, word.size as usize)?
    } else {
        read_chain(file, &fat, word.start, Some(word.size as usize))?
    };
    one_paragraph(&bytes)
}

fn load_fat(file: &[u8], csect_fat: u32) -> Result<Vec<u32>> {
    let mut fat = Vec::new();
    for i in 0..csect_fat as usize {
        let sid = get_u32(file, 0x4C + i * 4)?;
        if sid >= 0xFFFF_FFFA {
            return Err(shape());
        }
        let sector = sector(file, sid)?;
        fat.extend(words_of(sector)?);
    }
    Ok(fat)
}

fn words_of(bytes: &[u8]) -> Result<Vec<u32>> {
    if bytes.len() % 4 != 0 {
        return Err(shape());
    }
    let mut out = Vec::with_capacity(bytes.len() / 4);
    for chunk in bytes.chunks(4) {
        out.push(u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }
    Ok(out)
}

fn pick_streams(dir: &[u8]) -> Result<(StreamLoc, StreamLoc)> {
    let mut root = None;
    let mut word = None;
    for entry in dir.chunks(128) {
        let kind = entry[0x42];
        if kind == 0 {
            continue;
        }
        if entry[0x7C..0x80] != [0, 0, 0, 0] {
            return Err(shape());
        }
        let loc = StreamLoc {
            start: get_u32(entry, 0x74)?,
            size: get_u32(entry, 0x78)?,
        };
        match kind {
            5 => {
                if root.is_some() {
                    return Err(shape());
                }
                root = Some(loc);
            }
            2 => {
                let name = entry_name(entry)?;
                if name != "WordDocument" || word.is_some() {
                    return Err(shape());
                }
                word = Some(loc);
            }
            _ => return Err(shape()),
        }
    }
    match (root, word) {
        (Some(root), Some(word)) => Ok((root, word)),
        _ => Err(shape()),
    }
}

fn entry_name(entry: &[u8]) -> Result<String> {
    let n = get_u16(entry, 0x40)? as usize;
    if !(2..=64).contains(&n) || n % 2 != 0 {
        return Err(shape());
    }
    let chars = n / 2 - 1;
    let mut name = String::new();
    for i in 0..chars {
        let unit = get_u16(entry, i * 2)?;
        if unit > 0x7F {
            return Err(shape());
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
            return Err(shape());
        }
        guard += 1;
        let chunk = sector(file, sid)?;
        out.extend_from_slice(chunk);
        sid = fat[sid as usize];
    }
    if let Some(size) = size {
        if out.len() < size {
            return Err(shape());
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
            return Err(shape());
        }
        guard += 1;
        let off = sid as usize * MINI;
        if off >= mini_stream.len() {
            return Err(shape());
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
        .ok_or_else(shape)?;
    if off + SECTOR > file.len() {
        return Err(shape());
    }
    Ok(&file[off..off + SECTOR])
}

fn one_paragraph(word: &[u8]) -> Result<String> {
    if word.len() < 0x20 {
        return Err(shape());
    }
    if get_u16(word, 0)? != FIB_MAGIC || get_u16(word, 2)? != NFIB_WORD97 {
        return Err(shape());
    }
    if get_u16(word, 0x0A)? & FIB_REJECT != 0 {
        return Err(shape());
    }
    let fc_min = get_u32(word, 0x18)? as usize;
    let fc_mac = get_u32(word, 0x1C)? as usize;
    if fc_min >= fc_mac || fc_mac > word.len() {
        return Err(shape());
    }
    let span = &word[fc_min..fc_mac];
    if span.len() < 2 || span.len() > MAX_PARA || span[span.len() - 1] != 0x0D {
        return Err(shape());
    }
    let body = &span[..span.len() - 1];
    if body.is_empty() || !body.iter().copied().all(allowed_text) {
        return Err(shape());
    }
    let mut text = String::with_capacity(body.len());
    for &b in body {
        text.push(char::from(b));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn run(input: &[u8]) -> Result<Vec<u8>> {
        let mut cur = Cursor::new(input);
        let mut out = Vec::new();
        doc_to_html(&mut cur, &mut out)?;
        Ok(out)
    }

    #[test]
    fn one_paragraph_becomes_html() {
        let doc = word97_document(&["Fish & chips"]).unwrap();
        let html = String::from_utf8(run(&doc).unwrap()).unwrap();
        assert_eq!(
            html,
            "<!DOCTYPE html><html><head><meta charset=\"utf-8\"></head><body><p>Fish &amp; chips</p></body></html>"
        );
        let wide = "M".repeat(1000);
        let mid = word97_document(&[&wide]).unwrap();
        let html = String::from_utf8(run(&mid).unwrap()).unwrap();
        assert!(html.contains(&wide), "{html}");
    }

    #[test]
    fn long_paragraph_uses_the_fat() {
        let mut para = "A ".repeat(2100);
        para.push_str("ENDMARK");
        let doc = word97_document(&[&para]).unwrap();
        assert!(para.len() + FC_MIN >= MINI_CUTOFF as usize);
        let html = String::from_utf8(run(&doc).unwrap()).unwrap();
        assert!(html.contains("ENDMARK"), "{html}");
        assert!(html.contains("A A"), "{html}");
    }

    #[test]
    fn two_paragraphs_stay_a_miss() {
        let doc = word97_document(&["One", "Two"]).unwrap();
        let err = run(&doc).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{}", err.message);
        assert!(err.message.contains("not"), "{}", err.message);
        assert!(
            err.message.contains(&format!("input_b={}", doc.len())),
            "{}",
            err.message
        );
    }

    #[test]
    fn other_shapes_stay_a_miss() {
        let err = run(b"not a doc").unwrap_err();
        assert!(err.message.contains("input_b=9"), "{}", err.message);
        let mut doc = word97_document(&["Hi"]).unwrap();
        let pos = doc.windows(2).position(|w| w == [0xEC, 0xA5]).unwrap();
        doc[pos + 2] = 0;
        assert!(run(&doc).unwrap_err().message.starts_with("MISS:"));
        let mut doc = word97_document(&["Hi"]).unwrap();
        let pos = doc.windows(2).position(|w| w == [0xEC, 0xA5]).unwrap();
        doc[pos + 0x0B] = 0x01;
        assert!(run(&doc).unwrap_err().message.starts_with("MISS:"));
        let doc = word97_document(&["Hi"]).unwrap();
        let err = crate::convert::convert("doc", "pdf", &doc).unwrap_err();
        assert!(err.message.contains("no reader"), "{err}");
    }
}
