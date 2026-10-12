//! TrueType / OpenType container copy.
//!
//! Tables are copied. The scaler type is the container: `0x00010000` for TTF
//! and `OTTO` for OTF. `head.checkSumAdjustment` is recomputed so the file
//! sums to `0xB1B0AFBA`. Glyph outlines are not rewritten.

use crate::error::{Error, Result};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

const TTF_SCALER: u32 = 0x0001_0000;
const OTF_SCALER: u32 = 0x4F54_544F;
const MASTER: u32 = 0xB1B0_AFBA;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontContainer {
    Ttf,
    Otf,
}

pub fn tiny_ttf() -> Vec<u8> {
    let glyf0: Vec<u8> = {
        let mut g = Vec::new();
        g.extend_from_slice(&0i16.to_be_bytes());
        for _ in 0..4 {
            g.extend_from_slice(&0i16.to_be_bytes());
        }
        g
    };
    let glyf1 = square_glyph();
    let mut glyf = Vec::new();
    glyf.extend_from_slice(&glyf0);
    let g1 = glyf.len() as u32;
    glyf.extend_from_slice(&glyf1);
    let gend = glyf.len() as u32;
    let loca = {
        let mut l = Vec::new();
        for offset in [0u32, g1, gend] {
            l.extend_from_slice(&offset.to_be_bytes());
        }
        l
    };
    let tables: [(&[u8; 4], Vec<u8>); 9] = [
        (b"cmap", cmap_table()),
        (b"glyf", glyf),
        (b"head", head_table()),
        (b"hhea", hhea_table()),
        (b"hmtx", hmtx_table()),
        (b"loca", loca),
        (b"maxp", maxp_table()),
        (b"name", name_table()),
        (b"post", post_table()),
    ];
    let refs: Vec<(&[u8; 4], &[u8])> = tables
        .iter()
        .map(|(tag, data)| (*tag, data.as_slice()))
        .collect();
    write_sfnt(TTF_SCALER, &refs)
}

pub fn convert_font(src: &[u8], to: FontContainer) -> Result<Vec<u8>> {
    let (scaler, tables) = parse_sfnt(src)?;
    let target = match to {
        FontContainer::Ttf => TTF_SCALER,
        FontContainer::Otf => OTF_SCALER,
    };
    if scaler != TTF_SCALER && scaler != OTF_SCALER {
        return Err(Error::Format("font scaler"));
    }
    let refs: Vec<(&[u8; 4], &[u8])> = tables
        .iter()
        .map(|(tag, data)| (tag, data.as_slice()))
        .collect();
    Ok(write_sfnt(target, &refs))
}

pub fn font_container(src: &[u8]) -> Result<FontContainer> {
    let (scaler, _) = parse_sfnt(src)?;
    match scaler {
        TTF_SCALER => Ok(FontContainer::Ttf),
        OTF_SCALER => Ok(FontContainer::Otf),
        _ => Err(Error::Format("font scaler")),
    }
}

pub fn font_tables(src: &[u8]) -> Result<Vec<([u8; 4], Vec<u8>)>> {
    Ok(parse_sfnt(src)?.1)
}

/// Publish `src` converted to `to` by renaming `path.tmp` onto `path`.
pub fn convert_font_file(src: &[u8], to: FontContainer, path: impl AsRef<Path>) -> Result<()> {
    let bytes = convert_font(src, to)?;
    let path = path.as_ref();
    let tmp = sibling_tmp(path);
    let mut file = File::create(&tmp)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    fs::rename(&tmp, path).map_err(|err| {
        let _ = fs::remove_file(&tmp);
        Error::from(err)
    })?;
    Ok(())
}

type SfntTables = Vec<([u8; 4], Vec<u8>)>;

fn parse_sfnt(src: &[u8]) -> Result<(u32, SfntTables)> {
    if src.len() < 12 {
        return Err(Error::Format("truncated"));
    }
    let scaler = u32_be(&src[0..4]);
    if scaler != TTF_SCALER && scaler != OTF_SCALER {
        return Err(Error::Format("font scaler"));
    }
    let num = u16_be(&src[4..6]) as usize;
    if num == 0 || 12 + num * 16 > src.len() {
        return Err(Error::Format("font directory"));
    }
    let mut tables = Vec::with_capacity(num);
    let mut prev_tag = [0u8; 4];
    for i in 0..num {
        let rec = 12 + i * 16;
        let mut tag = [0u8; 4];
        tag.copy_from_slice(&src[rec..rec + 4]);
        if i > 0 && tag <= prev_tag {
            return Err(Error::Format("font directory"));
        }
        prev_tag = tag;
        let checksum = u32_be(&src[rec + 4..rec + 8]);
        let offset = u32_be(&src[rec + 8..rec + 12]) as usize;
        let length = u32_be(&src[rec + 12..rec + 16]) as usize;
        let end = offset
            .checked_add(length)
            .ok_or(Error::Format("truncated"))?;
        if end > src.len() {
            return Err(Error::Format("truncated"));
        }
        let body = src[offset..end].to_vec();
        let sum = if &tag == b"head" {
            checksum_head(&body)?
        } else {
            table_sum(&body)
        };
        if sum != checksum {
            return Err(Error::ChecksumMismatch);
        }
        tables.push((tag, body));
    }
    if master_sum(src) != MASTER {
        return Err(Error::ChecksumMismatch);
    }
    Ok((scaler, tables))
}

