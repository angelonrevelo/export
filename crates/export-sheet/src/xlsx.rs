//! One worksheet, shared strings, and numbers.
//!
//! The package is a zip of OOXML parts. This writer stores every part (zip
//! method 0). A reader accepts that, and also deflate, because [`crate::zip`]
//! inflates method 8 with the decoder tested against known streams. Anything
//! else refuses the file.
//!
//! The reader takes the workbook's active sheet (the first one when the
//! workbook names no active tab). When the sheet declares a `<dimension>`
//! rectangle, rows and cells are padded or dropped to that rectangle; without
//! one, rows come back exactly as stored. A numeric cell whose style carries
//! a date number format becomes an ISO date or time, matching what a
//! spreadsheet reader hands a text writer: serials under one day are times,
//! the 1900 leap-year serials 1-59 shift by one day, and the epoch is
//! 1899-12-30 (1904-01-01 when the workbook declares `date1904`). Booleans
//! become `True` and `False`. Numbers without a decimal point or exponent
//! keep their literal digits; other numbers are re-printed with the shortest
//! digits that round-trip, which is what a float repr prints.

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
    let sheet_rid = active_sheet_rid(workbook)?;
    let sheet_path = sheet_target(rels, &sheet_rid)?;
    let strings = match part_opt(&parts, "xl/sharedStrings.xml") {
        Some(bytes) => parse_shared_strings(bytes)?,
        None => Vec::new(),
    };
    let styles = match part_opt(&parts, "xl/styles.xml") {
        Some(bytes) => parse_styles(bytes)?,
        None => Styles::default(),
    };
    let ctx = SheetCtx {
        strings,
        styles,
        date1904: workbook_is_date1904(workbook)?,
    };
    let rows = parse_sheet(part(&parts, &sheet_path)?, &ctx)?;
    Ok((rows, lossy))
}

/// Which `<sheet>` the workbook's `activeTab` names; the first one by default.
fn active_sheet_rid(workbook: &[u8]) -> Result<String> {
    let mut pull = Pull::new(workbook);
    let mut text = String::new();
    let mut attrs = Vec::new();
    let mut active_tab: u32 = 0;
    let mut sheets: Vec<String> = Vec::new();
    while let Some(ev) = pull.next(&mut text, &mut attrs)? {
        if !matches!(ev, Ev::Open | Ev::Empty) {
            continue;
        }
        match pull.local() {
            "workbookView" => {
                if let Some(tab) = xml::attr(&attrs, "activeTab").and_then(|s| s.parse().ok()) {
                    active_tab = tab;
                }
            }
            "sheet" => {
                let id = xml::attr(&attrs, "id")
                    .ok_or_else(|| Error::new("xlsx sheet has no relationship id"))?;
                sheets.push(id.to_string());
            }
            _ => {}
        }
    }
    if sheets.is_empty() {
        return Err(Error::new("xlsx workbook has no sheet"));
    }
    let index = (active_tab as usize).min(sheets.len() - 1);
    Ok(std::mem::take(&mut sheets[index]))
}

/// `<workbookPr date1904="1"/>`: the 1904 date system instead of 1900.
fn workbook_is_date1904(workbook: &[u8]) -> Result<bool> {
    let mut pull = Pull::new(workbook);
    let mut text = String::new();
    let mut attrs = Vec::new();
    while let Some(ev) = pull.next(&mut text, &mut attrs)? {
        if matches!(ev, Ev::Open | Ev::Empty) && pull.local() == "workbookPr" {
            return Ok(matches!(
                xml::attr(&attrs, "date1904").unwrap_or(""),
                "1" | "true" | "TRUE" | "True"
            ));
        }
    }
    Ok(false)
}

/// Number formats declared by the workbook, and the format id of every cell style.
#[derive(Default)]
struct Styles {
    /// numFmtId of each `cellXfs` entry, in order.
    xfs: Vec<u32>,
    /// Custom `numFmt` codes by id (builtins 14-22 and 45-47 are dates).
    custom: HashMap<u32, String>,
}

