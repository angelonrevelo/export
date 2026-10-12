//! Cross-converts the three formats without holding the table.
//!
//! CSV and TSV share one reader. JSON is an array of objects when the
//! delimited input has a header, or an array of arrays when it does not.
//! The other direction reads the shape off the first element.

use std::io::{BufRead, Write};

use crate::delim::{self, DelimReader};
use crate::error::{Error, Result};
use crate::json::{self, JsonReader, RowShape};

/// Header names packed in one buffer, with byte spans into that buffer.
struct Names {
    bytes: Vec<u8>,
    spans: Vec<(usize, usize)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Csv,
    Tsv,
    Json,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Csv => "csv",
            Format::Tsv => "tsv",
            Format::Json => "json",
        }
    }

    fn delim(self) -> Option<u8> {
        match self {
            Format::Csv => Some(b','),
            Format::Tsv => Some(b'\t'),
            Format::Json => None,
        }
    }
}

/// `header` applies when a delimited file is read into JSON.
///
/// It is ignored for JSON input: an object array already carries its names,
/// and an array of arrays does not have them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub header: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { header: true }
    }
}

/// What one conversion did.
///
/// `rows` is the number of records written. A JSON array counts elements. A
/// CSV or TSV file counts lines, including the header line written for an
/// object array. `lossy` is how many input records contained bytes that were
/// not UTF-8 and were replaced with U+FFFD.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    pub rows: u64,
    pub lossy: u64,
}

pub fn convert<R: BufRead, W: Write>(
    from: Format,
    to: Format,
    input: R,
    mut output: W,
    options: Options,
) -> Result<Stats> {
    match (from.delim(), to.delim()) {
        (Some(src_delim), Some(dst_delim)) => {
            delim_to_delim(input, &mut output, src_delim, dst_delim)
        }
        (Some(src_delim), None) => delim_to_json(input, &mut output, src_delim, options.header),
        (None, Some(dst_delim)) => json_to_delim(input, &mut output, dst_delim),
        (None, None) => json_to_json(input, &mut output),
    }
}

pub fn convert_bytes(
    from: Format,
    to: Format,
    input: &[u8],
    options: Options,
) -> Result<(Vec<u8>, Stats)> {
    let mut out = Vec::new();
    let stats = convert(from, to, std::io::Cursor::new(input), &mut out, options)?;
    Ok((out, stats))
}

fn delim_to_delim<R: BufRead, W: Write>(
    input: R,
    output: &mut W,
    src_delim: u8,
    dst_delim: u8,
) -> Result<Stats> {
    let mut reader = DelimReader::new(input, src_delim);
    let mut rows = 0u64;
    while reader.next_row()? {
        delim::write_row(
            output,
            dst_delim,
            (0..reader.len()).map(|i| reader.field(i)),
        )?;
        rows += 1;
    }
    Ok(Stats {
        rows,
        lossy: reader.lossy(),
    })
}

fn delim_to_json<R: BufRead, W: Write>(
    input: R,
    output: &mut W,
    delim: u8,
    header: bool,
) -> Result<Stats> {
    let mut reader = DelimReader::new(input, delim);
    output.write_all(b"[")?;
    if !header {
        let mut rows = 0u64;
        let mut width: Option<usize> = None;
        let mut first = true;
        while reader.next_row()? {
            let n = reader.len();
            match width {
                None => width = Some(n),
                Some(w) if n != w => {
                    return Err(Error::new(format!(
                        "row {}: {n} columns, header has {w}",
                        rows + 1
                    )));
                }
                Some(_) => {}
            }
            if !first {
                output.write_all(b",")?;
            }
            first = false;
            write_json_array(output, &reader)?;
            rows += 1;
        }
        output.write_all(b"]\n")?;
        return Ok(Stats {
            rows,
            lossy: reader.lossy(),
        });
    }

    if !reader.next_row()? {
        output.write_all(b"]\n")?;
        return Ok(Stats {
            rows: 0,
            lossy: reader.lossy(),
        });
    }
    let width = reader.len();
    let names = header_names(&reader, width)?;
    let mut rows = 0u64;
    let mut first = true;
    while reader.next_row()? {
        if reader.len() != width {
            return Err(Error::new(format!(
                "row {}: {} columns, header has {width}",
                rows + 2,
                reader.len()
            )));
        }
        if !first {
            output.write_all(b",")?;
        }
        first = false;
        output.write_all(b"{")?;
        for (col, (a, b)) in names.spans.iter().enumerate() {
            if col > 0 {
                output.write_all(b",")?;
            }
            let key = std::str::from_utf8(&names.bytes[*a..*b]).expect("header stored as UTF-8");
            json::write_string(output, key)?;
            output.write_all(b":")?;
            json::write_string(output, reader.field(col))?;
        }
        output.write_all(b"}")?;
        rows += 1;
    }
    output.write_all(b"]\n")?;
    Ok(Stats {
        rows,
        lossy: reader.lossy(),
    })
}

fn header_names(reader: &DelimReader<impl BufRead>, width: usize) -> Result<Names> {
    let mut names = Names {
        bytes: Vec::new(),
        spans: Vec::with_capacity(width),
    };
    for i in 0..width {
        let name = reader.field(i).trim();
        if names
            .spans
            .iter()
            .any(|&(a, b)| &names.bytes[a..b] == name.as_bytes())
        {
            return Err(Error::new(format!("duplicate column name \"{name}\"")));
        }
        let start = names.bytes.len();
        names.bytes.extend_from_slice(name.as_bytes());
        names.spans.push((start, names.bytes.len()));
    }
    Ok(names)
}