fn checksum_head(body: &[u8]) -> Result<u32> {
    if body.len() < 12 {
        return Err(Error::Format("font head"));
    }
    let mut zeroed = body.to_vec();
    zeroed[8..12].fill(0);
    Ok(table_sum(&zeroed))
}

fn write_sfnt(scaler: u32, tables: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
    let n = tables.len() as u16;
    let (search, entry, range) = search_fields(n);
    let mut header = Vec::with_capacity(12 + tables.len() * 16);
    header.extend_from_slice(&scaler.to_be_bytes());
    header.extend_from_slice(&n.to_be_bytes());
    header.extend_from_slice(&search.to_be_bytes());
    header.extend_from_slice(&entry.to_be_bytes());
    header.extend_from_slice(&range.to_be_bytes());
    let mut offset = (header.len() + tables.len() * 16) as u32;
    let mut records = Vec::new();
    let mut blobs = Vec::new();
    for (tag, data) in tables {
        let sum = if *tag == b"head" {
            let mut zeroed = data.to_vec();
            if zeroed.len() >= 12 {
                zeroed[8..12].fill(0);
            }
            table_sum(&zeroed)
        } else {
            table_sum(data)
        };
        let mut rec = Vec::with_capacity(16);
        rec.extend_from_slice(*tag);
        rec.extend_from_slice(&sum.to_be_bytes());
        rec.extend_from_slice(&offset.to_be_bytes());
        rec.extend_from_slice(&(data.len() as u32).to_be_bytes());
        records.extend(rec);
        blobs.extend_from_slice(data);
        let pad = (4 - (data.len() % 4)) % 4;
        blobs.extend(std::iter::repeat(0).take(pad));
        offset += data.len() as u32 + pad as u32;
    }
    let mut file = header;
    file.extend(records);
    let head_at = tables
        .iter()
        .position(|(tag, _)| *tag == b"head")
        .map(|index| {
            let mut at = (12 + tables.len() * 16) as u32;
            for (tag, data) in tables.iter().take(index) {
                let _ = tag;
                let pad = (4 - (data.len() % 4)) % 4;
                at += data.len() as u32 + pad as u32;
            }
            at as usize
        });
    file.extend(blobs);
    if let Some(at) = head_at {
        if at + 12 <= file.len() {
            file[at + 8..at + 12].fill(0);
            let sum = master_sum(&file);
            let adjust = MASTER.wrapping_sub(sum);
            file[at + 8..at + 12].copy_from_slice(&adjust.to_be_bytes());
        }
    }
    file
}

fn master_sum(data: &[u8]) -> u32 {
    table_sum(data)
}

fn table_sum(data: &[u8]) -> u32 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 4 <= data.len() {
        sum = sum.wrapping_add(u32_be(&data[i..i + 4]));
        i += 4;
    }
    if i < data.len() {
        let mut pad = [0u8; 4];
        pad[..data.len() - i].copy_from_slice(&data[i..]);
        sum = sum.wrapping_add(u32::from_be_bytes(pad));
    }
    sum
}

fn search_fields(n: u16) -> (u16, u16, u16) {
    let mut pow = 1u16;
    let mut exp = 0u16;
    while pow.saturating_mul(2) <= n {
        pow *= 2;
        exp += 1;
    }
    let search = pow.saturating_mul(16);
    let range = n.saturating_mul(16).saturating_sub(search);
    (search, exp, range)
}

fn head_table() -> Vec<u8> {
    let mut t = vec![0u8; 54];
    put_u32(&mut t[0..4], 0x0001_0000);
    put_u32(&mut t[4..8], 0x0001_0000);
    put_u32(&mut t[12..16], 0x5F0F_3CF5);
    put_u16(&mut t[16..18], 0x000B);
    put_u16(&mut t[18..20], 1000);
    put_u16(&mut t[36..38], 0);
    put_u16(&mut t[40..42], 500);
    put_u16(&mut t[42..44], 500);
    put_u16(&mut t[46..48], 8);
    put_u16(&mut t[48..50], 2);
    put_u16(&mut t[50..52], 1);
    t
}

fn hhea_table() -> Vec<u8> {
    let mut t = vec![0u8; 36];
    put_u32(&mut t[0..4], 0x0001_0000);
    put_i16(&mut t[4..6], 800);
    put_i16(&mut t[6..8], -200);
    put_u16(&mut t[10..12], 500);
    put_u16(&mut t[16..18], 500);
    put_i16(&mut t[18..20], 1);
    put_u16(&mut t[34..36], 2);
    t
}

