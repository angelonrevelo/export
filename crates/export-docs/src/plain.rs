//! The DOCX-to-text renderer that mirrors pandoc's plain writer.
//!
//! Pandoc reads the document into blocks and writes them with `--wrap=auto`
//! at 72 columns: one blank line between blocks, paragraphs re-wrapped,
//! list items carrying their marker with a hanging indent, quotes indented
//! two spaces, and runs of whitespace collapsed to one space. The old
//! streaming text path put a single newline between every paragraph and
//! never wrapped, which matched none of that.
//!
//! Tables are the one known gap: pandoc draws them as a dashed grid with
//! content-based column widths. Here they keep the legacy tab-separated
//! rows, so a document with a table still converts but not to pandoc's
//! bytes.

use std::collections::HashMap;
use std::io::Write;

use crate::error::{Error, Result};
use crate::xml::{Attr, XmlSink};

const WRAP_COLUMNS: usize = 72;

/// Word's `numbering.xml`: which abstract definition each numId uses, and
/// each level's format code and marker text.
#[derive(Default)]
pub struct Numbering {
    abstract_of_num: HashMap<u32, u32>,
    /// (abstract id, level) -> (format code, marker text, start value).
    levels: HashMap<(u32, u32), (String, String, u32)>,
    /// (numId, level) -> start override from `<w:lvlOverride>`.
    start_override: HashMap<(u32, u32), u32>,
}

pub fn parse_numbering(xml: &[u8]) -> Numbering {
    let mut sink = NumberingSink {
        numbering: Numbering::default(),
        skip: 0,
        cur_abstract: None,
        cur_num: None,
        cur_level: None,
        fmt: String::new(),
        lvl_text: String::new(),
        cur_start: 0,
        in_override: false,
        cur_override_level: None,
    };
    let _ = crate::xml::pull(xml, &mut sink);
    sink.numbering
}

struct NumberingSink {
    numbering: Numbering,
    skip: u32,
    cur_abstract: Option<u32>,
    cur_num: Option<u32>,
    cur_level: Option<u32>,
    fmt: String,
    lvl_text: String,
    cur_start: u32,
    in_override: bool,
    cur_override_level: Option<u32>,
}

impl XmlSink for NumberingSink {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        if self.skip > 0 {
            self.skip += 1;
            return Ok(());
        }
        let local = name.rsplit_once(':').map(|p| p.1).unwrap_or(name);
        match local {
            "abstractNum" => {
                self.cur_abstract = xml_attr(attrs, "abstractNumId").and_then(|v| v.parse().ok());
            }
            "num" => {
                self.cur_num = xml_attr(attrs, "numId").and_then(|v| v.parse().ok());
                self.in_override = false;
            }
            "lvlOverride" => {
                self.in_override = true;
                self.cur_override_level =
                    xml_attr(attrs, "ilvl").and_then(|v| v.parse().ok());
            }
            "startOverride" => {
                if let (Some(num), Some(level)) = (self.cur_num, self.cur_override_level) {
                    if let Some(start) = xml_attr(attrs, "val").and_then(|v| v.parse().ok()) {
                        self.numbering
                            .start_override
                            .insert((num, level), start);
                    }
                }
            }
            "lvl" => {
                self.cur_level = xml_attr(attrs, "ilvl").and_then(|v| v.parse().ok());
                self.fmt.clear();
                self.lvl_text.clear();
                self.in_override = false;
            }
            "numFmt" => self.fmt = xml_attr(attrs, "val").unwrap_or("").to_string(),
            "lvlText" => self.lvl_text = xml_attr(attrs, "val").unwrap_or("").to_string(),
            "start" => {
                if let Some(start) = xml_attr(attrs, "val").and_then(|v| v.parse().ok()) {
                    self.cur_start = start;
                }
            }
            "abstractNumId" => {
                if let (Some(num), Some(abs)) = (
                    self.cur_num,
                    xml_attr(attrs, "val").and_then(|v| v.parse().ok()),
                ) {
                    self.numbering.abstract_of_num.insert(num, abs);
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn end(&mut self, name: &str) -> Result<()> {
        if self.skip > 0 {
            self.skip -= 1;
            return Ok(());
        }
        let local = name.rsplit_once(':').map(|p| p.1).unwrap_or(name);
        match local {
            "abstractNum" => {
                self.cur_abstract = None;
                self.cur_level = None;
            }
            "num" => self.cur_num = None,
            "lvl" => {
                if let (Some(abs), Some(level)) = (self.cur_abstract, self.cur_level) {
                    self.numbering.levels.insert(
                        (abs, level),
                        (
                            std::mem::take(&mut self.fmt),
                            std::mem::take(&mut self.lvl_text),
                            std::mem::take(&mut self.cur_start).max(1),
                        ),
                    );
                }
                self.cur_level = None;
            }
            _ => {}
        }
        Ok(())
    }

    fn text(&mut self, _text: &str) -> Result<()> {
        Ok(())
    }
}

fn xml_attr<'a>(attrs: &'a [Attr], local: &str) -> Option<&'a str> {
    let suffix = format!(":{local}");
    attrs
        .iter()
        .find(|a| a.name == local || a.name.ends_with(&suffix))
        .map(|a| a.value.as_str())
}

/// One rendered block. Consecutive list items share one block joined by a
/// single newline (pandoc's tight list); consecutive quotes share one too,
/// with a blank line inside.
enum Block {
    Text(String),
    Quote(String),
    Table(String),
    List(Vec<String>),
}

/// The DOCX event sink behind `docx_to_text`.
pub struct TextSink<'a, W> {
    out: &'a mut W,
    skip: u32,
    in_t: bool,
    /// Text of the paragraph being read; hard breaks are '\n'.
    para: String,
    style: String,
    numbering: Numbering,
    list: Option<(u32, u32)>,
    /// (numId, ilvl) -> next number, valid inside one list run.
    counters: HashMap<(u32, u32), u32>,
    blocks: Vec<Block>,
    /// Open list levels of the current run: (numId, ilvl, indent, width).
    levels: Vec<(u32, u32, usize, usize)>,
    /// Table depth. Cells stream in the legacy tab-separated form.
    table: u32,
    cells: usize,
    row: String,
}

