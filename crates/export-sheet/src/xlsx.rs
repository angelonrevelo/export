//! One worksheet, shared strings, and numbers.
//!
//! The package is a zip of OOXML parts. This writer stores every part (zip
//! method 0). A reader accepts that, and also deflate, because [`crate::zip`]
//! inflates method 8 with the decoder tested against known streams. Anything
//! else refuses the file.
//!
//! Only the first sheet is read. A cell whose text is a plain number is
//! written as a number, in the same characters, not through a float. Every
//! other non-empty cell is a shared string. Empty cells are written so a
//! trailing blank field survives.

use std::collections::HashMap;
use std::io::{BufRead, Write};

use crate::delim::{self, DelimReader};
use crate::error::{Error, Result};
use crate::xml::{self, Ev, Pull};
use crate::zip::{self, ZipWriter};
use crate::Stats;

pub(crate) fn delim_to_xlsx<R: BufRead, W: Write>(
    input: R,
    output: &mut W,
    delim: u8,
) -> Result<Stats> {
    let mut reader = DelimReader::new(input, delim);
    let mut sheet = SheetWriter::new();
    let mut width: Option<usize> = None;
    let mut seen = 0u64;
    while reader.next_row()? {
        seen += 1;
        let n = reader.len();
        match width {
            None => width = Some(n),
            Some(w) if n != w => {
                return Err(Error::new(format!(
                    "row {seen}: {n} columns, header has {w}"
                )));
            }
            Some(_) => {}
        }
        sheet.add_row(&reader)?;
    }
    let lossy = reader.lossy();
    let (bytes, rows) = sheet.finish()?;
    output.write_all(&bytes)?;
    Ok(Stats { rows, lossy })
}

pub(crate) fn xlsx_to_delim<R: BufRead, W: Write>(
    mut input: R,
    output: &mut W,
    delim: u8,
) -> Result<Stats> {
    let mut raw = Vec::new();
    input.read_to_end(&mut raw)?;
    let (rows, lossy) = read_first_sheet(&raw)?;
    for row in &rows {
        delim::write_row(output, delim, row.iter().map(String::as_str))?;
    }
    Ok(Stats {
        rows: rows.len() as u64,
        lossy,
    })
}

struct SheetWriter {
    rows: u64,
    string_uses: u32,
    strings: Vec<String>,
    index: HashMap<String, u32>,
    sheet: Vec<u8>,
}

