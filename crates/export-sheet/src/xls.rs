//! One generated BIFF8 workbook.
//!
//! The file is a version-3 compound document whose `Workbook` stream holds
//! BIFF8 records: a globals substream, one worksheet, one IEEE number, and
//! one shared string. A second generated file adds another IEEE number in
//! C1 and leaves B1 as that same string. Anything that is not that layout
//! is refused. There is
//! no writer for arbitrary sheets, and no guess for a record this parser
//! does not know how to read when that record is one of the ones below.

use std::io::{BufRead, Write};

use crate::delim;
use crate::error::{Error, Result};
use crate::Stats;

const MAX_FILE: usize = 64 * 1024 * 1024;
const OLE_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const ENDOFCHAIN: u32 = 0xFFFF_FFFE;
const FREESECT: u32 = 0xFFFF_FFFF;
const FATSECT: u32 = 0xFFFF_FFFD;
const NOSTREAM: u32 = 0xFFFF_FFFF;

const NUMBER_VALUE: f64 = 42.0;
const TEXT_VALUE: &str = "cat";

/// The one `.xls` this crate reads: BIFF8, cell A1 = 42, cell B1 = `cat`.
pub fn generated_biff_xls() -> Vec<u8> {
    wrap_ole(&workbook_bytes())
}

/// Same BIFF8 layout as [`generated_biff_xls`], with a second number in C1.
/// A1 stays 42 and B1 stays `cat`.
pub fn generated_biff_xls_two_numbers() -> Vec<u8> {
    wrap_ole(&workbook_two_numbers())
}

pub(crate) fn to_delim<R: BufRead, W: Write>(
    mut input: R,
    output: &mut W,
    delim: u8,
) -> Result<Stats> {
    let mut raw = Vec::new();
    input.read_to_end(&mut raw)?;
    if raw.len() > MAX_FILE {
        return Err(Error::new("xls is larger than 64MiB"));
    }
    let rows = read_biff8(&raw)?;
    for row in &rows {
        delim::write_row(output, delim, row.iter().map(String::as_str))?;
    }
    Ok(Stats {
        rows: rows.len() as u64,
        lossy: 0,
    })
}

fn workbook_bytes() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    push_rec(&mut out, 0x00FC, &sst(TEXT_VALUE));
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    // OpenOffice documents 0x04C1 as the usual worksheet WsBool.
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dimensions());
    push_rec(&mut out, 0x0203, &number_cell(0, 0, NUMBER_VALUE));
    push_rec(&mut out, 0x00FD, &labelsst_cell(0, 1, 0));
    push_rec(&mut out, 0x000A, &[]);
    out
}

fn workbook_two_numbers() -> Vec<u8> {
    let mut out = Vec::new();
    push_rec(&mut out, 0x0809, &bof(0x0005));
    push_rec(&mut out, 0x0042, &0x04E4u16.to_le_bytes());
    push_rec(&mut out, 0x00FC, &sst(TEXT_VALUE));
    let boundsheet_at = out.len();
    push_rec(&mut out, 0x0085, &boundsheet(0, "Sheet"));
    push_rec(&mut out, 0x000A, &[]);
    let sheet_at = out.len() as u32;
    out[boundsheet_at + 4..boundsheet_at + 8].copy_from_slice(&sheet_at.to_le_bytes());
    push_rec(&mut out, 0x0809, &bof(0x0010));
    push_rec(&mut out, 0x0081, &0x04C1u16.to_le_bytes());
    let mut dims = dimensions();
    dims[10..12].copy_from_slice(&3u16.to_le_bytes());
    push_rec(&mut out, 0x0200, &dims);
    push_rec(&mut out, 0x0203, &number_cell(0, 0, NUMBER_VALUE));
    push_rec(&mut out, 0x00FD, &labelsst_cell(0, 1, 0));
    push_rec(&mut out, 0x0203, &number_cell(0, 2, 7.0));
    push_rec(&mut out, 0x000A, &[]);
    out
}