impl<'a, W: Write> TextSink<'a, W> {
    pub fn new(out: &'a mut W, numbering: Numbering) -> Self {
        Self {
            out,
            skip: 0,
            in_t: false,
            para: String::new(),
            style: String::new(),
            numbering,
            list: None,
            counters: HashMap::new(),
            blocks: Vec::new(),
            levels: Vec::new(),
            table: 0,
            cells: 0,
            row: String::new(),
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.out
            .write_all(bytes)
            .map_err(|e| Error::new(e.to_string()))
    }

    /// A paragraph ended. In tables the text joins the row; otherwise it
    /// becomes a block.
    fn close_para(&mut self) -> Result<()> {
        let para = std::mem::take(&mut self.para);
        let style = std::mem::take(&mut self.style);
        let list = self.list.take();
        if self.table > 0 {
            self.row.push_str(&para);
            return Ok(());
        }
        let lines: Vec<String> = para
            .split('\n')
            .map(collapse)
            .filter(|line| !line.is_empty())
            .collect();
        if lines.is_empty() {
            self.counters.clear();
            self.levels.clear();
            return Ok(());
        }
        let is_list = matches!(&list, Some((num_id, _)) if self.numbering.is_list(*num_id));
        if !is_list {
            self.counters.clear();
            self.levels.clear();
            let quote = style.eq_ignore_ascii_case("blocktext") || style.eq_ignore_ascii_case("quote");
            self.levels.clear();
            if quote {
                self.push_quote(&lines);
            } else {
                let wrapped: Vec<String> = lines
                    .iter()
                    .flat_map(|line| wrap_text(line, WRAP_COLUMNS))
                    .collect();
                self.blocks.push(Block::Text(wrapped.join("\n")));
            }
            return Ok(());
        }
        let (num_id, ilvl) = list.expect("checked above");
        // Pandoc nests a list by indenting under the parent's marker: a
        // deeper level continues at the parent's indent plus its marker
        // width, a shallower one returns to the indent recorded for that
        // level. The run continues through nesting and through a same-numId
        // return; a different numId at the same level starts a new list,
        // which the blank-line rule then separates from the last one.
        let mut indent = 0usize;
        let mut continues = false;
        loop {
            match self.levels.last() {
                Some(&(_, top_level, top_indent, top_width)) if ilvl > top_level => {
                    indent = top_indent + top_width;
                    self.levels.push((num_id, ilvl, indent, 0));
                    continues = true;
                    break;
                }
                Some(&(_, top_level, _, _)) if ilvl < top_level => {
                    self.levels.pop();
                }
                Some(&(top_id, top_level, top_indent, _)) if ilvl == top_level => {
                    indent = top_indent;
                    continues = top_id == num_id;
                    if !continues {
                        self.counters.clear();
                        self.levels.pop();
                        self.levels.push((num_id, ilvl, top_indent, 0));
                    }
                    break;
                }
                Some(_) => continue,
                None => {
                    self.levels.push((num_id, ilvl, 0, 0));
                    break;
                }
            }
        }
        let (marker, marker_width) = self.marker_for(num_id, ilvl);
        if let Some(last) = self.levels.last_mut() {
            last.3 = marker_width;
        }
        let level_indent = " ".repeat(indent);
        let content_width = WRAP_COLUMNS.saturating_sub(level_indent.len() + marker_width);
        let mut item = String::new();
        for line in &lines {
            for (i, piece) in wrap_text(line, content_width).into_iter().enumerate() {
                if !item.is_empty() {
                    item.push('\n');
                }
                if i == 0 {
                    item.push_str(&level_indent);
                    item.push_str(&marker);
                    item.push_str(&" ".repeat(marker_width - marker.chars().count()));
                } else {
                    item.push_str(&level_indent);
                    item.push_str(&" ".repeat(marker_width));
                }
                item.push_str(&piece);
            }
        }
        let run_continues = continues
            && matches!(self.blocks.last(), Some(Block::List { .. }));
        if run_continues {
            if let Some(Block::List(items)) = self.blocks.last_mut() {
                items.push(item);
            }
        } else {
            self.blocks.push(Block::List(vec![item]));
        }
        Ok(())
    }

    /// Word's marker for this numbering level, with its column width.
    fn marker_for(&mut self, num_id: u32, ilvl: u32) -> (String, usize) {
        let (fmt, lvl_text, _) = self
            .numbering
            .level(num_id, ilvl)
            .cloned()
            .unwrap_or_else(|| ("decimal".to_string(), "%1.".to_string(), 1));
        if fmt == "bullet" {
            return ("-".to_string(), 2);
        }
        let start = self.numbering.start(num_id, ilvl);
        let counter = self
            .counters
            .entry((num_id, ilvl))
            .or_insert(start.saturating_sub(1));
        *counter += 1;
        let n = *counter;
        self.counters
            .retain(|(id, level), _| *id != num_id || *level <= ilvl);
        let number = render_number(&fmt, n);
        let (paren, suffix) = delim_of(&lvl_text);
        let marker = match (paren, suffix) {
            (true, ')') => format!("({number})"),
            (_, ')') => format!("{number})"),
            _ => format!("{number}."),
        };
        let width = (marker.chars().count() + 1).max(4);
        (marker, width)
    }

    /// Consecutive quote paragraphs are one blockquote: paragraphs inside
    /// it are separated by a blank line, every line indented two spaces.
    fn push_quote(&mut self, lines: &[String]) {
        let quoted: Vec<String> = lines
            .iter()
            .flat_map(|line| wrap_text(line, WRAP_COLUMNS - 2))
            .map(|line| format!("  {line}"))
            .collect();
        match self.blocks.last_mut() {
            Some(Block::Quote(prev)) => {
                prev.push_str("\n\n");
                prev.push_str(&quoted.join("\n"));
            }
            _ => self.blocks.push(Block::Quote(quoted.join("\n"))),
        }
    }

    /// Flush the block list to the output. Called once, after the parse.
    pub fn finish_document(&mut self) -> Result<()> {
        let mut out = String::new();
        for (i, block) in std::mem::take(&mut self.blocks).into_iter().enumerate() {
            if i > 0 {
                out.push_str("\n\n");
            }
            match block {
                Block::Text(text) => out.push_str(&text),
                Block::Quote(text) => out.push_str(&text),
                Block::Table(text) => out.push_str(&text),
                Block::List(items) => out.push_str(&items.join("\n")),
            }
        }
        if !out.is_empty() {
            out.push('\n');
        }
        self.write(out.as_bytes())
    }
}

/// Pandoc's plain writer collapses whitespace runs to one space.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Greedy wrap: as many space-separated words as fit within `width`; a word
/// longer than the whole width stands on its own line.
fn wrap_text(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split(' ').filter(|w| !w.is_empty()) {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
    }
    lines.push(current);
    lines
}

fn render_number(fmt: &str, n: u32) -> String {
    match fmt {
        "lowerLetter" => to_alpha(n, b'a'),
        "upperLetter" => to_alpha(n, b'A'),
        "lowerRoman" => to_roman(n).to_lowercase(),
        "upperRoman" => to_roman(n).to_string(),
        _ => n.to_string(),
    }
}

/// 1 = a, 26 = z, 27 = aa.
fn to_alpha(mut n: u32, base: u8) -> String {
    let mut out = Vec::new();
    while n > 0 {
        let rem = ((n - 1) % 26) as u8;
        out.push(base + rem);
        n = (n - 1) / 26;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

fn to_roman(mut n: u32) -> String {
    const PAIRS: [(u32, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut out = String::new();
    for (value, glyph) in PAIRS {
        while n >= value {
            out.push_str(glyph);
            n -= value;
        }
    }
    out
}

/// `"%1."` and `"%1)"` put the delimiter after the number.
fn delim_of(lvl_text: &str) -> (bool, char) {
    let Some(pos) = lvl_text.find("%1") else {
        return (false, '.');
    };
    let prefix = lvl_text[..pos].ends_with('(');
    let suffix = lvl_text[pos + 2..].chars().next().unwrap_or('.');
    let suffix = if suffix == ')' { ')' } else { '.' };
    (prefix, suffix)
}

impl Numbering {
    fn is_list(&self, num_id: u32) -> bool {
        self.abstract_of_num.contains_key(&num_id)
    }

    fn level(&self, num_id: u32, ilvl: u32) -> Option<&(String, String, u32)> {
        let abs = self.abstract_of_num.get(&num_id)?;
        self.levels.get(&(*abs, ilvl))
    }

    /// The number the first item of this level counts from.
    fn start(&self, num_id: u32, ilvl: u32) -> u32 {
        self.start_override
            .get(&(num_id, ilvl))
            .copied()
            .or_else(|| self.level(num_id, ilvl).map(|(_, _, start)| *start))
            .unwrap_or(1)
            .max(1)
    }
}

impl<W: Write> XmlSink for TextSink<'_, W> {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        if self.skip > 0 {
            self.skip += 1;
            return Ok(());
        }
        let Some(loc) = crate::docx::word_local(name) else {
            return Ok(());
        };
        match loc {
            "del" | "delText" | "instrText" => self.skip = 1,
            "p" => {
                self.para.clear();
                self.style.clear();
                self.list = None;
            }
            "pStyle" => {
                if let Some(v) = xml_attr(attrs, "val") {
                    self.style = v.to_string();
                }
            }
            "numPr" => self.list = Some((0, 0)),
            "ilvl" => {
                if let Some(pair) = self.list.as_mut() {
                    pair.1 = xml_attr(attrs, "val").and_then(|v| v.parse().ok()).unwrap_or(0);
                }
            }
            "numId" => {
                if let Some(pair) = self.list.as_mut() {
                    pair.0 = xml_attr(attrs, "val").and_then(|v| v.parse().ok()).unwrap_or(0);
                }
            }
            "t" => self.in_t = true,
            "tab" => self.para.push('\t'),
            "br" | "cr" => self.para.push('\n'),
            "tbl" => {
                self.table += 1;
                self.row.clear();
            }
            "tr" => self.cells = 0,
            "tc" => {
                if self.cells > 0 {
                    self.row.push('\t');
                }
                self.cells += 1;
            }
            _ => {}
        }
        Ok(())
    }

    fn end(&mut self, name: &str) -> Result<()> {
        if self.skip > 0 {
            self.skip -= 1;
            return Ok(());
        }
        let Some(loc) = crate::docx::word_local(name) else {
            return Ok(());
        };
        match loc {
            "t" => self.in_t = false,
            "p" => self.close_para()?,
            "tr" => {
                if self.table > 0 {
                    self.row.push('\n');
                    let line = std::mem::take(&mut self.row);
                    match self.blocks.last_mut() {
                        Some(Block::Table(prev)) => prev.push_str(&line),
                        _ => self.blocks.push(Block::Table(line)),
                    }
                }
                self.cells = 0;
            }
            "tbl" => {
                self.table = self.table.saturating_sub(1);
            }
            _ => {}
        }
        Ok(())
    }

    fn text(&mut self, text: &str) -> Result<()> {
        if self.skip > 0 || !self.in_t {
            return Ok(());
        }
        self.para.push_str(text);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_keeps_words_whole_and_fills_lines() {
        let lines = wrap_text(
            "aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv",
            20,
        );
        for line in &lines {
            assert!(line.chars().count() <= 20, "{line}");
        }
        assert_eq!(lines.join(" "), "aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv");
    }

    #[test]
    fn a_word_longer_than_the_line_stands_alone() {
        let lines = wrap_text("short veryveryverylongword tail", 10);
        assert_eq!(lines, vec!["short", "veryveryverylongword", "tail"]);
    }

    #[test]
    fn letters_and_romans_count_the_word_way() {
        assert_eq!(to_alpha(1, b'a'), "a");
        assert_eq!(to_alpha(26, b'a'), "z");
        assert_eq!(to_alpha(27, b'a'), "aa");
        assert_eq!(to_alpha(28, b'A'), "AB");
        assert_eq!(to_roman(4), "IV");
        assert_eq!(to_roman(1990), "MCMXC");
    }

    #[test]
    fn the_marker_text_picks_the_delimiter() {
        assert_eq!(delim_of("%1."), (false, '.'));
        assert_eq!(delim_of("%1)"), (false, ')'));
        assert_eq!(delim_of("(%1)"), (true, ')'));
        assert_eq!(delim_of(" "), (false, '.'));
    }

    #[test]
    fn markers_pad_to_four_columns_or_one_space() {
        // "1." is two chars: pad to 4. "(10)" is four: one space.
        let one = render_number("decimal", 1);
        assert_eq!(format!("{one}."), "1.");
        let ten = render_number("decimal", 10);
        assert_eq!(format!("{ten}."), "10.");
        let width = (format!("({ten})").chars().count() + 1).max(4);
        assert_eq!(width, 5);
    }

    #[test]
    fn numbering_reads_formats_starts_and_overrides() {
        let xml = concat!(
            "<?xml version=\"1.0\"?><w:numbering xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">",
            "<w:abstractNum w:abstractNumId=\"9\"><w:lvl w:ilvl=\"0\">",
            "<w:start w:val=\"10\"/><w:numFmt w:val=\"decimal\"/><w:lvlText w:val=\"%1.\"/></w:lvl>",
            "<w:lvl w:ilvl=\"1\"><w:start w:val=\"1\"/><w:numFmt w:val=\"lowerLetter\"/><w:lvlText w:val=\"%1.\"/></w:lvl>",
            "</w:abstractNum>",
            "<w:abstractNum w:abstractNumId=\"11\"><w:lvl w:ilvl=\"0\">",
            "<w:start w:val=\"1\"/><w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"&#8226;\"/></w:lvl></w:abstractNum>",
            "<w:num w:numId=\"100\"><w:abstractNumId w:val=\"9\"/></w:num>",
            "<w:num w:numId=\"200\"><w:abstractNumId w:val=\"9\"/>",
            "<w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"5\"/></w:lvlOverride></w:num>",
            "<w:num w:numId=\"300\"><w:abstractNumId w:val=\"11\"/></w:num>",
            "</w:numbering>"
        );
        let numbering = parse_numbering(xml.as_bytes());
        assert_eq!(numbering.level(100, 0).unwrap().2, 10);
        assert_eq!(numbering.level(100, 1).unwrap().1, "%1.");
        assert_eq!(numbering.level(100, 1).unwrap().0, "lowerLetter");
        assert_eq!(numbering.start(100, 0), 10);
        assert_eq!(numbering.start(200, 0), 5, "startOverride wins");
        assert_eq!(numbering.start(300, 0), 1);
        assert!(numbering.is_list(100));
        assert!(!numbering.is_list(999));
    }
}
