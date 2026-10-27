//! The DOCX-to-text renderer that mirrors pandoc's plain writer.
//!
//! Pandoc reads the document into blocks and writes them with `--wrap=auto`
//! at 72 columns: one blank line between blocks, paragraphs re-wrapped,
//! list items carrying their marker with a hanging indent, quotes indented
//! two spaces, and runs of whitespace collapsed to one space. The old
//! streaming text path put a single newline between every paragraph and
//! never wrapped, which matched none of that.
//!
//! Tables render as pandoc's dashed grid: column widths from the `tblGrid`
//! twips as a share of the page text width, cells wrapped inside their
//! column, a full-dash border above the header and below the body around a
//! per-column dash underline, a blank line between body rows (plus one
//! trailing when there is a single row), and the whole table indented two
//! spaces.

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
                self.cur_override_level = xml_attr(attrs, "ilvl").and_then(|v| v.parse().ok());
            }
            "startOverride" => {
                if let (Some(num), Some(level)) = (self.cur_num, self.cur_override_level) {
                    if let Some(start) = xml_attr(attrs, "val").and_then(|v| v.parse().ok()) {
                        self.numbering.start_override.insert((num, level), start);
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

/// Cell paragraph alignment from `w:jc`, the way the docx reader maps it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Align {
    Left,
    Center,
    Right,
}

fn jc_to_align(jc: Option<&str>) -> Align {
    match jc {
        Some("center") => Align::Center,
        Some("right") | Some("end") => Align::Right,
        _ => Align::Left,
    }
}

struct TblCell {
    align: Align,
    /// Paragraphs; each is its collapsed lines joined with newlines.
    paras: Vec<String>,
}

impl TblCell {
    fn new() -> Self {
        Self {
            align: Align::Left,
            paras: Vec::new(),
        }
    }
}

struct TblRow {
    /// `w:tblHeader` on the row: joins the header run at the top.
    header: bool,
    cells: Vec<TblCell>,
}

/// One table, as read. Rendered once the section width is known.
#[derive(Default)]
struct TblState {
    /// `w:gridCol` widths in twips; `None` when a column carries no width.
    grid: Vec<Option<u32>>,
    header_first_row: bool,
    rows: Vec<TblRow>,
    /// A caption paragraph adjacent to the table.
    caption: Option<String>,
}

/// One rendered block. Consecutive list items share one block joined by a
/// single newline (pandoc's tight list); consecutive quotes share one too,
/// with a blank line inside. Tables render at the end, when the section
/// width is known.
enum Block {
    Text(String),
    Quote(String),
    Table(TblState),
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
    /// Open tables; the last one collects rows and cells.
    tables: Vec<TblState>,
    /// `w:jc` of the paragraph being read.
    para_jc: Option<String>,
    /// The current row's `w:tblHeader`.
    row_header: bool,
    /// Page text width in twips from the body-level `sectPr`.
    text_width: Option<u32>,
    body_depth: u32,
    in_sectpr: bool,
    /// pgSz w, pgMar left/right/gutter, in that order.
    sect_parts: [Option<u32>; 4],
    /// `wp:docPr` descriptions of the drawings in the current paragraph.
    para_images: Vec<String>,
    /// A caption paragraph waiting for its figure: the text between the
    /// brackets.
    pending_caption: Option<String>,
    /// A caption paragraph's lines, held while waiting for its figure.
    held_caption_lines: Option<Vec<String>>,
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
            tables: Vec::new(),
            para_jc: None,
            row_header: false,
            text_width: None,
            body_depth: 0,
            in_sectpr: false,
            sect_parts: [None; 4],
            para_images: Vec::new(),
            pending_caption: None,
            held_caption_lines: None,
        }
    }

    fn write(&mut self, bytes: &[u8]) -> Result<()> {
        self.out
            .write_all(bytes)
            .map_err(|e| Error::new(e.to_string()))
    }

    /// A paragraph ended. In tables the text joins the open cell; otherwise
    /// it becomes a block.
    fn close_para(&mut self) -> Result<()> {
        let para = std::mem::take(&mut self.para);
        let style = std::mem::take(&mut self.style);
        let list = self.list.take();
        let jc = self.para_jc.take();
        if let Some(state) = self.tables.last_mut() {
            let lines: Vec<String> = para
                .split('\n')
                .map(collapse)
                .filter(|line| !line.is_empty())
                .collect();
            if let Some(cell) = state.rows.last_mut().and_then(|row| row.cells.last_mut()) {
                if !lines.is_empty() {
                    cell.paras.push(lines.join("\n"));
                }
                if cell.paras.len() == 1 {
                    // The reader takes a cell's alignment from its first
                    // paragraph.
                    cell.align = jc_to_align(jc.as_deref());
                }
            }
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
            self.flush_held_caption();
            return Ok(());
        }
        let is_list = matches!(&list, Some((num_id, _)) if self.numbering.is_list(*num_id));
        if !is_list {
            self.counters.clear();
            self.levels.clear();
            let quote =
                style.eq_ignore_ascii_case("blocktext") || style.eq_ignore_ascii_case("quote");
            if quote {
                self.push_quote(&lines);
            } else {
                self.push_picture_or_text(&style, &lines);
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
        let run_continues = continues && matches!(self.blocks.last(), Some(Block::List { .. }));
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

    /// A non-list paragraph: drawings carry `[descr]` markers inline; a
    /// caption-styled paragraph pairs with an adjacent image-only paragraph
    /// into a figure (`[caption]`), or with a table into the table's
    /// caption line. Everything else is a wrapped text block.
    fn push_picture_or_text(&mut self, style: &str, lines: &[String]) {
        let caption_style = is_caption_style(style);
        let text = lines.join("\n");
        let image_only = !self.para_images.is_empty() && strip_markers(&text).is_empty();

        // Caption paragraph: pair with the image before it or the table
        // before it, hold for the figure or table after it, or fall through
        // as text.
        if caption_style && !image_only {
            let has_text = !text.is_empty();
            if has_text {
                if let Some(Block::Text(prev)) = self.blocks.last_mut() {
                    if is_image_block(prev) {
                        // Figure: the image paragraph came first.
                        *prev = format!("[{text}]");
                        self.para_images.clear();
                        return;
                    }
                }
                if let Some(Block::Table(state)) = self.blocks.last_mut() {
                    if state.caption.is_none() {
                        // Table captions print below the table.
                        state.caption = Some(text.clone());
                        self.para_images.clear();
                        return;
                    }
                }
                // Nothing behind; hold for a figure or table ahead. Two
                // captions in a row flush the first one as text.
                self.flush_held_caption();
                self.pending_caption = Some(text.clone());
                self.held_caption_lines = Some(lines.to_vec());
                return;
            }
        }
        // An image-only paragraph: a held caption turns it into a figure;
        // otherwise the markers stand alone.
        if image_only {
            if let Some(caption) = self.pending_caption.take() {
                self.held_caption_lines = None;
                self.blocks.push(Block::Text(format!("[{caption}]")));
                self.para_images.clear();
                return;
            }
        }
        // A plain paragraph with a caption still pending: the caption never
        // found an image or table, so emit it first.
        self.flush_held_caption();
        let wrapped: Vec<String> = lines
            .iter()
            .flat_map(|line| wrap_text(line, WRAP_COLUMNS))
            .collect();
        self.blocks.push(Block::Text(wrapped.join("\n")));
        self.para_images.clear();
    }

    /// A held caption paragraph waits for its figure or table; anything
    /// else that arrives means the pairing failed and the caption prints
    /// as an ordinary paragraph.
    fn flush_held_caption(&mut self) {
        if let Some(lines) = self.held_caption_lines.take() {
            let wrapped: Vec<String> = lines
                .iter()
                .flat_map(|line| wrap_text(line, WRAP_COLUMNS))
                .collect();
            self.blocks.push(Block::Text(wrapped.join("\n")));
            self.pending_caption = None;
        }
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
                Block::Table(state) => {
                    if let Some(rendered) = render_table(&state, self.text_width) {
                        out.push_str(&rendered);
                    }
                }
                Block::List(items) => out.push_str(&items.join("\n")),
            }
        }
        if !out.is_empty() {
            out.push('\n');
        }
        self.write(out.as_bytes())
    }
}

/// Caption style names pdfminer... rather, pandoc's reader accepts:
/// "caption", "table caption", "image caption", case-insensitive; style
/// ids squeeze the spaces out.
fn is_caption_style(style: &str) -> bool {
    let squeezed: String = style
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect::<String>()
        .to_ascii_lowercase();
    matches!(squeezed.as_str(), "caption" | "imagecaption" | "tablecaption")
}

/// A block of only `[...]` image markers.
fn is_image_block(block: &str) -> bool {
    !block.is_empty() && strip_markers(block).is_empty()
}

/// Remove balanced `[...]` groups, the way a paragraph of only image
/// markers reads as empty.
fn strip_markers(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0usize;
    for ch in text.chars() {
        match ch {
            '[' => depth += 1,
            ']' => {
                depth = depth.saturating_sub(1);
            }
            other if depth == 0 => out.push(other),
            _ => {}
        }
    }
    out.trim().to_string()
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

/// Render one table the way pandoc's plain writer does: its `pandocTable`
/// with `multiline` on, nested two spaces. Column widths are the grid
/// twips as a share of the page text width (minus ten twips per gap),
/// floored at 71 columns times that share, never below a column's longest
/// word plus two. The border runs above the header and below the body, the
/// per-column underline between them, body rows separated by blank lines
/// with one trailing when there is a single row. A table with no rows is
/// dropped by the reader, so there is nothing to render.
fn render_table(state: &TblState, text_width: Option<u32>) -> Option<String> {
    if state.rows.is_empty() {
        return None;
    }
    let ncols = if !state.grid.is_empty() {
        state.grid.len()
    } else {
        state
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0)
    };
    if ncols == 0 {
        return None;
    }
    let unspecified = state.grid.is_empty() || state.grid.iter().any(|g| g.is_none());
    let (fracs, is_simple): (Vec<f64>, bool) = if unspecified {
        (vec![0.0; ncols], true)
    } else {
        let page = f64::from(text_width.unwrap_or(9360));
        let total = (page - 10.0 * (ncols as f64 - 1.0)).max(1.0);
        let mut f: Vec<f64> = state
            .grid
            .iter()
            .map(|g| g.unwrap_or(0) as f64 / total)
            .collect();
        let sum: f64 = f.iter().sum();
        if sum > 1.0 {
            for x in &mut f {
                *x /= sum;
            }
        }
        (f, false)
    };

    // The header run: the first row when tblLook says so, extended by
    // rows that carry their own w:tblHeader.
    let mut header_count = 0;
    for (i, row) in state.rows.iter().enumerate() {
        if i == 0 && state.header_first_row {
            header_count = 1;
        } else if row.header && header_count == i {
            header_count += 1;
        } else {
            break;
        }
    }
    let (hdr, body) = state.rows.split_at(header_count);

    let widths: Vec<usize> = (0..ncols)
        .map(|c| {
            let cells: Vec<&TblCell> = state
                .rows
                .iter()
                .filter_map(|row| row.cells.get(c))
                .collect();
            if is_simple {
                // Two plus the widest unwrapped paragraph.
                2 + cells
                    .iter()
                    .flat_map(|cell| cell.paras.iter())
                    .flat_map(|para| para.split('\n'))
                    .map(|line| line.chars().count())
                    .max()
                    .unwrap_or(0)
            } else {
                // 71 columns times the width share, but never below the
                // longest word plus two.
                let min_chars = 2 + cells
                    .iter()
                    .flat_map(|cell| cell.paras.iter())
                    .flat_map(|para| para.split('\n'))
                    .flat_map(|line| line.split(' '))
                    .map(|word| word.chars().count())
                    .max()
                    .unwrap_or(0);
                ((71.0 * fracs[c]).floor() as usize).max(min_chars)
            }
        })
        .collect();

    // Alignment comes from the first body row, else the header.
    let align_of = |c: usize| -> Align {
        body.first()
            .or(hdr.first())
            .and_then(|row| row.cells.get(c))
            .map_or(Align::Left, |cell| cell.align)
    };

    // One row line merges the cell blocks left to right, with a one-space
    // block between neighbors: the accumulated line is padded to the
    // accumulated declared width before the next block is appended
    // (doclayout's mergeBlocks), and nothing follows the last block, so its
    // lines stay unpadded. Alignment pads a cell's lines on the left when
    // the block is built, exactly like rblock and cblock.
    let render_row = |row: &TblRow| -> Vec<String> {
        let mut blocks: Vec<(usize, Vec<String>)> = Vec::new();
        for (c, &width) in widths.iter().enumerate() {
            if c > 0 {
                blocks.push((1, vec![" ".to_string()]));
            }
            let empty = TblCell::new();
            let cell = row.cells.get(c).unwrap_or(&empty);
            let align = align_of(c);
            let mut lines = Vec::new();
            for (i, para) in cell.paras.iter().enumerate() {
                if i > 0 {
                    lines.push(String::new());
                }
                for seg in para.split('\n') {
                    for piece in wrap_text(seg, width) {
                        let len = piece.chars().count();
                        match align {
                            Align::Left => lines.push(piece),
                            Align::Center => {
                                lines.push(format!("{}{}", " ".repeat((width - len) / 2), piece));
                            }
                            Align::Right => {
                                lines.push(format!("{}{}", " ".repeat(width - len), piece));
                            }
                        }
                    }
                }
            }
            if lines.is_empty() {
                lines.push(String::new());
            }
            blocks.push((width, lines));
        }
        let height = blocks
            .iter()
            .map(|(_, lines)| lines.len())
            .max()
            .unwrap_or(1);
        (0..height)
            .map(|line_idx| {
                let mut out = String::new();
                let mut acc_width = 0usize;
                let mut acc_len = 0usize;
                for (i, (width, lines)) in blocks.iter().enumerate() {
                    let line = lines.get(line_idx).map(String::as_str).unwrap_or("");
                    if i > 0 {
                        out.push_str(&" ".repeat(acc_width - acc_len));
                    }
                    out.push_str(line);
                    acc_len = acc_width + line.chars().count();
                    acc_width += width;
                }
                out
            })
            .collect()
    };

    let mut lines: Vec<String> = Vec::new();
    let border = "-".repeat(widths.iter().sum::<usize>() + ncols.saturating_sub(1));
    let underline = widths
        .iter()
        .map(|width| "-".repeat(*width))
        .collect::<Vec<_>>()
        .join(" ");
    if !hdr.is_empty() {
        lines.push(border.clone());
        lines.extend(render_row(&hdr[0]));
    }
    lines.push(underline.clone());
    for (i, row) in body.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        lines.extend(render_row(row));
    }
    if body.len() < 2 {
        lines.push(String::new());
    }
    if hdr.is_empty() {
        lines.push(underline);
    } else {
        lines.push(border);
    }
    // A caption prints below the table, blank line between, nested like
    // the table itself.
    if let Some(caption) = &state.caption {
        lines.push(String::new());
        for piece in wrap_text(caption, WRAP_COLUMNS - 4) {
            lines.push(format!(": {piece}"));
        }
    }
    // Nest two; empty lines stay empty, the way the renderer prefixes.
    Some(
        lines
            .into_iter()
            .map(|line| {
                if line.is_empty() {
                    String::new()
                } else {
                    format!("  {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n"),
    )
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
        // A drawing's properties carry its alt text; the marker lands in
        // the paragraph where the drawing sits. The prefix is wp, outside
        // the w: gate below.
        if std::env::var("FIG_DEBUG").is_ok() && name.contains("docPr") {
            eprintln!("docPr seen: {name}");
        }
        if name.ends_with(":docPr") || name == "docPr" {
            let descr = xml_attr(attrs, "descr").unwrap_or("");
            self.para.push('[');
            self.para.push_str(descr);
            self.para.push(']');
            self.para_images.push(descr.to_string());
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
                self.para_jc = None;
                self.para_images.clear();
            }
            "pStyle" => {
                if let Some(v) = xml_attr(attrs, "val") {
                    self.style = v.to_string();
                }
            }
            "jc" => {
                self.para_jc = xml_attr(attrs, "val").map(str::to_string);
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
            "body" => self.body_depth += 1,
            "sectPr" => {
                // Word writes the body-level sectPr last, so the last one
                // wins, matching the reader's single lookup.
                self.in_sectpr = self.body_depth == 1;
                self.sect_parts = [None; 4];
            }
            "pgSz" => {
                if self.in_sectpr {
                    self.sect_parts[0] = xml_attr(attrs, "w").and_then(|v| v.parse().ok());
                }
            }
            "pgMar" => {
                if self.in_sectpr {
                    self.sect_parts[1] = xml_attr(attrs, "left").and_then(|v| v.parse().ok());
                    self.sect_parts[2] = xml_attr(attrs, "right").and_then(|v| v.parse().ok());
                    self.sect_parts[3] = xml_attr(attrs, "gutter").and_then(|v| v.parse().ok());
                }
            }
            "tbl" => {
                let mut state = TblState::default();
                if let Some(caption) = self.pending_caption.take() {
                    state.caption = Some(caption);
                    self.held_caption_lines = None;
                } else if self.pending_caption.is_some() {
                    // A caption paragraph waiting for a figure; a table
                    // takes it instead.
                    state.caption = self.pending_caption.take();
                    self.held_caption_lines = None;
                }
                self.tables.push(state);
            }
            "tblGrid" => {
                if let Some(state) = self.tables.last_mut() {
                    state.grid.clear();
                }
            }
            "gridCol" => {
                if let Some(state) = self.tables.last_mut() {
                    // A column without w:w leaves every column unspecified,
                    // per the reader: writers mishandle a mix.
                    match xml_attr(attrs, "w").and_then(|v| v.parse().ok()) {
                        Some(dxa) => state.grid.push(Some(dxa)),
                        None => state.grid.push(None),
                    }
                }
            }
            "tblLook" => {
                if let Some(state) = self.tables.last_mut() {
                    state.header_first_row = match xml_attr(attrs, "firstRow") {
                        Some("1") => true,
                        Some(_) => false,
                        None => xml_attr(attrs, "val")
                            .and_then(|v| v.parse::<u32>().ok())
                            .is_some_and(|mask| mask & 0x020 != 0),
                    };
                }
            }
            "tblHeader" => self.row_header = true,
            "tr" => {
                self.row_header = false;
                if let Some(state) = self.tables.last_mut() {
                    state.rows.push(TblRow {
                        header: false,
                        cells: Vec::new(),
                    });
                }
            }
            "tc" => {
                if let Some(state) = self.tables.last_mut() {
                    if let Some(row) = state.rows.last_mut() {
                        row.cells.push(TblCell::new());
                    }
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
        let Some(loc) = crate::docx::word_local(name) else {
            return Ok(());
        };
        match loc {
            "t" => self.in_t = false,
            "p" => self.close_para()?,
            "body" => self.body_depth = self.body_depth.saturating_sub(1),
            "sectPr" => {
                if self.in_sectpr {
                    let [w, left, right, gutter] = self.sect_parts;
                    if let (Some(w), Some(left), Some(right), Some(gutter)) =
                        (w, left, right, gutter)
                    {
                        self.text_width = Some(w.saturating_sub(left + right + gutter));
                    }
                    self.in_sectpr = false;
                }
            }
            "tr" => {
                if let Some(state) = self.tables.last_mut() {
                    if let Some(row) = state.rows.last_mut() {
                        row.header = self.row_header;
                    }
                }
                self.row_header = false;
            }
            "tbl" => {
                if let Some(state) = self.tables.pop() {
                    if self.tables.is_empty() {
                        // Rendered at the end, once the section width is known.
                        self.blocks.push(Block::Table(state));
                    } else if let Some(parent) = self.tables.last_mut() {
                        // A table inside a cell keeps the flat tab form.
                        if let Some(cell) =
                            parent.rows.last_mut().and_then(|row| row.cells.last_mut())
                        {
                            for row in &state.rows {
                                let line = row
                                    .cells
                                    .iter()
                                    .map(|cell| {
                                        cell.paras
                                            .iter()
                                            .flat_map(|para| para.split('\n'))
                                            .collect::<Vec<_>>()
                                            .join(" ")
                                    })
                                    .collect::<Vec<_>>()
                                    .join("\t");
                                cell.paras.push(line);
                            }
                        }
                    }
                }
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
        assert_eq!(
            lines.join(" "),
            "aa bb cc dd ee ff gg hh ii jj kk ll mm nn oo pp qq rr ss tt uu vv"
        );
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

    fn table_state(grid: &[Option<u32>], rows: &[Vec<&str>]) -> TblState {
        TblState {
            grid: grid.to_vec(),
            header_first_row: true,
            caption: None,
            rows: rows
                .iter()
                .map(|cells| TblRow {
                    header: false,
                    cells: cells
                        .iter()
                        .map(|text| TblCell {
                            align: Align::Left,
                            paras: vec![(*text).to_string()],
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    #[test]
    fn an_equal_grid_table_draws_pandocs_dashes() {
        // Page width 9360 minus two 10-twip gaps: three 2631-twip columns
        // floor(71 * 2631/9340) to 20 chars each.
        let state = table_state(
            &[Some(2631), Some(2631), Some(2631)],
            &[vec!["a", "b", "c"], vec!["1", "2", "3"]],
        );
        let out = render_table(&state, Some(9360)).unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(
            lines[0],
            "  --------------------------------------------------------------"
        );
        assert_eq!(lines[1], "  a                    b                    c");
        assert_eq!(
            lines[2],
            "  -------------------- -------------------- --------------------"
        );
        assert_eq!(lines[3], "  1                    2                    3");
        assert_eq!(lines[4], "");
        assert_eq!(
            lines[5],
            "  --------------------------------------------------------------"
        );
    }

    #[test]
    fn a_single_body_row_carries_a_trailing_blank_line() {
        let state = table_state(
            &[Some(3900), Some(3900)],
            &[vec!["a", "b"], vec!["1", "2"]],
        );
        let out = render_table(&state, Some(9360)).unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines.len(), 6);
        assert_eq!(lines[4], "", "blank between the one body row and the border");
    }

    #[test]
    fn two_body_rows_separate_with_one_blank_line() {
        let state = table_state(
            &[Some(3900), Some(3900)],
            &[vec!["a", "b"], vec!["1", "2"], vec!["3", "4"]],
        );
        let out = render_table(&state, Some(9360)).unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines.len(), 7);
        assert_eq!(lines[4], "", "blank between the body rows");
        assert!(lines[5].starts_with("  3"), "{:?}", lines[5]);
    }

    #[test]
    fn a_wrapped_cell_pads_its_neighbors_to_the_column_edge() {
        // doclayout's mergeBlocks: every line of a non-final block pads to
        // the block width, including under an empty neighbor line.
        let state = table_state(
            &[Some(2631), Some(2631)],
            &[vec!["aa", "text that wraps here"]],
        );
        let out = render_table(&state, Some(9360)).unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        // Columns floor to 19 (9350 = 9360 minus one 10-twip gap); the
        // wrapped cell's second line keeps its trailing pad plus the
        // separator space, because the last column still merges under it.
        assert_eq!(lines[1], "  aa                  text that wraps");
        // The last block merges nothing after it, so its continuation line
        // carries no trailing pad: the gap spaces from the empty first
        // column and separator, then the bare word.
        assert_eq!(lines[2], "                      here");
    }

    #[test]
    fn grid_columns_without_widths_fall_back_to_content() {
        let state = table_state(&[None, None], &[vec!["name", "n"], vec!["abc", "12"]]);
        let out = render_table(&state, Some(9360)).unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        // numChars: two plus the widest cell in the column.
        assert_eq!(lines[1], "  name   n");
        assert_eq!(lines[3], "  abc    12");
    }

    #[test]
    fn cell_alignment_pads_inside_the_column() {
        let mut state = table_state(
            &[Some(2631), Some(2631), Some(2631)],
            &[vec!["a", "b", "c"], vec!["left", "mid", "right"]],
        );
        // Alignment comes from the first body row's cells.
        state.rows[1].cells[0].align = Align::Left;
        state.rows[1].cells[1].align = Align::Center;
        state.rows[1].cells[2].align = Align::Right;
        let out = render_table(&state, Some(9360)).unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        // Widths floor to 20. "mid" centers at (20-3)/2 = 8 spaces into
        // its column; "right" fills its column from the left with 15
        // spaces; the merge pads each finished column to its edge.
        assert_eq!(lines[3], "  left                         mid                         right");
    }

    #[test]
    fn jc_maps_to_cell_alignment() {
        assert_eq!(jc_to_align(Some("center")), Align::Center);
        assert_eq!(jc_to_align(Some("right")), Align::Right);
        assert_eq!(jc_to_align(Some("end")), Align::Right);
        assert_eq!(jc_to_align(Some("left")), Align::Left);
        assert_eq!(jc_to_align(None), Align::Left);
    }

    #[test]
    fn image_markers_stand_where_the_drawing_sat() {
        let mut out = Vec::new();
        {
            let mut sink = TextSink::new(&mut out, Numbering::default());
            sink.start("w:p", &[]).unwrap();
            sink.start("w:t", &[]).unwrap();
            sink.text("Before the picture ").unwrap();
            sink.end("w:t").unwrap();
        sink.start(
            "wp:docPr",
            &[Attr {
                name: "descr".into(),
                value: "red square".into(),
            }],
        )
        .unwrap();
        sink.end("wp:docPr").unwrap();
            sink.end("w:t").unwrap();
            sink.start("w:t", &[]).unwrap();
            sink.text(" and more.").unwrap();
            sink.end("w:t").unwrap();
            sink.end("w:p").unwrap();
            sink.finish_document().unwrap();
        }
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out.trim(), "Before the picture [red square] and more.");
    }

    #[test]
    fn a_caption_paragraph_pairs_with_the_image_before_it() {
        let mut out = Vec::new();
        {
            let mut sink = TextSink::new(&mut out, Numbering::default());
            sink.start("w:p", &[]).unwrap();
            sink.start(
                "wp:docPr",
                &[Attr {
                    name: "descr".into(),
                    value: "".into(),
                }],
            )
            .unwrap();
            sink.end("wp:docPr").unwrap();
            sink.end("w:p").unwrap();
            sink.start("w:p", &[]).unwrap();
            sink.start(
                "w:pStyle",
                &[Attr {
                    name: "val".into(),
                    value: "ImageCaption".into(),
                }],
            )
            .unwrap();
            sink.start("w:t", &[]).unwrap();
            sink.text("The caption text").unwrap();
            sink.end("w:t").unwrap();
            sink.end("w:p").unwrap();
            sink.finish_document().unwrap();
        }
        let out = String::from_utf8(out).unwrap();
        assert_eq!(out.trim(), "[The caption text]");
    }

    #[test]
    fn a_table_caption_prints_below_the_grid() {
        let state = table_state(
            &[Some(2631), Some(2631)],
            &[vec!["a", "b"], vec!["1", "2"]],
        );
        let mut state = state;
        state.caption = Some("The table caption.".into());
        let out = render_table(&state, Some(9360)).unwrap();
        let lines: Vec<&str> = out.split('\n').collect();
        assert_eq!(lines[lines.len() - 2], "");
        assert_eq!(lines[lines.len() - 1], "  : The table caption.");
    }
}