fn maxp_table() -> Vec<u8> {
    let mut t = vec![0u8; 32];
    put_u32(&mut t[0..4], 0x0001_0000);
    put_u16(&mut t[4..6], 2);
    put_u16(&mut t[6..8], 4);
    put_u16(&mut t[8..10], 1);
    put_u16(&mut t[14..16], 2);
    t
}

fn hmtx_table() -> Vec<u8> {
    let mut t = Vec::new();
    for _ in 0..2 {
        t.extend_from_slice(&500u16.to_be_bytes());
        t.extend_from_slice(&0i16.to_be_bytes());
    }
    t
}

fn post_table() -> Vec<u8> {
    let mut t = vec![0u8; 32];
    put_u32(&mut t[0..4], 0x0003_0000);
    put_i16(&mut t[8..10], -100);
    put_i16(&mut t[10..12], 50);
    t
}

fn name_table() -> Vec<u8> {
    let utf16 = utf16_be("ExportPack");
    let mut t = Vec::new();
    t.extend_from_slice(&0u16.to_be_bytes());
    t.extend_from_slice(&3u16.to_be_bytes());
    t.extend_from_slice(&42u16.to_be_bytes());
    for name_id in [1u16, 4, 6] {
        t.extend_from_slice(&3u16.to_be_bytes());
        t.extend_from_slice(&1u16.to_be_bytes());
        t.extend_from_slice(&0x0409u16.to_be_bytes());
        t.extend_from_slice(&name_id.to_be_bytes());
        t.extend_from_slice(&(utf16.len() as u16).to_be_bytes());
        t.extend_from_slice(&0u16.to_be_bytes());
    }
    t.extend_from_slice(&utf16);
    t
}

fn cmap_table() -> Vec<u8> {
    // format 4: U+0000 -> glyph 0, U+0041 -> glyph 1, sentinel.
    let mut format4 = Vec::new();
    format4.extend_from_slice(&4u16.to_be_bytes());
    let length_at = format4.len();
    format4.extend_from_slice(&0u16.to_be_bytes());
    format4.extend_from_slice(&0u16.to_be_bytes());
    format4.extend_from_slice(&6u16.to_be_bytes()); // segCountX2
    format4.extend_from_slice(&4u16.to_be_bytes()); // searchRange
    format4.extend_from_slice(&1u16.to_be_bytes()); // entrySelector
    format4.extend_from_slice(&2u16.to_be_bytes()); // rangeShift
    for end in [0u16, 0x0041, 0xFFFF] {
        format4.extend_from_slice(&end.to_be_bytes());
    }
    format4.extend_from_slice(&0u16.to_be_bytes());
    for start in [0u16, 0x0041, 0xFFFF] {
        format4.extend_from_slice(&start.to_be_bytes());
    }
    // idDelta: 0, 1 - 0x41, 1
    for delta in [0i16, 1 - 0x41, 1] {
        format4.extend_from_slice(&delta.to_be_bytes());
    }
    for _ in 0..3 {
        format4.extend_from_slice(&0u16.to_be_bytes());
    }
    let len = format4.len() as u16;
    format4[length_at..length_at + 2].copy_from_slice(&len.to_be_bytes());

    let mut cmap = Vec::new();
    cmap.extend_from_slice(&0u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&3u16.to_be_bytes());
    cmap.extend_from_slice(&1u16.to_be_bytes());
    cmap.extend_from_slice(&12u32.to_be_bytes());
    cmap.extend(format4);
    cmap
}

fn square_glyph() -> Vec<u8> {
    let mut g = Vec::new();
    g.extend_from_slice(&1i16.to_be_bytes());
    for v in [0i16, 0, 500, 500] {
        g.extend_from_slice(&v.to_be_bytes());
    }
    g.extend_from_slice(&3u16.to_be_bytes());
    g.extend_from_slice(&0u16.to_be_bytes());
    g.extend_from_slice(&[0x01, 0x01, 0x01, 0x01]);
    for x in [0i16, 500, 0, -500] {
        g.extend_from_slice(&x.to_be_bytes());
    }
    for y in [0i16, 0, 500, 0] {
        g.extend_from_slice(&y.to_be_bytes());
    }
    g
}

fn utf16_be(text: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

fn u16_be(b: &[u8]) -> u16 {
    u16::from_be_bytes([b[0], b[1]])
}

fn u32_be(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn put_u16(dst: &mut [u8], value: u16) {
    dst[..2].copy_from_slice(&value.to_be_bytes());
}

fn put_i16(dst: &mut [u8], value: i16) {
    dst[..2].copy_from_slice(&value.to_be_bytes());
}

fn put_u32(dst: &mut [u8], value: u32) {
    dst[..4].copy_from_slice(&value.to_be_bytes());
}

fn sibling_tmp(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".tmp");
    PathBuf::from(os)
}