fn parse_styles(xml: &[u8]) -> Result<Styles> {
    let mut pull = Pull::new(xml);
    let mut text = String::new();
    let mut attrs = Vec::new();
    let mut styles = Styles::default();
    let mut in_cell_xfs = false;
    while let Some(ev) = pull.next(&mut text, &mut attrs)? {
        match ev {
            Ev::Open | Ev::Empty => match pull.local() {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (
                        xml::attr(&attrs, "numFmtId").and_then(|s| s.parse().ok()),
                        xml::attr(&attrs, "formatCode"),
                    ) {
                        styles.custom.insert(id, code.to_string());
                    }
                }
                "cellXfs" => in_cell_xfs = ev == Ev::Open,
                "xf" if in_cell_xfs => {
                    let id = xml::attr(&attrs, "numFmtId")
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0);
                    styles.xfs.push(id);
                }
                _ => {}
            },
            Ev::Close if pull.local() == "cellXfs" => in_cell_xfs = false,
            _ => {}
        }
    }
    Ok(styles)
}

/// The builtin ids whose format code is a date or time.
fn builtin_is_date(id: u32) -> bool {
    matches!(id, 14..=22 | 45..=47)
}

/// openpyxl's `is_date_format`: first `;` section, quoted literals and
/// non-time bracket conditions stripped, then a date/time letter that is not
/// escaped by `_` or `\`.
fn format_is_date(code: &str) -> bool {
    let first = code.split(';').next().unwrap_or("");
    let chars: Vec<char> = first.chars().collect();
    let mut stripped: Vec<char> = Vec::with_capacity(chars.len());
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '"' => {
                i += 1;
                while i < chars.len() && chars[i] != '"' {
                    i += 1;
                }
                i += 1;
            }
            '[' => {
                let end = chars[i + 1..].iter().position(|&c| c == ']').map(|p| i + 1 + p);
                match end {
                    Some(end) => {
                        let inner: String = chars[i + 1..end].iter().collect();
                        if matches!(inner.as_str(), "h" | "hh" | "m" | "mm" | "s" | "ss") {
                            stripped.extend(chars[i + 1..end].iter());
                        }
                        i = end + 1;
                    }
                    None => break,
                }
            }
            c => {
                stripped.push(c);
                i += 1;
            }
        }
    }
    stripped.iter().enumerate().any(|(i, &c)| {
        matches!(c, 'd' | 'm' | 'h' | 'y' | 's' | 'D' | 'M' | 'H' | 'Y' | 'S')
            && (i == 0 || (stripped[i - 1] != '_' && stripped[i - 1] != '\\'))
    })
}