fn bof(dt: u16) -> [u8; 16] {
    let mut data = [0u8; 16];
    data[0..2].copy_from_slice(&0x0600u16.to_le_bytes());
    data[2..4].copy_from_slice(&dt.to_le_bytes());
    data[4..6].copy_from_slice(&0x0DBBu16.to_le_bytes());
    data[6..8].copy_from_slice(&0x07CCu16.to_le_bytes());
    data[8..12].copy_from_slice(&0x0000_00C1u32.to_le_bytes());
    data[12..16].copy_from_slice(&0x0000_0006u32.to_le_bytes());
    data
}

fn sst(text: &str) -> Vec<u8> {
    let bytes = text.as_bytes();
    let mut data = Vec::with_capacity(8 + 3 + bytes.len());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&1u32.to_le_bytes());
    data.extend_from_slice(&(bytes.len() as u16).to_le_bytes());
    data.push(0);
    data.extend_from_slice(bytes);
    data
}

fn boundsheet(pos: u32, name: &str) -> Vec<u8> {
    let bytes = name.as_bytes();
    let mut data = Vec::with_capacity(8 + bytes.len());
    data.extend_from_slice(&pos.to_le_bytes());
    // hsState is the low 2 bits. dt (worksheet = 0) is the high byte.
    data.extend_from_slice(&0u16.to_le_bytes());
    data.push(bytes.len() as u8);
    data.push(0);
    data.extend_from_slice(bytes);
    data
}

fn dimensions() -> [u8; 14] {
    let mut data = [0u8; 14];
    data[4..8].copy_from_slice(&1u32.to_le_bytes());
    data[10..12].copy_from_slice(&2u16.to_le_bytes());
    data
}

fn number_cell(row: u16, col: u16, value: f64) -> [u8; 14] {
    let mut data = [0u8; 14];
    data[0..2].copy_from_slice(&row.to_le_bytes());
    data[2..4].copy_from_slice(&col.to_le_bytes());
    data[6..14].copy_from_slice(&value.to_le_bytes());
    data
}

fn labelsst_cell(row: u16, col: u16, index: u32) -> [u8; 10] {
    let mut data = [0u8; 10];
    data[0..2].copy_from_slice(&row.to_le_bytes());
    data[2..4].copy_from_slice(&col.to_le_bytes());
    data[6..10].copy_from_slice(&index.to_le_bytes());
    data
}

fn push_rec(out: &mut Vec<u8>, op: u16, data: &[u8]) {
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(data.len() as u16).to_le_bytes());
    out.extend_from_slice(data);
}

fn wrap_ole(workbook: &[u8]) -> Vec<u8> {
    let mini = 64usize;
    let minis = workbook.len().div_ceil(mini);
    let mini_stream_size = minis * mini;
    let mut file = vec![0u8; 512 * 5];
    file[..8].copy_from_slice(&OLE_MAGIC);
    put_u16(&mut file, 0x18, 0x003E);
    put_u16(&mut file, 0x1A, 0x0003);
    put_u16(&mut file, 0x1C, 0xFFFE);
    put_u16(&mut file, 0x1E, 9);
    put_u16(&mut file, 0x20, 6);
    put_u32(&mut file, 0x2C, 1);
    put_u32(&mut file, 0x30, 1);
    put_u32(&mut file, 0x38, 4096);
    put_u32(&mut file, 0x3C, 2);
    put_u32(&mut file, 0x40, 1);
    put_u32(&mut file, 0x44, ENDOFCHAIN);
    put_u32(&mut file, 0x4C, 0);
    for i in 1..109 {
        put_u32(&mut file, 0x4C + i * 4, FREESECT);
    }

    let fat = 512;
    for i in 0..128 {
        put_u32(&mut file, fat + i * 4, FREESECT);
    }
    put_u32(&mut file, fat, FATSECT);
    put_u32(&mut file, fat + 4, ENDOFCHAIN);
    put_u32(&mut file, fat + 8, ENDOFCHAIN);
    put_u32(&mut file, fat + 12, ENDOFCHAIN);

    let dir = 1024;
    write_dir(
        &mut file[dir..dir + 128],
        "Root Entry",
        5,
        1,
        3,
        mini_stream_size as u64,
    );
    write_dir(
        &mut file[dir + 128..dir + 256],
        "Workbook",
        2,
        NOSTREAM,
        0,
        workbook.len() as u64,
    );
    free_dir(&mut file[dir + 256..dir + 384]);
    free_dir(&mut file[dir + 384..dir + 512]);

    let minifat = 1536;
    for i in 0..128 {
        put_u32(&mut file, minifat + i * 4, FREESECT);
    }
    for i in 0..minis {
        let next = if i + 1 == minis {
            ENDOFCHAIN
        } else {
            (i + 1) as u32
        };
        put_u32(&mut file, minifat + i * 4, next);
    }
    file[2048..2048 + workbook.len()].copy_from_slice(workbook);
    file
}