fn write_json_array<R: BufRead>(out: &mut impl Write, reader: &DelimReader<R>) -> Result<()> {
    out.write_all(b"[")?;
    for i in 0..reader.len() {
        if i > 0 {
            out.write_all(b",")?;
        }
        json::write_string(out, reader.field(i))?;
    }
    out.write_all(b"]")?;
    Ok(())
}

fn json_to_delim<R: BufRead, W: Write>(input: R, output: &mut W, delim: u8) -> Result<Stats> {
    let mut reader = JsonReader::new(input);
    let mut rows = 0u64;
    let mut width: Option<usize> = None;
    let mut header = Names {
        bytes: Vec::new(),
        spans: Vec::new(),
    };
    let mut index = 0u64;
    while reader.next_row()? {
        index += 1;
        match reader.shape() {
            Some(RowShape::Array) => {
                let n = reader.len();
                match width {
                    None => width = Some(n),
                    Some(w) if n != w => {
                        return Err(Error::new(format!(
                            "row {index}: {n} columns, header has {w}"
                        )));
                    }
                    Some(_) => {}
                }
                delim::write_row(output, delim, (0..reader.len()).map(|i| reader.value(i)))?;
                rows += 1;
            }
            Some(RowShape::Object) => {
                if header.spans.is_empty() && rows == 0 {
                    collect_object_header(&reader, &mut header);
                    delim::write_row(output, delim, header.iter())?;
                    rows += 1;
                } else {
                    reject_extra_keys(&reader, &header, index)?;
                }
                write_object_cells(output, delim, &reader, &header)?;
                rows += 1;
            }
            None => {}
        }
    }
    Ok(Stats {
        rows,
        lossy: reader.lossy(),
    })
}

fn json_to_json<R: BufRead, W: Write>(input: R, output: &mut W) -> Result<Stats> {
    let mut reader = JsonReader::new(input);
    output.write_all(b"[")?;
    let mut rows = 0u64;
    let mut first = true;
    let mut header = Names {
        bytes: Vec::new(),
        spans: Vec::new(),
    };
    let mut header_locked = false;
    let mut width: Option<usize> = None;
    let mut index = 0u64;
    while reader.next_row()? {
        index += 1;
        if !first {
            output.write_all(b",")?;
        }
        first = false;
        match reader.shape() {
            Some(RowShape::Array) => {
                let n = reader.len();
                match width {
                    None => width = Some(n),
                    Some(w) if n != w => {
                        return Err(Error::new(format!(
                            "row {index}: {n} columns, header has {w}"
                        )));
                    }
                    Some(_) => {}
                }
                output.write_all(b"[")?;
                for i in 0..n {
                    if i > 0 {
                        output.write_all(b",")?;
                    }
                    json::write_string(output, reader.value(i))?;
                }
                output.write_all(b"]")?;
                rows += 1;
            }
            Some(RowShape::Object) => {
                if !header_locked {
                    collect_object_header(&reader, &mut header);
                    header_locked = true;
                } else {
                    reject_extra_keys(&reader, &header, index)?;
                }
                output.write_all(b"{")?;
                for (col, key) in header.iter().enumerate() {
                    if col > 0 {
                        output.write_all(b",")?;
                    }
                    json::write_string(output, key)?;
                    output.write_all(b":")?;
                    let value = match object_index(&reader, key) {
                        Some(i) => reader.value(i),
                        None => "",
                    };
                    json::write_string(output, value)?;
                }
                output.write_all(b"}")?;
                rows += 1;
            }
            None => {}
        }
    }
    output.write_all(b"]\n")?;
    Ok(Stats {
        rows,
        lossy: reader.lossy(),
    })
}

impl Names {
    fn iter(&self) -> impl Iterator<Item = &str> {
        self.spans
            .iter()
            .map(|&(a, b)| std::str::from_utf8(&self.bytes[a..b]).expect("header stored as UTF-8"))
    }
}

fn collect_object_header(reader: &JsonReader<impl BufRead>, names: &mut Names) {
    for i in 0..reader.len() {
        let key = reader.key(i);
        if find_key(names, key).is_some() {
            continue;
        }
        let start = names.bytes.len();
        names.bytes.extend_from_slice(key.as_bytes());
        names.spans.push((start, names.bytes.len()));
    }
}

fn reject_extra_keys(reader: &JsonReader<impl BufRead>, names: &Names, index: u64) -> Result<()> {
    for i in 0..reader.len() {
        let key = reader.key(i);
        if find_key(names, key).is_none() {
            return Err(Error::new(format!(
                "row {index}: key \"{key}\" is not in the first object"
            )));
        }
    }
    Ok(())
}

fn write_object_cells<W: Write>(
    output: &mut W,
    delim: u8,
    reader: &JsonReader<impl BufRead>,
    names: &Names,
) -> Result<()> {
    for (col, key) in names.iter().enumerate() {
        if col > 0 {
            output.write_all(&[delim])?;
        }
        let value = match object_index(reader, key) {
            Some(i) => reader.value(i),
            None => "",
        };
        delim::write_field(output, delim, value)?;
    }
    output.write_all(b"\n")?;
    Ok(())
}

fn object_index<R: BufRead>(reader: &JsonReader<R>, key: &str) -> Option<usize> {
    let mut found = None;
    for i in 0..reader.len() {
        if reader.key(i) == key {
            found = Some(i);
        }
    }
    found
}

fn find_key(names: &Names, key: &str) -> Option<usize> {
    names
        .spans
        .iter()
        .position(|&(a, b)| &names.bytes[a..b] == key.as_bytes())
}