/// One sheet's read context: shared strings, styles, and the date epoch.
struct SheetCtx {
    strings: Vec<String>,
    styles: Styles,
    date1904: bool,
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

fn parse_sheet(xml: &[u8], ctx: &SheetCtx) -> Result<Vec<Vec<String>>> {
    let mut pull = Pull::new(xml);
    let mut text = String::new();
    let mut attrs = Vec::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut in_row = false;
    let mut in_c = false;
    let mut in_v = false;
    let mut in_is = false;
    let mut in_inline_t = false;
    let mut cell_type = String::new();
    let mut cell_col: Option<u32> = None;
    let mut cell_style: u32 = 0;
    let mut value = String::new();
    let mut inline = String::new();
    let mut rect: Option<(u32, u32)> = None;
    while let Some(ev) = pull.next(&mut text, &mut attrs)? {
        match ev {
            Ev::Open | Ev::Empty => {
                match pull.local() {
                    "dimension" => {
                        rect = xml::attr(&attrs, "ref").and_then(parse_dimension);
                    }
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
                        cell_style = xml::attr(&attrs, "s")
                            .and_then(|s| s.parse().ok())
                            .unwrap_or(0);
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
                            cell_text(&cell_type, &value, &inline, ctx, cell_style)?,
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
                        cell_text(&cell_type, &value, &inline, ctx, cell_style)?,
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
    if let Some((max_row, max_col)) = rect {
        let max_row = max_row as usize;
        let max_col = max_col as usize;
        // openpyxl fills missing rows between parsed ones, and fills up to
        // the rectangle only when a parsed row ran past it.
        let ran_past = rows.len() > max_row;
        rows.truncate(max_row);
        for row in rows.iter_mut() {
            row.truncate(max_col);
            row.resize(max_col, String::new());
        }
        if ran_past {
            while rows.len() < max_row {
                rows.push(vec![String::new(); max_col]);
            }
        }
    }
    Ok(rows)
}

/// `<dimension ref="A1:C3"/>` as a 1-based (rows, cols) rectangle.
fn parse_dimension(reference: &str) -> Option<(u32, u32)> {
    let (start, end) = reference.split_once(':').unwrap_or((reference, reference));
    let row_of = |part: &str| -> Option<u32> {
        let digits: String = part.chars().skip_while(|c| c.is_ascii_alphabetic()).collect();
        digits.parse().ok()
    };
    let col_of = |part: &str| -> Option<u32> {
        let letters: String = part.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
        // col_index is 0-based; the rectangle width counts columns from 1.
        col_index(&letters).map(|c| c + 1)
    };
    let (rows, cols) = (row_of(end)?, col_of(end)?);
    let rows = rows.max(row_of(start).unwrap_or(rows));
    let cols = cols.max(col_of(start).unwrap_or(cols));
    Some((rows.max(1), cols.max(1)))
}

fn cell_text(
    kind: &str,
    value: &str,
    inline: &str,
    ctx: &SheetCtx,
    style: u32,
) -> Result<String> {
    match kind {
        "s" => {
            let idx: usize = value.trim().parse().map_err(|_| {
                Error::new(format!("shared string index \"{value}\" is not a number"))
            })?;
            ctx.strings.get(idx).cloned().ok_or_else(|| {
                Error::new(format!("shared string index {idx} is out of range"))
            })
        }
        "inlineStr" => Ok(inline.to_string()),
        "b" => Ok(bool_text(value.trim())),
        "n" | "" => Ok(number_text(value.trim(), ctx, style)),
        _ => Ok(value.trim().to_string()),
    }
}

/// openpyxl reads `t="b"` through `bool(int(value))`.
fn bool_text(value: &str) -> String {
    match value {
        "1" | "true" | "TRUE" | "True" => "True".to_string(),
        "0" | "false" | "FALSE" | "False" => "False".to_string(),
        other => match other.parse::<i64>() {
            Ok(n) => {
                if n != 0 {
                    "True".to_string()
                } else {
                    "False".to_string()
                }
            }
            Err(_) => other.to_string(),
        },
    }
}

/// A numeric cell: a date serial through the 1900/1904 rules when the style
/// says date, otherwise the number a float repr would print.
fn number_text(value: &str, ctx: &SheetCtx, style: u32) -> String {
    if value.is_empty() {
        return String::new();
    }
    let fmt_id = ctx.styles.xfs.get(style as usize).copied().unwrap_or(0);
    let is_date = builtin_is_date(fmt_id)
        || ctx
            .styles
            .custom
            .get(&fmt_id)
            .is_some_and(|code| format_is_date(code));
    if is_date {
        if let Ok(serial) = value.parse::<f64>() {
            return excel_serial_iso(serial, ctx.date1904);
        }
    }
    // openpyxl casts a cell with a decimal point or exponent to float and
    // prints the repr; everything else stays an int in its own digits.
    if value.contains('.') || value.contains('E') || value.contains('e') {
        match value.parse::<f64>() {
            Ok(x) => python_float_repr(x),
            Err(_) => value.to_string(),
        }
    } else {
        match value.parse::<i64>() {
            Ok(n) => n.to_string(),
            Err(_) => value.to_string(),
        }
    }
}

/// Python `round()`: halves go to the even neighbour.
fn py_round(x: f64) -> f64 {
    let r = x.round();
    if (x - x.trunc()).abs() == 0.5 && r % 2.0 != 0.0 {
        r - x.signum()
    } else {
        r
    }
}

/// Excel serial → ISO date, time, or datetime, the way openpyxl's
/// `from_excel` builds it: serials under one day are times, 1900 serials
/// 1-59 shift by a day (the leap-year bug), and the epoch is 1899-12-30 or
/// 1904-01-01.
fn excel_serial_iso(value: f64, date1904: bool) -> String {
    let day = value.floor();
    let fraction = value - day;
    let ms = py_round(fraction * 86_400_000.0) as i64;
    if (0.0..1.0).contains(&value) && ms < 86_400_000 {
        return time_iso(ms);
    }
    let mut day = day as i64;
    if !date1904 && (0.0..60.0).contains(&value) {
        day += 1;
    }
    let (ey, em, ed) = if date1904 { (1904, 1, 1) } else { (1899, 12, 30) };
    let days = days_from_civil(ey, em, ed) + day + ms / 86_400_000;
    let rem = ms % 86_400_000;
    let (y, m, d) = civil_from_days(days);
    let clock = time_iso(rem);
    format!("{y:04}-{m:02}-{d:02}T{clock}")
}

/// A millisecond count inside one day as `HH:MM:SS[.ffffff]`, a Python
/// `time.isoformat()`.
fn time_iso(ms: i64) -> String {
    let h = ms / 3_600_000;
    let m = (ms / 60_000) % 60;
    let s = (ms / 1_000) % 60;
    let us = (ms % 1_000) * 1000;
    if us == 0 {
        format!("{h:02}:{m:02}:{s:02}")
    } else {
        format!("{h:02}:{m:02}:{s:02}.{us:06}")
    }
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Python `repr()` of a finite float: the shortest digits that round-trip,
/// fixed notation between 1e-4 and 1e16, scientific with a signed two-digit
/// exponent outside that window.
fn python_float_repr(x: f64) -> String {
    if x == 0.0 {
        return if x.is_sign_negative() { "-0.0".into() } else { "0.0".into() };
    }
    let sci = format!("{x:e}");
    let (mant, exp) = sci.split_once('e').expect("{:e} always has an exponent");
    let e: i32 = exp.parse().expect("{:e} exponent is an integer");
    let neg = mant.starts_with('-');
    let digits: String = mant.chars().filter(|c| c.is_ascii_digit()).collect();
    let sign = if neg { "-" } else { "" };
    if (-4..16).contains(&e) {
        let mut out = String::from(sign);
        if e >= 0 {
            let int_len = e as usize + 1;
            if digits.len() > int_len {
                out.push_str(&digits[..int_len]);
                out.push('.');
                out.push_str(&digits[int_len..]);
            } else {
                out.push_str(&digits);
                for _ in digits.len()..int_len {
                    out.push('0');
                }
                out.push_str(".0");
            }
        } else {
            out.push_str("0.");
            for _ in 0..(-e - 1) {
                out.push('0');
            }
            out.push_str(&digits);
        }
        out
    } else {
        let mut out = String::from(sign);
        out.push_str(&digits[..1]);
        if digits.len() > 1 {
            out.push('.');
            out.push_str(&digits[1..]);
        }
        if e >= 0 {
            out.push_str(&format!("e+{e:02}"));
        } else {
            out.push_str(&format!("e-{:02}", -e));
        }
        out
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored zip with one worksheet, the workbook plumbing, and the given
    /// styles and sheet XML.
    fn package(sheet: &str, styles: Option<&str>) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new();
        zip.add_stored("[Content_Types].xml", CONTENT_TYPES).unwrap();
        zip.add_stored("_rels/.rels", ROOT_RELS).unwrap();
        zip.add_stored("xl/workbook.xml", WORKBOOK).unwrap();
        zip.add_stored("xl/_rels/workbook.xml.rels", WORKBOOK_RELS).unwrap();
        zip.add_stored("xl/styles.xml", styles.unwrap_or_default().as_bytes()).unwrap();
        zip.add_stored("xl/worksheets/sheet1.xml", sheet.as_bytes()).unwrap();
        zip.finish().unwrap()
    }

    fn sheet_xml(body: &str) -> String {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
             <worksheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"\
             xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">{body}</worksheet>"
        )
    }

    fn rows_of(sheet: &str, styles: Option<&str>) -> Vec<Vec<String>> {
        let bytes = package(sheet, styles);
        read_first_sheet(&bytes).unwrap().0
    }

    #[test]
    fn date_serials_become_iso_datetimes_and_times() {
        // cellXfs[1] = builtin 14 (mm-dd-yy), cellXfs[2] = custom "yyyy-mm-dd",
        // cellXfs[3] = builtin 20 (h:mm).
        let styles = concat!(
            "<?xml version=\"1.0\"?><styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\">",
            "<numFmts count=\"1\"><numFmt numFmtId=\"164\" formatCode=\"yyyy-mm-dd\"/></numFmts>",
            "<cellXfs count=\"4\"><xf numFmtId=\"0\"/><xf numFmtId=\"14\"/>",
            "<xf numFmtId=\"164\"/><xf numFmtId=\"20\"/></cellXfs></styleSheet>"
        );
        let sheet = sheet_xml(concat!(
            "<sheetData>",
            "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>45296</v></c></row>",
            "<row r=\"2\"><c r=\"A2\" s=\"2\"><v>45296</v></c></row>",
            "<row r=\"3\"><c r=\"A3\" s=\"3\"><v>0.6041666666666666</v></c></row>",
            "<row r=\"4\"><c r=\"A4\" s=\"2\"><v>45296.60416666666</v></c></row>",
            "</sheetData>"
        ));
        let rows = rows_of(&sheet, Some(styles));
        assert_eq!(rows[0][0], "2024-01-05T00:00:00");
        assert_eq!(rows[1][0], "2024-01-05T00:00:00");
        assert_eq!(rows[2][0], "14:30:00");
        assert_eq!(rows[3][0], "2024-01-05T14:30:00");
    }

    #[test]
    fn the_1900_leap_bug_shifts_serials_one_to_fifty_nine() {
        // Serial 1 is 1900-01-01, serial 59 is 1900-02-28, serial 61 is 1900-03-01.
        let sheet = sheet_xml(concat!(
            "<sheetData>",
            "<row r=\"1\"><c r=\"A1\" s=\"1\"><v>1</v></c></row>",
            "<row r=\"2\"><c r=\"A2\" s=\"1\"><v>59</v></c></row>",
            "<row r=\"3\"><c r=\"A3\" s=\"1\"><v>61</v></c></row>",
            "</sheetData>"
        ));
        let styles = "<?xml version=\"1.0\"?><styleSheet xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\"><cellXfs count=\"2\"><xf numFmtId=\"0\"/><xf numFmtId=\"14\"/></cellXfs></styleSheet>";
        let rows = rows_of(&sheet, Some(styles));
        assert_eq!(rows[0][0], "1900-01-01T00:00:00");
        assert_eq!(rows[1][0], "1900-02-28T00:00:00");
        assert_eq!(rows[2][0], "1900-03-01T00:00:00");
    }

    #[test]
    fn booleans_print_true_and_false() {
        let sheet = sheet_xml(concat!(
            "<sheetData>",
            "<row r=\"1\"><c r=\"A1\" t=\"b\"><v>1</v></c><c r=\"B1\" t=\"b\"><v>0</v></c></row>",
            "</sheetData>"
        ));
        let rows = rows_of(&sheet, None);
        assert_eq!(rows[0][0], "True");
        assert_eq!(rows[0][1], "False");
    }

    #[test]
    fn a_dimension_rectangle_pads_and_trims() {
        // Row 4 is outside the rectangle; rows 2-3 are short.
        let sheet = sheet_xml(concat!(
            "<dimension ref=\"A1:C3\"/>",
            "<sheetData>",
            "<row r=\"1\"><c r=\"A1\"><v>1</v></c><c r=\"B1\"><v>2</v></c><c r=\"C1\"><v>3</v></c><c r=\"D1\"><v>4</v></c></row>",
            "<row r=\"2\"><c r=\"A2\"><v>5</v></c></row>",
            "<row r=\"4\"><c r=\"A4\"><v>9</v></c></row>",
            "</sheetData>"
        ));
        let rows = rows_of(&sheet, None);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec!["1", "2", "3"], "D1 is outside the rectangle");
        assert_eq!(rows[1], vec!["5", "", ""]);
        assert_eq!(rows[2], vec!["", "", ""], "row 3 exists inside the rectangle");
    }

    #[test]
    fn without_a_dimension_rows_come_back_as_stored() {
        let sheet = sheet_xml(concat!(
            "<sheetData>",
            "<row r=\"1\"><c r=\"A1\"><v>1</v></c></row>",
            "<row r=\"2\"><c r=\"A2\"><v>2</v></c><c r=\"B2\"><v>3</v></c></row>",
            "</sheetData>"
        ));
        let rows = rows_of(&sheet, None);
        assert_eq!(rows, vec![vec!["1"], vec!["2", "3"]]);
    }

    #[test]
    fn floats_print_what_a_python_repr_prints() {
        let sheet = sheet_xml(concat!(
            "<sheetData>",
            "<row r=\"1\">",
            "<c r=\"A1\"><v>3.50</v></c>",
            "<c r=\"B1\"><v>1e5</v></c>",
            "<c r=\"C1\"><v>0.0001</v></c>",
            "<c r=\"D1\"><v>1e16</v></c>",
            "<c r=\"E1\"><v>-2.5</v></c>",
            "</row>",
            "</sheetData>"
        ));
        let rows = rows_of(&sheet, None);
        assert_eq!(rows[0], vec!["3.5", "100000.0", "0.0001", "1e+16", "-2.5"]);
    }

    #[test]
    fn the_active_sheet_is_read_not_always_the_first() {
        let mut zip = zip::ZipWriter::new();
        zip.add_stored("[Content_Types].xml", CONTENT_TYPES).unwrap();
        zip.add_stored("_rels/.rels", ROOT_RELS).unwrap();
        zip.add_stored(
            "xl/workbook.xml",
            concat!(
                "<?xml version=\"1.0\"?><workbook xmlns=\"http://schemas.openxmlformats.org/spreadsheetml/2006/main\" ",
                "xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\">",
                "<bookViews><workbookView activeTab=\"1\"/></bookViews>",
                "<sheets><sheet name=\"One\" sheetId=\"1\" r:id=\"rId1\"/>",
                "<sheet name=\"Two\" sheetId=\"2\" r:id=\"rId2\"/></sheets></workbook>"
            )
            .as_bytes(),
        )
        .unwrap();
        zip.add_stored(
            "xl/_rels/workbook.xml.rels",
            concat!(
                "<?xml version=\"1.0\"?><Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">",
                "<Relationship Id=\"rId1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet1.xml\"/>",
                "<Relationship Id=\"rId2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\" Target=\"worksheets/sheet2.xml\"/>",
                "</Relationships>"
            )
            .as_bytes(),
        )
        .unwrap();
        zip.add_stored("xl/styles.xml", STYLES).unwrap();
        zip.add_stored(
            "xl/worksheets/sheet1.xml",
            sheet_xml("<sheetData><row r=\"1\"><c r=\"A1\"><v>1</v></c></row></sheetData>").as_bytes(),
        )
        .unwrap();
        zip.add_stored(
            "xl/worksheets/sheet2.xml",
            sheet_xml("<sheetData><row r=\"1\"><c r=\"A1\"><v>22</v></c></row></sheetData>").as_bytes(),
        )
        .unwrap();
        let bytes = zip.finish().unwrap();
        let (rows, _) = read_first_sheet(&bytes).unwrap();
        assert_eq!(rows, vec![vec!["22"]]);
    }

    #[test]
    fn python_round_halves_go_to_the_even_neighbour() {
        assert_eq!(py_round(2.5), 2.0);
        assert_eq!(py_round(3.5), 4.0);
        assert_eq!(py_round(-2.5), -2.0);
        assert_eq!(py_round(0.4), 0.0);
        assert_eq!(py_round(1.6), 2.0);
    }
}