impl SheetWriter {
    fn new() -> Self {
        let mut sheet = Vec::new();
        sheet.extend_from_slice(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
        sheet.extend_from_slice(
            br#"<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData>"#,
        );
        Self {
            rows: 0,
            string_uses: 0,
            strings: Vec::new(),
            index: HashMap::new(),
            sheet,
        }
    }

    fn add_row<R: BufRead>(&mut self, reader: &DelimReader<R>) -> Result<()> {
        self.rows += 1;
        if self.rows > u32::MAX as u64 {
            return Err(Error::new("xlsx row number does not fit"));
        }
        let row = self.rows as u32;
        self.sheet.extend_from_slice(b"<row r=\"");
        push_u32(&mut self.sheet, row);
        self.sheet.extend_from_slice(b"\">");
        for col in 0..reader.len() {
            self.write_cell(row, col as u32, reader.field(col))?;
        }
        self.sheet.extend_from_slice(b"</row>");
        Ok(())
    }

    fn write_cell(&mut self, row: u32, col: u32, field: &str) -> Result<()> {
        self.sheet.extend_from_slice(b"<c r=\"");
        push_col(&mut self.sheet, col);
        push_u32(&mut self.sheet, row);
        self.sheet.push(b'"');
        if field.is_empty() {
            self.sheet.extend_from_slice(b"/>");
            return Ok(());
        }
        if is_number(field) {
            self.sheet.extend_from_slice(b"><v>");
            self.sheet.extend_from_slice(field.as_bytes());
            self.sheet.extend_from_slice(b"</v></c>");
            return Ok(());
        }
        let id = self.intern(field);
        self.sheet.extend_from_slice(b" t=\"s\"><v>");
        push_u32(&mut self.sheet, id);
        self.sheet.extend_from_slice(b"</v></c>");
        Ok(())
    }

    fn intern(&mut self, field: &str) -> u32 {
        if let Some(&id) = self.index.get(field) {
            self.string_uses += 1;
            return id;
        }
        let id = self.strings.len() as u32;
        self.strings.push(field.to_string());
        self.index.insert(field.to_string(), id);
        self.string_uses += 1;
        id
    }

    fn finish(mut self) -> Result<(Vec<u8>, u64)> {
        self.sheet.extend_from_slice(b"</sheetData></worksheet>");
        let rows = self.rows;
        let mut zip = ZipWriter::new();
        zip.add_stored("[Content_Types].xml", CONTENT_TYPES)?;
        zip.add_stored("_rels/.rels", ROOT_RELS)?;
        zip.add_stored("xl/workbook.xml", WORKBOOK)?;
        zip.add_stored("xl/_rels/workbook.xml.rels", WORKBOOK_RELS)?;
        zip.add_stored("xl/styles.xml", STYLES)?;
        zip.add_stored(
            "xl/sharedStrings.xml",
            &shared_strings(&self.strings, self.string_uses),
        )?;
        zip.add_stored("xl/worksheets/sheet1.xml", &self.sheet)?;
        Ok((zip.finish()?, rows))
    }
}

fn shared_strings(strings: &[String], uses: u32) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#);
    out.extend_from_slice(
        br#"<sst xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" count=""#,
    );
    push_u32(&mut out, uses);
    out.extend_from_slice(br#"" uniqueCount=""#);
    push_u32(&mut out, strings.len() as u32);
    out.extend_from_slice(br#"">"#);
    for text in strings {
        out.extend_from_slice(b"<si><t");
        if preserve(text) {
            out.extend_from_slice(br#" xml:space="preserve""#);
        }
        out.push(b'>');
        push_xml_text(&mut out, text);
        out.extend_from_slice(b"</t></si>");
    }
    out.extend_from_slice(b"</sst>");
    out
}

fn preserve(text: &str) -> bool {
    text.starts_with(char::is_whitespace) || text.ends_with(char::is_whitespace)
}

fn push_xml_text(out: &mut Vec<u8>, text: &str) {
    for ch in text.chars() {
        match ch {
            '&' => out.extend_from_slice(b"&amp;"),
            '<' => out.extend_from_slice(b"&lt;"),
            '>' => out.extend_from_slice(b"&gt;"),
            '"' => out.extend_from_slice(b"&quot;"),
            '\t' => out.extend_from_slice(b"&#9;"),
            '\n' => out.extend_from_slice(b"&#10;"),
            '\r' => out.extend_from_slice(b"&#13;"),
            _ => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
}

fn push_u32(out: &mut Vec<u8>, mut n: u32) {
    let mut tmp = [0u8; 10];
    let mut i = 10;
    if n == 0 {
        out.push(b'0');
        return;
    }
    while n > 0 {
        i -= 1;
        tmp[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    out.extend_from_slice(&tmp[i..]);
}

fn push_col(out: &mut Vec<u8>, index: u32) {
    let mut tmp = [0u8; 8];
    let mut n = 0;
    let mut c = index + 1;
    while c > 0 {
        c -= 1;
        tmp[n] = b'A' + (c % 26) as u8;
        n += 1;
        c /= 26;
    }
    while n > 0 {
        n -= 1;
        out.push(tmp[n]);
    }
}

fn is_number(text: &str) -> bool {
    let b = text.as_bytes();
    if b.is_empty() {
        return false;
    }
    let mut i = 0;
    if b[0] == b'-' {
        i = 1;
        if i == b.len() {
            return false;
        }
    }
    if b[i] == b'0' {
        i += 1;
    } else if b[i].is_ascii_digit() {
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
    } else {
        return false;
    }
    if i < b.len() && b[i] == b'.' {
        i += 1;
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
        i += 1;
        if i < b.len() && (b[i] == b'+' || b[i] == b'-') {
            i += 1;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_digit() {
            i += 1;
        }
        if i == start {
            return false;
        }
    }
    i == b.len()
}

fn read_first_sheet(bytes: &[u8]) -> Result<(Vec<Vec<String>>, u64)> {
    let (parts, lossy) = zip::unzip(bytes)?;
    let workbook = part(&parts, "xl/workbook.xml")?;
    let rels = part(&parts, "xl/_rels/workbook.xml.rels")?;
    let sheet_rid = first_sheet_rid(workbook)?;
    let sheet_path = sheet_target(rels, &sheet_rid)?;
    let strings = match part_opt(&parts, "xl/sharedStrings.xml") {
        Some(bytes) => parse_shared_strings(bytes)?,
        None => Vec::new(),
    };
    let rows = parse_sheet(part(&parts, &sheet_path)?, &strings)?;
    Ok((rows, lossy))
}

fn part<'a>(parts: &'a [zip::Part], name: &str) -> Result<&'a [u8]> {
    parts
        .iter()
        .find(|part| part.name == name)
        .map(|part| part.bytes.as_slice())
        .ok_or_else(|| Error::new(format!("xlsx is missing {name}")))
}

fn part_opt<'a>(parts: &'a [zip::Part], name: &str) -> Option<&'a [u8]> {
    parts
        .iter()
        .find(|part| part.name == name)
        .map(|part| part.bytes.as_slice())
}

fn first_sheet_rid(workbook: &[u8]) -> Result<String> {
    let mut pull = Pull::new(workbook);
    let mut text = String::new();
    let mut attrs = Vec::new();
    while let Some(ev) = pull.next(&mut text, &mut attrs)? {
        if matches!(ev, Ev::Open | Ev::Empty) && pull.local() == "sheet" {
            let id = xml::attr(&attrs, "id")
                .ok_or_else(|| Error::new("xlsx sheet has no relationship id"))?;
            return Ok(id.to_string());
        }
    }
    Err(Error::new("xlsx workbook has no sheet"))
}

fn sheet_target(rels: &[u8], rid: &str) -> Result<String> {
    let mut pull = Pull::new(rels);
    let mut text = String::new();
    let mut attrs = Vec::new();
    let mut fallback: Option<String> = None;
    while let Some(ev) = pull.next(&mut text, &mut attrs)? {
        if !matches!(ev, Ev::Open | Ev::Empty) || pull.local() != "Relationship" {
            continue;
        }
        let id = xml::attr(&attrs, "Id").unwrap_or("");
        let kind = xml::attr(&attrs, "Type").unwrap_or("");
        let target = xml::attr(&attrs, "Target").unwrap_or("");
        if !kind.contains("worksheet") {
            continue;
        }
        let path = resolve_target(target);
        if id == rid {
            return Ok(path);
        }
        if fallback.is_none() {
            fallback = Some(path);
        }
    }
    fallback.ok_or_else(|| Error::new("xlsx workbook does not point at a worksheet"))
}

fn resolve_target(target: &str) -> String {
    let target = target.strip_prefix('/').unwrap_or(target);
    if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{target}")
    }
}

fn parse_shared_strings(xml: &[u8]) -> Result<Vec<String>> {
    let mut pull = Pull::new(xml);
    let mut text = String::new();
    let mut attrs = Vec::new();
    let mut strings = Vec::new();
    let mut current = String::new();
    let mut in_si = false;
    let mut in_t = false;
    let mut skip = 0i32;
    while let Some(ev) = pull.next(&mut text, &mut attrs)? {
        match ev {
            Ev::Open | Ev::Empty => {
                if pull.local() == "si" {
                    in_si = true;
                    current.clear();
                }
                if pull.local() == "rPh" || pull.local() == "phoneticPr" {
                    skip += 1;
                }
                if pull.local() == "t" && skip == 0 {
                    in_t = true;
                }
                if ev == Ev::Empty {
                    if pull.local() == "t" {
                        in_t = false;
                    }
                    if pull.local() == "si" {
                        strings.push(std::mem::take(&mut current));
                        in_si = false;
                    }
                    if pull.local() == "rPh" || pull.local() == "phoneticPr" {
                        skip -= 1;
                    }
                }
            }
            Ev::Close => {
                if pull.local() == "t" {
                    in_t = false;
                }
                if pull.local() == "rPh" || pull.local() == "phoneticPr" {
                    skip -= 1;
                }
                if pull.local() == "si" {
                    strings.push(std::mem::take(&mut current));
                    in_si = false;
                }
            }
            Ev::Text if in_si && in_t && skip == 0 => current.push_str(&text),
            Ev::Text => {}
        }
    }
    Ok(strings)
}

fn parse_sheet(xml: &[u8], strings: &[String]) -> Result<Vec<Vec<String>>> {
    let mut pull = Pull::new(xml);
    let mut text = String::new();
    let mut attrs = Vec::new();
    let mut rows = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut in_row = false;
    let mut in_c = false;
    let mut in_v = false;
    let mut in_is = false;
    let mut in_inline_t = false;
    let mut cell_type = String::new();
    let mut cell_col: Option<u32> = None;
    let mut value = String::new();
    let mut inline = String::new();
    while let Some(ev) = pull.next(&mut text, &mut attrs)? {
        match ev {
            Ev::Open | Ev::Empty => {
                match pull.local() {
                    "row" => {
                        if in_row {
                            return Err(Error::new("xlsx row is nested"));
                        }
                        in_row = true;
                        row.clear();
                        let declared = xml::attr(&attrs, "r").and_then(|s| s.parse::<u32>().ok());
                        if let Some(n) = declared {
                            if n == 0 || n < rows.len() as u32 + 1 {
                                return Err(Error::new("xlsx row numbers go backwards"));
                            }
                            while rows.len() as u32 + 1 < n {
                                rows.push(Vec::new());
                            }
                        }
                    }
                    "c" if in_row => {
                        in_c = true;
                        cell_type = xml::attr(&attrs, "t").unwrap_or("").to_string();
                        cell_col = xml::attr(&attrs, "r").and_then(col_index);
                        value.clear();
                        inline.clear();
                    }
                    "v" if in_c => in_v = true,
                    "is" if in_c => in_is = true,
                    "t" if in_is => in_inline_t = ev == Ev::Open,
                    _ => {}
                }
                if ev == Ev::Empty {
                    if pull.local() == "c" && in_row {
                        place_cell(
                            &mut row,
                            cell_col,
                            cell_text(&cell_type, &value, &inline, strings)?,
                        )?;
                        in_c = false;
                        in_v = false;
                    }
                    if pull.local() == "row" {
                        rows.push(std::mem::take(&mut row));
                        in_row = false;
                    }
                    if pull.local() == "v" {
                        in_v = false;
                    }
                    if pull.local() == "is" {
                        in_is = false;
                    }
                }
            }
            Ev::Close => match pull.local() {
                "t" => in_inline_t = false,
                "v" => in_v = false,
                "is" => {
                    in_is = false;
                    in_inline_t = false;
                }
                "c" => {
                    place_cell(
                        &mut row,
                        cell_col,
                        cell_text(&cell_type, &value, &inline, strings)?,
                    )?;
                    in_c = false;
                }
                "row" => {
                    rows.push(std::mem::take(&mut row));
                    in_row = false;
                }
                _ => {}
            },
            Ev::Text if in_v => value.push_str(&text),
            Ev::Text if in_inline_t => inline.push_str(&text),
            Ev::Text => {}
        }
    }
    Ok(rows)
}

fn cell_text(kind: &str, value: &str, inline: &str, strings: &[String]) -> Result<String> {
    match kind {
        "s" => {
            let idx: usize = value.trim().parse().map_err(|_| {
                Error::new(format!("shared string index \"{value}\" is not a number"))
            })?;
            strings
                .get(idx)
                .cloned()
                .ok_or_else(|| Error::new(format!("shared string index {idx} is out of range")))
        }
        "inlineStr" => Ok(inline.to_string()),
        _ => Ok(value.trim().to_string()),
    }
}

fn place_cell(row: &mut Vec<String>, col: Option<u32>, text: String) -> Result<()> {
    let col = col.unwrap_or(row.len() as u32) as usize;
    if col < row.len() {
        return Err(Error::new("xlsx cells are out of order"));
    }
    while row.len() < col {
        row.push(String::new());
    }
    row.push(text);
    Ok(())
}

fn col_index(reference: &str) -> Option<u32> {
    let mut n = 0u32;
    let mut saw = false;
    for b in reference.bytes() {
        match b {
            b'A'..=b'Z' => {
                saw = true;
                n = n.checked_mul(26)?.checked_add(u32::from(b - b'A' + 1))?;
            }
            b'a'..=b'z' => {
                saw = true;
                n = n.checked_mul(26)?.checked_add(u32::from(b - b'a' + 1))?;
            }
            _ => break,
        }
    }
    saw.then_some(n.checked_sub(1)?)
}

const CONTENT_TYPES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">
<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>
<Default Extension="xml" ContentType="application/xml"/>
<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>
<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>
<Override PartName="/xl/sharedStrings.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sharedStrings+xml"/>
<Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/>
</Types>"#;

const ROOT_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>
</Relationships>"#;

const WORKBOOK: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships">
<sheets><sheet name="Sheet" sheetId="1" r:id="rId1"/></sheets>
</workbook>"#;

const WORKBOOK_RELS: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">
<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>
<Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/sharedStrings" Target="sharedStrings.xml"/>
<Relationship Id="rId3" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/>
</Relationships>"#;

const STYLES: &[u8] = br#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main">
<fonts count="1"><font><sz val="11"/><name val="Calibri"/></font></fonts>
<fills count="1"><fill><patternFill patternType="none"/></fill></fills>
<borders count="1"><border/></borders>
<cellStyleXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0"/></cellStyleXfs>
<cellXfs count="1"><xf numFmtId="0" fontId="0" fillId="0" borderId="0" xfId="0"/></cellXfs>
</styleSheet>"#;