fn write_dir(entry: &mut [u8], name: &str, kind: u8, child: u32, start: u32, size: u64) {
    let mut n = 0;
    for unit in name.encode_utf16() {
        entry[n..n + 2].copy_from_slice(&unit.to_le_bytes());
        n += 2;
    }
    entry[0x40..0x42].copy_from_slice(&((n + 2) as u16).to_le_bytes());
    entry[0x42] = kind;
    entry[0x43] = 1;
    entry[0x44..0x48].copy_from_slice(&NOSTREAM.to_le_bytes());
    entry[0x48..0x4C].copy_from_slice(&NOSTREAM.to_le_bytes());
    entry[0x4C..0x50].copy_from_slice(&child.to_le_bytes());
    entry[0x74..0x78].copy_from_slice(&start.to_le_bytes());
    entry[0x78..0x80].copy_from_slice(&size.to_le_bytes());
}

fn free_dir(entry: &mut [u8]) {
    entry[0x44..0x48].copy_from_slice(&NOSTREAM.to_le_bytes());
    entry[0x48..0x4C].copy_from_slice(&NOSTREAM.to_le_bytes());
    entry[0x4C..0x50].copy_from_slice(&NOSTREAM.to_le_bytes());
}

fn put_u16(buf: &mut [u8], at: usize, value: u16) {
    buf[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn put_u32(buf: &mut [u8], at: usize, value: u32) {
    buf[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn read_biff8(file: &[u8]) -> Result<Vec<Vec<String>>> {
    if file.len() < 512 || file[..8] != OLE_MAGIC {
        return Err(Error::new("xls is not BIFF"));
    }
    let stream = workbook_stream(file)?;
    parse_workbook(&stream)
}

fn workbook_stream(file: &[u8]) -> Result<Vec<u8>> {
    let major = u16_at(file, 0x1A)?;
    let order = u16_at(file, 0x1C)?;
    let sec_shift = u16_at(file, 0x1E)?;
    let mini_shift = u16_at(file, 0x20)?;
    if order != 0xFFFE || major != 3 || sec_shift != 9 || mini_shift != 6 {
        return Err(Error::new("xls is not BIFF"));
    }
    let num_fat = u32_at(file, 0x2C)?;
    let dir_start = u32_at(file, 0x30)?;
    let cutoff = u32_at(file, 0x38)?;
    let minifat_start = u32_at(file, 0x3C)?;
    let num_minifat = u32_at(file, 0x40)?;
    let num_difat = u32_at(file, 0x48)?;
    if cutoff != 4096 || num_difat != 0 || num_fat == 0 || num_fat > 109 {
        return Err(Error::new("xls is not BIFF"));
    }
    let mut fat = Vec::new();
    for i in 0..num_fat as usize {
        let sid = u32_at(file, 0x4C + i * 4)?;
        fat.extend_from_slice(&sector(file, sid)?);
    }
    let directory = read_fat_chain(file, &fat, dir_start, None)?;
    if directory.len() < 256 || entry_name(&directory[..128])? != "Root Entry" {
        return Err(Error::new("xls is not BIFF"));
    }
    let workbook = directory
        .chunks(128)
        .skip(1)
        .find(|entry| entry[0x42] == 2 && entry_name(entry).ok().as_deref() == Some("Workbook"))
        .ok_or_else(|| Error::new("xls is not BIFF"))?;
    let start = u32::from_le_bytes(workbook[0x74..0x78].try_into().unwrap());
    let size = u32::from_le_bytes(workbook[0x78..0x7C].try_into().unwrap()) as usize;
    if size > MAX_FILE {
        return Err(Error::new("xls is larger than 64MiB"));
    }
    if (size as u32) < cutoff {
        let mini_stream_size = u32::from_le_bytes(directory[0x78..0x7C].try_into().unwrap());
        let root_start = u32::from_le_bytes(directory[0x74..0x78].try_into().unwrap());
        let mini_stream = read_fat_chain(file, &fat, root_start, Some(mini_stream_size))?;
        let minifat = read_fat_chain(
            file,
            &fat,
            minifat_start,
            Some(num_minifat.saturating_mul(512)),
        )?;
        return read_mini_chain(&mini_stream, &minifat, start, size);
    }
    read_fat_chain(file, &fat, start, Some(size as u32))
}

fn parse_workbook(stream: &[u8]) -> Result<Vec<Vec<String>>> {
    if stream.len() < 8 {
        return Err(Error::new("xls is not BIFF"));
    }
    let mut i = 0usize;
    let mut saw_globals = false;
    let mut in_sheet = false;
    let mut sheet_done = false;
    let mut strings: Vec<String> = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut sheet_at: Option<u32> = None;
    let mut saw_sheet_bof = false;
    while i + 4 <= stream.len() {
        let op = u16::from_le_bytes(stream[i..i + 2].try_into().unwrap());
        let len = u16::from_le_bytes(stream[i + 2..i + 4].try_into().unwrap()) as usize;
        let data_at = i + 4;
        let next = data_at
            .checked_add(len)
            .ok_or_else(|| Error::new("xls record is truncated"))?;
        if next > stream.len() {
            return Err(Error::new("xls record is truncated"));
        }
        let data = &stream[data_at..next];
        if !saw_globals && op != 0x0809 {
            return Err(Error::new("xls is not BIFF"));
        }
        match op {
            0x0809 => {
                if data.len() != 16 {
                    return Err(Error::new("xls is not BIFF"));
                }
                let vers = u16::from_le_bytes(data[0..2].try_into().unwrap());
                let dt = u16::from_le_bytes(data[2..4].try_into().unwrap());
                if vers != 0x0600 {
                    return Err(Error::new("xls is not BIFF"));
                }
                if !saw_globals {
                    if dt != 0x0005 {
                        return Err(Error::new("xls is not BIFF"));
                    }
                    saw_globals = true;
                } else if dt == 0x0010 && !sheet_done {
                    in_sheet = true;
                    saw_sheet_bof = true;
                } else if in_sheet {
                    return Err(Error::new("xls sheet bof is nested"));
                }
            }
            0x000A => {
                if len != 0 {
                    return Err(Error::new("xls eof length is refused"));
                }
                if in_sheet {
                    in_sheet = false;
                    sheet_done = true;
                }
            }
            0x002F => return Err(Error::new("encrypted xls is refused")),
            0x0085 if sheet_at.is_none() => {
                if data.len() < 4 {
                    return Err(Error::new("xls boundsheet is truncated"));
                }
                sheet_at = Some(u32::from_le_bytes(data[0..4].try_into().unwrap()));
            }
            0x00FC if !in_sheet => strings = parse_sst(data)?,
            0x003C => return Err(Error::new("xls continue record is refused")),
            0x0203 if in_sheet => {
                if data.len() != 14 {
                    return Err(Error::new("xls number record length is refused"));
                }
                let row = u16::from_le_bytes(data[0..2].try_into().unwrap());
                let col = u16::from_le_bytes(data[2..4].try_into().unwrap());
                let value = f64::from_le_bytes(data[6..14].try_into().unwrap());
                place(&mut rows, row, col, number_text(value)?)?;
            }
            0x00FD if in_sheet => {
                if data.len() != 10 {
                    return Err(Error::new("xls labelsst record length is refused"));
                }
                let row = u16::from_le_bytes(data[0..2].try_into().unwrap());
                let col = u16::from_le_bytes(data[2..4].try_into().unwrap());
                let index = u32::from_le_bytes(data[6..10].try_into().unwrap()) as usize;
                let text = strings.get(index).cloned().ok_or_else(|| {
                    Error::new(format!("xls shared string index {index} is out of range"))
                })?;
                place(&mut rows, row, col, text)?;
            }
            _ => {}
        }
        i = next;
    }
    if !saw_globals || !saw_sheet_bof {
        return Err(Error::new("xls is not BIFF"));
    }
    let pos = sheet_at.ok_or_else(|| Error::new("xls is not BIFF"))? as usize;
    if stream.get(pos..pos + 4) != Some(&[0x09, 0x08, 0x10, 0x00]) {
        return Err(Error::new("xls is not BIFF"));
    }
    Ok(rows)
}

fn parse_sst(data: &[u8]) -> Result<Vec<String>> {
    if data.len() < 8 {
        return Err(Error::new("xls shared string table is truncated"));
    }
    let total = u32::from_le_bytes(data[0..4].try_into().unwrap());
    let unique = u32::from_le_bytes(data[4..8].try_into().unwrap());
    if unique > total || unique > 1_048_576 {
        return Err(Error::new("xls shared string table is refused"));
    }
    let mut i = 8usize;
    let mut out = Vec::with_capacity(unique as usize);
    for _ in 0..unique {
        if i + 3 > data.len() {
            return Err(Error::new("xls shared string table is truncated"));
        }
        let chars = u16::from_le_bytes(data[i..i + 2].try_into().unwrap()) as usize;
        let flags = data[i + 2];
        i += 3;
        if flags & !1 != 0 {
            return Err(Error::new("xls shared string flags are refused"));
        }
        let nbytes = if flags & 1 == 0 {
            chars
        } else {
            chars
                .checked_mul(2)
                .ok_or_else(|| Error::new("xls shared string is refused"))?
        };
        if i + nbytes > data.len() {
            return Err(Error::new("xls shared string table is truncated"));
        }
        let text = if flags & 1 == 0 {
            data[i..i + nbytes].iter().map(|&b| char::from(b)).collect()
        } else {
            let units: Vec<u16> = data[i..i + nbytes]
                .chunks_exact(2)
                .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
                .collect();
            String::from_utf16(&units).map_err(|_| Error::new("xls shared string is not utf-16"))?
        };
        i += nbytes;
        out.push(text);
    }
    if i != data.len() {
        return Err(Error::new("xls shared string table has trailing bytes"));
    }
    Ok(out)
}

fn number_text(value: f64) -> Result<String> {
    if !value.is_finite() {
        return Err(Error::new("xls number is not finite"));
    }
    Ok(format!("{value}"))
}

fn place(rows: &mut Vec<Vec<String>>, row: u16, col: u16, text: String) -> Result<()> {
    if col > 255 {
        return Err(Error::new("xls cell is outside BIFF8 limits"));
    }
    let row = row as usize;
    let col = col as usize;
    while rows.len() < row {
        rows.push(Vec::new());
    }
    if rows.len() == row {
        rows.push(Vec::new());
    }
    let line = &mut rows[row];
    if col < line.len() {
        return Err(Error::new("xls cells are out of order"));
    }
    while line.len() < col {
        line.push(String::new());
    }
    line.push(text);
    Ok(())
}

fn sector(file: &[u8], id: u32) -> Result<Vec<u8>> {
    let start = (id as usize)
        .checked_add(1)
        .and_then(|n| n.checked_mul(512))
        .ok_or_else(|| Error::new("xls sector is out of range"))?;
    let end = start + 512;
    if end > file.len() {
        return Err(Error::new("xls sector is truncated"));
    }
    Ok(file[start..end].to_vec())
}

fn read_fat_chain(file: &[u8], fat: &[u8], start: u32, size: Option<u32>) -> Result<Vec<u8>> {
    let want = size.map(|n| n as usize);
    if let Some(0) = want {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    let mut sid = start;
    let mut guard = 0usize;
    while sid != ENDOFCHAIN {
        if sid == FREESECT || sid == FATSECT || sid >= 0xFFFF_FFF0 {
            return Err(Error::new("xls fat chain is refused"));
        }
        guard += 1;
        if guard > fat.len() / 4 + 1 {
            return Err(Error::new("xls fat chain loops"));
        }
        let bytes = sector(file, sid)?;
        if let Some(want) = want {
            let take = (want - out.len()).min(bytes.len());
            out.extend_from_slice(&bytes[..take]);
            if out.len() == want {
                return Ok(out);
            }
        } else {
            out.extend_from_slice(&bytes);
        }
        let at = (sid as usize)
            .checked_mul(4)
            .ok_or_else(|| Error::new("xls fat chain is refused"))?;
        if at + 4 > fat.len() {
            return Err(Error::new("xls fat chain is truncated"));
        }
        sid = u32::from_le_bytes(fat[at..at + 4].try_into().unwrap());
    }
    if want.is_some_and(|want| out.len() != want) {
        return Err(Error::new("xls stream is truncated"));
    }
    Ok(out)
}

fn read_mini_chain(mini_stream: &[u8], minifat: &[u8], start: u32, size: usize) -> Result<Vec<u8>> {
    if size == 0 {
        return Ok(Vec::new());
    }
    let mut out = Vec::with_capacity(size);
    let mut sid = start;
    let mut guard = 0usize;
    while out.len() < size {
        if sid >= 0xFFFF_FFF0 {
            return Err(Error::new("xls mini fat chain is refused"));
        }
        guard += 1;
        if guard > minifat.len() / 4 + 1 {
            return Err(Error::new("xls mini fat chain loops"));
        }
        let at = (sid as usize) * 64;
        if at + 64 > mini_stream.len() && at >= mini_stream.len() {
            return Err(Error::new("xls mini stream is truncated"));
        }
        let end = (at + 64).min(mini_stream.len());
        let take = (size - out.len()).min(end - at);
        out.extend_from_slice(&mini_stream[at..at + take]);
        if out.len() == size {
            break;
        }
        let fat_at = (sid as usize) * 4;
        if fat_at + 4 > minifat.len() {
            return Err(Error::new("xls mini fat chain is truncated"));
        }
        sid = u32::from_le_bytes(minifat[fat_at..fat_at + 4].try_into().unwrap());
    }
    if out.len() != size {
        return Err(Error::new("xls mini stream is truncated"));
    }
    Ok(out)
}

fn entry_name(entry: &[u8]) -> Result<String> {
    let n = u16::from_le_bytes(entry[0x40..0x42].try_into().unwrap()) as usize;
    if !(2..=64).contains(&n) || n % 2 != 0 {
        return Err(Error::new("xls directory name is refused"));
    }
    let units: Vec<u16> = entry[..n - 2]
        .chunks_exact(2)
        .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
        .collect();
    String::from_utf16(&units).map_err(|_| Error::new("xls directory name is not utf-16"))
}

fn u16_at(file: &[u8], at: usize) -> Result<u16> {
    file.get(at..at + 2)
        .map(|b| u16::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| Error::new("xls is not BIFF"))
}

fn u32_at(file: &[u8], at: usize) -> Result<u32> {
    file.get(at..at + 4)
        .map(|b| u32::from_le_bytes(b.try_into().unwrap()))
        .ok_or_else(|| Error::new("xls is not BIFF"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(stream: &[u8], op: u16) -> &[u8] {
        let mut i = 0;
        while i + 4 <= stream.len() {
            let found = u16::from_le_bytes(stream[i..i + 2].try_into().unwrap());
            let len = u16::from_le_bytes(stream[i + 2..i + 4].try_into().unwrap()) as usize;
            if found == op {
                return &stream[i + 4..i + 4 + len];
            }
            i += 4 + len;
        }
        panic!("missing record {op:#x}");
    }

    #[test]
    fn generated_stream_is_biff8_with_42_and_cat() {
        let stream = workbook_bytes();
        assert_eq!(
            &stream[..8],
            &[0x09, 0x08, 0x10, 0x00, 0x00, 0x06, 0x05, 0x00]
        );
        let number = record(&stream, 0x0203);
        assert_eq!(number.len(), 14);
        assert_eq!(f64::from_le_bytes(number[6..14].try_into().unwrap()), 42.0);
        let sst = record(&stream, 0x00FC);
        assert_eq!(&sst[8..], &[3, 0, 0, b'c', b'a', b't']);
        let pos = u32::from_le_bytes(record(&stream, 0x0085)[0..4].try_into().unwrap()) as usize;
        assert_eq!(
            &stream[pos..pos + 8],
            &[0x09, 0x08, 0x10, 0x00, 0x00, 0x06, 0x10, 0x00]
        );
        let file = generated_biff_xls();
        assert_eq!(&file[..8], &OLE_MAGIC);
        assert_eq!(u16::from_le_bytes(file[0x1A..0x1C].try_into().unwrap()), 3);
        assert_eq!(u16::from_le_bytes(file[0x1E..0x20].try_into().unwrap()), 9);
    }

    #[test]
    fn two_numbers_keep_the_same_text_cell() {
        let stream = workbook_two_numbers();
        assert_eq!(
            &stream[..8],
            &[0x09, 0x08, 0x10, 0x00, 0x00, 0x06, 0x05, 0x00]
        );
        let numbers = records(&stream, 0x0203);
        assert_eq!(numbers.len(), 2);
        assert_eq!(u16::from_le_bytes(numbers[0][2..4].try_into().unwrap()), 0);
        assert_eq!(
            f64::from_le_bytes(numbers[0][6..14].try_into().unwrap()),
            42.0
        );
        assert_eq!(u16::from_le_bytes(numbers[1][2..4].try_into().unwrap()), 2);
        assert_eq!(
            f64::from_le_bytes(numbers[1][6..14].try_into().unwrap()),
            7.0
        );
        let text = record(&stream, 0x00FD);
        assert_eq!(u16::from_le_bytes(text[2..4].try_into().unwrap()), 1);
        assert_eq!(&record(&stream, 0x00FC)[8..], &[3, 0, 0, b'c', b'a', b't']);
    }

    fn records(stream: &[u8], op: u16) -> Vec<&[u8]> {
        let mut found = Vec::new();
        let mut i = 0;
        while i + 4 <= stream.len() {
            let opcode = u16::from_le_bytes(stream[i..i + 2].try_into().unwrap());
            let len = u16::from_le_bytes(stream[i + 2..i + 4].try_into().unwrap()) as usize;
            if opcode == op {
                found.push(&stream[i + 4..i + 4 + len]);
            }
            i += 4 + len;
        }
        found
    }
}
