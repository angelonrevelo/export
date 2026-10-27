//! Layout-aware PDF text: characters with positions from the content
//! streams, shaped the way pdfplumber's `extract_text` shapes them.
//!
//! The literal-strings path in [`crate::pdf`] reads `Tj` payloads without
//! positions or font knowledge, so subset-font files come out as glyph
//! indices. This module runs the text state machine instead: text and
//! line matrices, font widths, character and word spacing, and the
//! horizontal scale all place every glyph; ToUnicode CMaps and the base
//! encodings turn codes into unicode. The shaping then follows pdfplumber:
//! tops cluster into lines (three points, chained on distinct values),
//! characters sort by x, gaps over three points and top shifts start new
//! words, ligatures expand, words join with spaces and lines with newlines.
//!
//! What is not here: vertical writing, non-identity CID encodings other
//! than the two-byte Identity cmaps, base-fourteen font metrics when a
//! file omits widths, and pdfminer's layout tree ordering (chars are
//! taken in content-stream order; the shaping is order-independent for
//! upright text, which is what documents are made of).

use std::collections::HashMap;

use crate::error::{Error, Result};
use crate::glyphlist::glyphname2unicode;
use crate::pdf::{Obj, Src};
use Tok::{Name, Num};

const X_TOLERANCE: f64 = 3.0;
const Y_TOLERANCE: f64 = 3.0;

/// One page's text, shaped like pdfplumber's `extract_text`.
pub fn layout_text_pages(data: &[u8]) -> Result<Vec<String>> {
    let mut src = Src::open(data)?;
    let ids = src.page_ids()?;
    let mut pages = Vec::new();
    for id in ids {
        pages.push(layout_page(&mut src, id)?);
    }
    Ok(pages)
}

/// One character, in pdfplumber's page coordinates.
struct PChar {
    text: String,
    x0: f64,
    x1: f64,
    top: f64,
    upright: bool,
}

fn layout_page(src: &mut Src, id: u32) -> Result<String> {
    let page = src.load(id)?;
    let mb = inherited_box(src, id, &page, "MediaBox");
    let mb = match mb {
        Some(v) => rect_of(&v).ok_or_else(|| Error::new("pdf: MediaBox is not a rectangle"))?,
        None => [0.0, 0.0, 612.0, 792.0],
    };
    let resources = inherited_box(src, id, &page, "Resources");
    let rotate = page
        .dict_get("Rotate")
        .and_then(obj_int)
        .unwrap_or(0)
        .rem_euclid(360);

    // The page's transform, per pdfminer's process_page for each rotation.
    let ctm = match rotate {
        90 => [0.0, -1.0, 1.0, 0.0, -mb[1], mb[2]],
        180 => [-1.0, 0.0, 0.0, -1.0, mb[2], mb[3]],
        270 => [0.0, 1.0, -1.0, 0.0, mb[3], -mb[0]],
        _ => [1.0, 0.0, 0.0, 1.0, -mb[0], -mb[1]],
    };

    let contents = match page.dict_get("Contents") {
        Some(v) => src.stream_bytes(v)?,
        None => Vec::new(),
    };
    let mut fonts = Fonts::new();
    if let Some(res) = &resources {
        if let Some(fonts_obj) = res.dict_get("Font") {
            fonts.set_root(fonts_obj.clone());
        }
    }
    let raw = Interpreter {
        src,
        fonts: &mut fonts,
        resources: resources.clone(),
        chars: Vec::new(),
        ctm,
        ts: TextState::default(),
        stack: Vec::new(),
        saved: Vec::new(),
    }
    .run(&contents)?;

    // pdfplumber's conversion: tops flip around the mediabox, and the
    // mediabox's x offset slides back in.
    let height = mb[3] - mb[1];
    let mb_x0 = mb[0];
    let mb_top = mb[1];
    let out: Vec<PChar> = raw
        .into_iter()
        .map(|c| PChar {
            x0: c.x0 + mb_x0,
            x1: c.x1 + mb_x0,
            top: height - c.y1 + mb_top,
            text: c.text,
            upright: c.upright,
        })
        .collect();
    Ok(shape(&out))
}

/// MediaBox and Resources can live on parent nodes; `Src::inherit` walks
/// the Parent chain the same way the writer path does.
fn inherited_box(src: &mut Src, id: u32, page: &Obj, key: &str) -> Option<Obj> {
    if let Some(v) = page.dict_get(key) {
        return Some(v.clone());
    }
    src.inherit(id, key).ok().flatten()
}

fn rect_of(obj: &Obj) -> Option<[f64; 4]> {
    let Obj::Arr(items) = obj else {
        return None;
    };
    if items.len() != 4 {
        return None;
    }
    let mut out = [0.0f64; 4];
    for (i, item) in items.iter().enumerate() {
        out[i] = match item {
            Obj::Int(n) => *n as f64,
            Obj::Real(s) => s.parse().ok()?,
            Obj::Ref(id) => {
                // Callers resolve references before this point in most
                // files; a bare reference here cannot be followed without
                // the source, so the box is unusable.
                let _ = id;
                return None;
            }
            _ => return None,
        };
    }
    Some(out)
}

fn obj_int(obj: &Obj) -> Option<i64> {
    match obj {
        Obj::Int(n) => Some(*n),
        Obj::Real(s) => s.parse().ok(),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Text state and fonts
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct TextState {
    /// The text matrix, Tm.
    matrix: [f64; 6],
    /// The line matrix's translation, Tlm's e/f.
    line_x: f64,
    line_y: f64,
    font: Option<std::rc::Rc<PdfFont>>,
    fontsize: f64,
    charspace: f64,
    wordspace: f64,
    /// Tz, in percent.
    scaling: f64,
    /// Negated, as pdfminer stores it.
    leading: f64,
    rise: f64,
}

impl Default for TextState {
    fn default() -> Self {
        Self {
            matrix: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
            line_x: 0.0,
            line_y: 0.0,
            font: None,
            fontsize: 0.0,
            charspace: 0.0,
            wordspace: 0.0,
            scaling: 100.0,
            leading: 0.0,
            rise: 0.0,
        }
    }
}

/// One font, resolved lazily when `Tf` names it.
struct PdfFont {
    /// Two-byte codes (a Type0 font with an Identity-style encoding).
    two_byte: bool,
    /// Widths in thousandths of text space, keyed by character id.
    widths: HashMap<u32, f64>,
    default_width: f64,
    /// In thousandths, forced negative like pdfminer does.
    descent: f64,
    /// From the ToUnicode CMap, when present.
    to_uni: Option<HashMap<u32, String>>,
    /// Simple fonts: the base encoding plus Differences.
    simple_encoding: Option<HashMap<u32, String>>,
    /// A built-in font's widths, keyed by glyph name and looked up through
    /// the character's unicode, the way pdfminer's tables are keyed.
    name_widths: Option<HashMap<String, f64>>,
}

impl PdfFont {
    fn decode(&self, bytes: &[u8]) -> Vec<u32> {
        if self.two_byte {
            bytes
                .chunks_exact(2)
                .map(|pair| (u32::from(pair[0]) << 8) | u32::from(pair[1]))
                .collect()
        } else {
            bytes.iter().map(|b| u32::from(*b)).collect()
        }
    }

    fn to_unichr(&self, cid: u32) -> Option<String> {
        if let Some(map) = &self.to_uni {
            if let Some(text) = map.get(&cid) {
                return Some(text.clone());
            }
        }
        if let Some(enc) = &self.simple_encoding {
            if let Some(text) = enc.get(&cid) {
                return Some(text.clone());
            }
        }
        None
    }

    /// Width in thousandths of text space: the width table by id, else the
    /// default. pdfminer also retries with the character's unicode when the
    /// table is keyed by names; the tables this builds are numeric only.
    fn char_width(&self, cid: u32) -> f64 {
        if let Some(w) = self.widths.get(&cid) {
            return *w;
        }
        if let Some(table) = &self.name_widths {
            if let Some(text) = self.to_unichr(cid) {
                if let Some(w) = table.get(&text) {
                    return *w;
                }
            }
        }
        self.default_width
    }
}

/// The fonts of one page, resolved from `Resources /Font` on demand.
struct Fonts {
    root: Option<Obj>,
    cache: HashMap<String, std::rc::Rc<PdfFont>>,
    empty_font: std::rc::Rc<PdfFont>,
}

impl Fonts {
    fn new() -> Self {
        Self {
            root: None,
            cache: HashMap::new(),
            empty_font: std::rc::Rc::new(PdfFont {
                two_byte: false,
                widths: HashMap::new(),
                default_width: 0.0,
                descent: 0.0,
                to_uni: None,
                simple_encoding: None,
                name_widths: None,
            }),
        }
    }

    fn set_root(&mut self, root: Obj) {
        self.root = Some(root);
    }
}

/// Resolve (and memoize) the font a `Tf` names.
fn get_font(fonts: &mut Fonts, name: &str, src: &mut Src) -> std::rc::Rc<PdfFont> {
    if let Some(font) = fonts.cache.get(name) {
        return font.clone();
    }
    let font = fonts
        .root
        .as_ref()
        .and_then(|root| root.dict_get(name))
        .and_then(|spec| resolve_font(spec, src));
    let font = font.unwrap_or_else(|| fonts.empty_font.clone());
    fonts.cache.insert(name.to_string(), font.clone());
    font
}

fn resolve_font(spec: &Obj, src: &mut Src) -> Option<std::rc::Rc<PdfFont>> {
    let spec = deref(spec, src, 0)?;
    let subtype = spec.name_val("Subtype").unwrap_or("").to_string();
    let to_uni = spec
        .dict_get("ToUnicode")
        .and_then(|stream| {
            src.stream_bytes(stream)
                .ok()
                .map(|data| parse_to_unicode(&data))
        })
        .filter(|map| !map.is_empty());

    let font = match subtype.as_str() {
        "Type0" => {
            let mut font = PdfFont {
                two_byte: true,
                widths: HashMap::new(),
                default_width: 1000.0,
                descent: 0.0,
                to_uni,
                simple_encoding: None,
                name_widths: None,
            };
            // The descendant carries widths and the descriptor.
            if let Some(Obj::Arr(descendants)) = spec.dict_get("DescendantFonts") {
                if let Some(desc) = descendants.first().and_then(|d| deref(d, src, 0)) {
                    parse_widths_into(&mut font, desc.dict_get("W"), desc.dict_get("DW"));
                    if let Some(descriptor) = desc.dict_get("FontDescriptor") {
                        font.descent = num_field(descriptor, "Descent");
                    }
                }
            }
            std::rc::Rc::new(font)
        }
        "Type3" => {
            // Type3 glyph procedures are not painted; the matrix and the
            // FontBBox descent still place the characters.
            let mut font = PdfFont {
                two_byte: false,
                widths: HashMap::new(),
                default_width: 0.0,
                descent: 0.0,
                to_uni,
                simple_encoding: None,
                name_widths: None,
            };
            parse_widths_into(&mut font, spec.dict_get("Widths"), None);
            let first_char = spec.dict_get("FirstChar").and_then(obj_int).unwrap_or(0);
            shift_widths(&mut font.widths, first_char);
            let scale = spec
                .dict_get("FontMatrix")
                .and_then(|m| match m {
                    Obj::Arr(items) => items.first().and_then(obj_int),
                    _ => None,
                })
                .unwrap_or(1);
            font.descent = bbox_descent(spec.dict_get("FontBBox"), scale);
            font.default_width /= scale.max(1) as f64;
            finish_font(font)
        }
        // Type1, TrueType, and the rest of the simple fonts.
        _ => {
            let default_encoding = if subtype == "TrueType" {
                "WinAnsiEncoding"
            } else {
                "StandardEncoding"
            };
            let simple_encoding = match spec.dict_get("Encoding") {
                Some(Obj::Name(name)) => Some(base_table(name)),
                Some(Obj::Dict(d)) => {
                    // A dictionary: BaseEncoding plus Differences.
                    let base = d
                        .iter()
                        .find(|(k, _)| k == "BaseEncoding")
                        .and_then(|(_, v)| match v {
                            Obj::Name(n) => Some(n.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| default_encoding.to_string());
                    let mut table = base_table(&base);
                    if let Some(Obj::Arr(diff)) = d
                        .iter()
                        .find(|(k, _)| k == "Differences")
                        .map(|(_, v)| v)
                    {
                        apply_differences(&mut table, diff);
                    }
                    Some(table)
                }
                _ => Some(base_table(default_encoding)),
            };
            // The built-in metrics win when the BaseFont names one, exactly
            // like pdfminer's FontMetricsDB lookup ahead of the spec.
            let basefont = spec.name_val("BaseFont").unwrap_or("");
            if let Some(builtin) = crate::base14::base14().get(basefont) {
                let font = PdfFont {
                    two_byte: false,
                    widths: HashMap::new(),
                    default_width: builtin.default_width,
                    descent: builtin.descent,
                    to_uni,
                    simple_encoding,
                    name_widths: Some(builtin.widths.clone()),
                };
                return Some(std::rc::Rc::new(font));
            }
            let mut font = PdfFont {
                two_byte: false,
                widths: HashMap::new(),
                default_width: 0.0,
                descent: 0.0,
                to_uni,
                simple_encoding,
                name_widths: None,
            };
            if let Some(descriptor) = spec.dict_get("FontDescriptor").and_then(|d| deref(d, src, 0)) {
                font.descent = num_field(&descriptor, "Descent");
                font.default_width = num_field(&descriptor, "MissingWidth");
            }
            parse_widths_into(&mut font, spec.dict_get("Widths"), None);
            let first_char = spec.dict_get("FirstChar").and_then(obj_int).unwrap_or(0);
            shift_widths(&mut font.widths, first_char);
            finish_font(font)
        }
    };
    Some(font)
}

/// Wrap a font, forcing its descent negative like pdfminer does: a
/// positive one would put the glyphs above the baseline.
fn finish_font(font: PdfFont) -> std::rc::Rc<PdfFont> {
    let mut font = font;
    if font.descent > 0.0 {
        font.descent = -font.descent;
    }
    std::rc::Rc::new(font)
}

fn parse_widths_into(font: &mut PdfFont, widths: Option<&Obj>, default: Option<&Obj>) {
    if let Some(d) = default.and_then(obj_int) {
        font.default_width = d as f64;
    }
    let Some(Obj::Arr(items)) = widths else {
        return;
    };
    let mut runs: Vec<f64> = Vec::new();
    for item in items {
        match item {
            Obj::Int(n) => runs.push(*n as f64),
            Obj::Real(s) => {
                if let Ok(v) = s.parse::<f64>() {
                    runs.push(v);
                }
            }
            Obj::Arr(list) => {
                if let Some(char1) = runs.pop() {
                    for (i, w) in list.iter().enumerate() {
                        let w = match w {
                            Obj::Int(n) => *n as f64,
                            Obj::Real(s) => s.parse().unwrap_or(0.0),
                            _ => continue,
                        };
                        font.widths.insert(char1 as u32 + i as u32, w);
                    }
                }
            }
            _ => {}
        }
        if runs.len() == 3 {
            let (char1, char2, w) = (runs[0], runs[1], runs[2]);
            if char1.fract() == 0.0 && char2.fract() == 0.0 {
                for cid in char1 as u32..=char2 as u32 {
                    font.widths.insert(cid, w);
                }
            }
            runs.clear();
        }
    }
}

fn shift_widths(widths: &mut HashMap<u32, f64>, first_char: i64) {
    if first_char == 0 {
        return;
    }
    let shifted: Vec<(u32, f64)> = widths
        .iter()
        .map(|(k, v)| ((*k as i64 + first_char) as u32, *v))
        .collect();
    widths.clear();
    widths.extend(shifted);
}

fn num_field(dict: &Obj, key: &str) -> f64 {
    dict.dict_get(key)
        .and_then(obj_int)
        .unwrap_or(0) as f64
}

fn bbox_descent(bbox: Option<&Obj>, scale: i64) -> f64 {
    // A Type3's descent comes from the FontBBox's lower edge.
    if let Some(Obj::Arr(items)) = bbox {
        if items.len() == 4 {
            if let Some(y0) = obj_int(&items[1]) {
                return y0 as f64 / scale.max(1) as f64;
            }
        }
    }
    0.0
}

fn deref(obj: &Obj, src: &mut Src, depth: u32) -> Option<Obj> {
    if depth > 8 {
        return None;
    }
    match obj {
        Obj::Ref(id) => {
            let loaded = src.load(*id).ok()?;
            deref(&loaded, src, depth + 1)
        }
        other => Some(other.clone()),
    }
}

// ---------------------------------------------------------------------------
// Encodings
// ---------------------------------------------------------------------------

/// WinAnsiEncoding code to unicode.
pub(crate) const WIN_ANSI: [Option<u32>; 256] = include!("pdftext_winansi.in");
/// StandardEncoding code to unicode.
pub(crate) const STANDARD: [Option<u32>; 256] = include!("pdftext_standard.in");
/// MacRomanEncoding code to unicode.
pub(crate) const MAC_ROMAN: [Option<u32>; 256] = include!("pdftext_macroman.in");

// The tables are generated from pdfminer's EncodingDB so the code points
// match the reference exactly, including its quirk of mapping both 0x20
// and a few control codes to plain spaces.

fn base_table(name: &str) -> HashMap<u32, String> {
    let table: &[Option<u32>; 256] = match name {
        "WinAnsiEncoding" => &WIN_ANSI,
        "MacRomanEncoding" => &MAC_ROMAN,
        // The reference falls back to StandardEncoding for unknown names.
        _ => &STANDARD,
    };
    table
        .iter()
        .enumerate()
        .filter_map(|(code, u)| u.map(|cp| (code as u32, unicode_from_cp(cp))))
        .collect()
}

fn unicode_from_cp(cp: u32) -> String {
    char::from_u32(cp).map(String::from).unwrap_or_default()
}

/// `Differences`: numbers set the code, names map to unicode through the
/// glyph list (or the `uniXXXX`/`uXXXX` forms).
fn apply_differences(table: &mut HashMap<u32, String>, diff: &[Obj]) {
    let agl = glyphname2unicode();
    let mut cid = 0u32;
    for item in diff {
        match item {
            Obj::Int(n) => cid = *n as u32,
            Obj::Name(name) => {
                if let Some(text) = name_to_unicode(name, agl) {
                    table.insert(cid, text);
                }
                cid += 1;
            }
            _ => {}
        }
    }
}

/// Adobe's glyph-name rules: components split on underscores, `uniXXXX`
/// groups of four hex digits, `uXXXXXX` single codepoints, else the glyph
/// list.
fn name_to_unicode(name: &str, agl: &HashMap<String, String>) -> Option<String> {
    let base = name.split('.').next().unwrap_or(name);
    if base.contains('_') {
        let parts: Option<Vec<String>> = base.split('_').map(|p| name_to_unicode(p, agl)).collect();
        return parts.map(|parts| parts.concat());
    }
    if let Some(text) = agl.get(base) {
        return Some(text.clone());
    }
    if let Some(hex) = base.strip_prefix("uni") {
        if !hex.is_empty() && hex.len() % 4 == 0 && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let mut out = String::new();
            for i in (0..hex.len()).step_by(4) {
                let cp = u32::from_str_radix(&hex[i..i + 4], 16).ok()?;
                out.push(char::from_u32(cp)?);
            }
            return Some(out);
        }
    }
    if let Some(hex) = base.strip_prefix('u') {
        if (4..=6).contains(&hex.len()) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
            let cp = u32::from_str_radix(hex, 16).ok()?;
            return char::from_u32(cp).map(String::from);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// ToUnicode CMaps
// ---------------------------------------------------------------------------

/// `beginbfchar` pairs and `beginbfrange` runs, into cid to unicode.
fn parse_to_unicode(data: &[u8]) -> HashMap<u32, String> {
    let mut map = HashMap::new();
    let mut section = 0u8; // 0 none, 1 bfchar, 2 bfrange
    let mut pending: Vec<Tok> = Vec::new();
    let mut i = 0;
    while let Some(tok) = next_token(data, &mut i) {
        match tok {
            Tok::Op(op) => match op.as_str() {
                "beginbfchar" => {
                    section = 1;
                    pending.clear();
                }
                "endbfchar" => {
                    for pair in pending.chunks(2) {
                        if let [Tok::Hex(cid), Tok::Hex(code)] = pair {
                            insert_unicode(&mut map, nunpack(cid), code);
                        }
                    }
                    section = 0;
                }
                "beginbfrange" => {
                    section = 2;
                    pending.clear();
                }
                "endbfrange" => {
                    for triple in pending.chunks(3) {
                        if let [Tok::Hex(start), Tok::Hex(end), to] = triple {
                            let start = nunpack(start);
                            let end = nunpack(end);
                            if end < start {
                                continue;
                            }
                            match to {
                                Tok::Hex(code) => {
                                    // A base: the low bytes count up, kept
                                    // to the base's byte length.
                                    let vlen = code.len().min(4);
                                    let base = nunpack(&code[code.len().saturating_sub(vlen)..]);
                                    let prefix = &code[..code.len() - vlen];
                                    for i in 0..=(end - start) {
                                        let value = base + i;
                                        let bytes = value.to_be_bytes();
                                        let mut text: Vec<u8> = prefix.to_vec();
                                        text.extend_from_slice(&bytes[4 - vlen..]);
                                        insert_unicode(&mut map, start + i, &text);
                                    }
                                }
                                Tok::Arr(items) => {
                                    for (i, item) in items.iter().enumerate() {
                                        if let Tok::Hex(code) = item {
                                            insert_unicode(&mut map, start + i as u32, code);
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                    section = 0;
                }
                "beginnotdefrange" | "endnotdefrange" | "begincodespacerange"
                | "endcodespacerange" | "beginbfchar_section" => {
                    section = if op == "beginnotdefrange" { 9 } else { 0 };
                    pending.clear();
                }
                _ => {}
            },
            Tok::Hex(_) => {
                if section == 1 || section == 2 {
                    pending.push(tok);
                    if pending.len() > 4096 {
                        pending.clear();
                    }
                }
            }
            Tok::Arr(items) if section == 2 => {
                pending.push(Tok::Arr(items));
            }
            _ => {}
        }
    }
    map
}

/// CMap strings decode UTF-16BE, or a single byte as a codepoint; a
/// non-breaking space collapses onto an existing plain space for the same
/// cid, like pdfminer's guard.
fn insert_unicode(map: &mut HashMap<u32, String>, cid: u32, code: &[u8]) {
    let text = if code.len() % 2 == 0 && code.len() >= 2 {
        let units: Vec<u16> = code
            .chunks_exact(2)
            .map(|pair| (u16::from(pair[0]) << 8) | u16::from(pair[1]))
            .collect();
        String::from_utf16(&units).unwrap_or_default()
    } else if code.len() == 1 {
        unicode_from_cp(u32::from(code[0]))
    } else {
        return;
    };
    if text == "\u{00a0}" && map.get(&cid).map(String::as_str) == Some(" ") {
        return;
    }
    if !text.is_empty() {
        map.insert(cid, text);
    }
}

/// The decoded CMap byte strings are big-endian numbers (pdfminer's
/// `nunpack`).
fn nunpack(bytes: &[u8]) -> u32 {
    let mut value = 0u32;
    for b in bytes {
        value = (value << 8) | u32::from(*b);
    }
    value
}

// ---------------------------------------------------------------------------
// The content stream interpreter
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
enum Tok {
    Num(f64),
    Str(Vec<u8>),
    Hex(Vec<u8>),
    Name(String),
    Op(String),
    Arr(Vec<Tok>),
}

/// One content token at `i`, advancing it. Dicts are skipped: content
/// streams only carry them inside inline images, which do not paint text.
fn next_token(data: &[u8], i: &mut usize) -> Option<Tok> {
    loop {
        while *i < data.len() && matches!(data[*i], b' ' | b'\t' | b'\n' | b'\r' | 0 | 12) {
            *i += 1;
        }
        let b = *data.get(*i)?;
        *i += 1;
        return match b {
            b'(' => Some(Tok::Str(parse_literal(data, i)?)),
            b'<' if data.get(*i) != Some(&b'<') => Some(Tok::Hex(parse_hex(data, i)?)),
            b'<' => {
                skip_dict(data, i)?;
                continue;
            }
            b'/' => Some(Tok::Name(parse_name(data, i))),
            b'[' => {
                let mut items = Vec::new();
                loop {
                    match next_token(data, i) {
                        Some(Tok::Op(op)) if op == "]" => break,
                        Some(tok) => items.push(tok),
                        None => break,
                    }
                }
                Some(Tok::Arr(items))
            }
            b']' => Some(Tok::Op("]".to_string())),
            b'{' | b'}' => continue,
            b'-' | b'+' | b'.' | b'0'..=b'9' => {
                let start = *i - 1;
                while *i < data.len()
                    && matches!(data[*i], b'0'..=b'9' | b'.' | b'-' | b'+' | b'e' | b'E')
                {
                    *i += 1;
                }
                let text = std::str::from_utf8(&data[start..*i]).ok()?;
                Some(Tok::Num(text.parse().unwrap_or(0.0)))
            }
            b'"' => Some(Tok::Op("\"".to_string())),
            b'\'' => Some(Tok::Op("'".to_string())),
            b'0'..=b'z' => {
                let start = *i - 1;
                while *i < data.len() && is_word(data[*i]) {
                    *i += 1;
                }
                Some(Tok::Op(
                    String::from_utf8_lossy(&data[start..*i]).into_owned(),
                ))
            }
            _ => continue,
        };
    }
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'*' || b == b'#' || b == b'!' || b == b'?'
}

fn parse_literal(data: &[u8], i: &mut usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut depth = 1i32;
    while *i < data.len() && depth > 0 {
        let b = data[*i];
        *i += 1;
        match b {
            b'\\' => {
                let e = *data.get(*i)?;
                *i += 1;
                match e {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'\n' => {}
                    b'(' | b')' | b'\\' => out.push(e),
                    b'0'..=b'7' => {
                        let mut v = u32::from(e - b'0');
                        for _ in 0..2 {
                            if let Some(d) = data.get(*i).copied() {
                                if (b'0'..=b'7').contains(&d) {
                                    v = v * 8 + u32::from(d - b'0');
                                    *i += 1;
                                }
                            }
                        }
                        out.push(v as u8);
                    }
                    b'\r' => {
                        if data.get(*i) == Some(&b'\n') {
                            *i += 1;
                        }
                    }
                    other => out.push(other),
                }
            }
            b'(' => {
                depth += 1;
                out.push(b);
            }
            b')' => {
                depth -= 1;
                if depth > 0 {
                    out.push(b);
                }
            }
            _ => out.push(b),
        }
    }
    if depth != 0 {
        return None;
    }
    Some(out)
}

fn parse_hex(data: &[u8], i: &mut usize) -> Option<Vec<u8>> {
    let mut nibbles = Vec::new();
    while *i < data.len() {
        let b = data[*i];
        *i += 1;
        match b {
            b'>' => break,
            b'0'..=b'9' | b'a'..=b'f' | b'A'..=b'F' => nibbles.push(b),
            _ => continue,
        }
    }
    if nibbles.len() % 2 == 1 {
        nibbles.push(b'0');
    }
    Some(
        nibbles
            .chunks_exact(2)
            .map(|pair| {
                let hi = (pair[0] as char).to_digit(16).unwrap_or(0) as u8;
                let lo = (pair[1] as char).to_digit(16).unwrap_or(0) as u8;
                (hi << 4) | lo
            })
            .collect(),
    )
}

fn parse_name(data: &[u8], i: &mut usize) -> String {
    let mut out = String::new();
    while *i < data.len() && is_word(data[*i]) {
        if data[*i] == b'#' && *i + 2 < data.len() {
            let hi = (data[*i + 1] as char).to_digit(16);
            let lo = (data[*i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push(((hi << 4) | lo) as u8 as char);
                *i += 3;
                continue;
            }
        }
        out.push(data[*i] as char);
        *i += 1;
    }
    out
}

fn skip_dict(data: &[u8], i: &mut usize) -> Option<()> {
    let mut depth = 1usize;
    while *i < data.len() && depth > 0 {
        match data[*i] {
            b'<' if data.get(*i + 1) == Some(&b'<') => {
                depth += 1;
                *i += 1;
            }
            b'>' if data.get(*i + 1) == Some(&b'>') => {
                depth -= 1;
                *i += 1;
            }
            _ => {}
        }
        *i += 1;
    }
    Some(())
}

struct RawChar {
    text: String,
    x0: f64,
    x1: f64,
    y1: f64,
    upright: bool,
}

struct Interpreter<'a, 'b> {
    src: &'b mut Src<'a>,
    fonts: &'b mut Fonts,
    /// The current `/Resources`, for fonts and form XObjects; a form's own
    /// Resources replace it while it runs, like pdfminer's push_state.
    resources: Option<Obj>,
    chars: Vec<RawChar>,
    ctm: [f64; 6],
    ts: TextState,
    stack: Vec<Tok>,
    /// `q` saves the CTM and the whole text state, like the interpreter's
    /// `(ctm, textstate, graphicstate)` push.
    saved: Vec<([f64; 6], TextState)>,
}

impl Interpreter<'_, '_> {
    fn run(&mut self, data: &[u8]) -> Result<Vec<RawChar>> {
        let mut i = 0;
        while let Some(tok) = next_token(data, &mut i) {
            match tok {
                Tok::Op(op) => self.execute(&op)?,
                other => self.stack.push(other),
            }
        }
        Ok(std::mem::take(&mut self.chars))
    }

    fn execute(&mut self, op: &str) -> Result<()> {
        match op {
            "q" => {
                self.saved.push((self.ctm, self.ts.clone()));
                self.stack.clear();
                Ok(())
            }
            "Q" => {
                if let Some((ctm, ts)) = self.saved.pop() {
                    self.ctm = ctm;
                    self.ts = ts;
                }
                self.stack.clear();
                Ok(())
            }
            "cm" => {
                if let Some(m) = self.matrix_args() {
                    self.ctm = mult(&m, &self.ctm);
                }
                self.stack.clear();
                Ok(())
            }
            "BT" => {
                self.ts.matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                self.ts.line_x = 0.0;
                self.ts.line_y = 0.0;
                self.stack.clear();
                Ok(())
            }
            "ET" | "BX" | "EX" => {
                self.stack.clear();
                Ok(())
            }
            "Tf" => {
                // Operands stack name first, size second.
                if let [Name(name), Num(size)] = &self.stack[..] {
                    self.ts.fontsize = *size;
                    let font = get_font(self.fonts, name, self.src);
                    self.ts.font = Some(font);
                }
                self.stack.clear();
                Ok(())
            }
            "Td" => {
                if let [Num(tx), Num(ty)] = &self.stack[..] {
                    shift_line(&mut self.ts, *tx, *ty);
                    self.ts.leading = 0.0;
                }
                self.stack.clear();
                Ok(())
            }
            "TD" => {
                if let [Num(tx), Num(ty)] = &self.stack[..] {
                    // do_TD stores the operand as-is; only TL negates.
                    self.ts.leading = *ty;
                    shift_line(&mut self.ts, *tx, *ty);
                }
                self.stack.clear();
                Ok(())
            }
            "Tm" => {
                if let [Num(a), Num(b), Num(c), Num(d), Num(e), Num(f)] = &self.stack[..] {
                    self.ts.matrix = [*a, *b, *c, *d, *e, *f];
                    self.ts.line_x = 0.0;
                    self.ts.line_y = 0.0;
                }
                self.stack.clear();
                Ok(())
            }
            "T*" => {
                let leading = self.ts.leading;
                shift_line(&mut self.ts, 0.0, leading);
                self.show_text();
                self.stack.clear();
                Ok(())
            }
            "Tj" => {
                self.show_text();
                self.stack.clear();
                Ok(())
            }
            "TJ" => {
                if let Some(Tok::Arr(items)) = self.stack.last() {
                    let seq: Vec<Tok> = items.to_vec();
                    self.show_seq(&seq);
                }
                self.stack.clear();
                Ok(())
            }
            "'" => {
                let leading = self.ts.leading;
                shift_line(&mut self.ts, 0.0, leading);
                self.show_text();
                self.stack.clear();
                Ok(())
            }
            "\"" => {
                // aw ac string: word space, char space, show.
                let seq = self.stack.last().cloned();
                let aw = match self.stack.get(self.stack.len().saturating_sub(3)) {
                    Some(Tok::Num(n)) => Some(*n),
                    _ => None,
                };
                let ac = match self.stack.get(self.stack.len().saturating_sub(2)) {
                    Some(Tok::Num(n)) => Some(*n),
                    _ => None,
                };
                if let (Some(aw), Some(ac), Some(seq)) = (aw, ac, seq) {
                    self.ts.wordspace = aw;
                    self.ts.charspace = ac;
                    if matches!(seq, Tok::Str(_) | Tok::Hex(_)) {
                        self.show_seq(&[seq]);
                    }
                }
                self.stack.clear();
                Ok(())
            }
            "Tc" => {
                if let Some(Num(v)) = self.stack.last() {
                    self.ts.charspace = *v;
                }
                self.stack.clear();
                Ok(())
            }
            "Tw" => {
                if let Some(Num(v)) = self.stack.last() {
                    self.ts.wordspace = *v;
                }
                self.stack.clear();
                Ok(())
            }
            "Do" => {
                // A form XObject runs its own content stream with the
                // form's matrix prepended and its own resources.
                let name = match self.stack.last() {
                    Some(Tok::Name(name)) => name.clone(),
                    _ => String::new(),
                };
                self.stack.clear();
                if !name.is_empty() {
                    self.run_form(&name)?;
                }
                Ok(())
            }
            "Tz" => {
                if let Some(Num(v)) = self.stack.last() {
                    self.ts.scaling = *v;
                }
                self.stack.clear();
                Ok(())
            }
            "TL" => {
                if let Some(Num(v)) = self.stack.last() {
                    self.ts.leading = -v;
                }
                self.stack.clear();
                Ok(())
            }
            "Ts" => {
                if let Some(Num(v)) = self.stack.last() {
                    self.ts.rise = *v;
                }
                self.stack.clear();
                Ok(())
            }
            _ => {
                self.stack.clear();
                Ok(())
            }
        }
    }

    fn matrix_args(&mut self) -> Option<[f64; 6]> {
        if self.stack.len() >= 6 {
            let args = &self.stack[self.stack.len() - 6..];
            let mut m = [0.0f64; 6];
            for (i, tok) in args.iter().enumerate() {
                match tok {
                    Tok::Num(n) => m[i] = *n,
                    _ => return None,
                }
            }
            return Some(m);
        }
        None
    }

    /// `Tj` and friends: the stack holds one string.
    fn show_text(&mut self) {
        if let Some(last) = self.stack.last() {
            if matches!(last, Tok::Str(_) | Tok::Hex(_)) {
                let seq = vec![last.clone()];
                self.show_seq(&seq);
            }
        }
    }

    /// The displacement loop from pdfminer's `render_string_horizontal`.
    fn show_seq(&mut self, seq: &[Tok]) {
        let Some(font) = self.ts.font.clone() else {
            return;
        };
        let fontsize = self.ts.fontsize;
        let scaling = self.ts.scaling * 0.01;
        let charspace = self.ts.charspace * scaling;
        let wordspace = if font.two_byte {
            0.0
        } else {
            self.ts.wordspace * scaling
        };
        let dxscale = 0.001 * fontsize * scaling;
        let matrix = mult(&self.ts.matrix, &self.ctm);
        let mut x = self.ts.line_x;
        let y = self.ts.line_y;
        let mut needcharspace = false;
        for tok in seq {
            match tok {
                Tok::Num(n) => {
                    x -= n * dxscale;
                    needcharspace = true;
                }
                Tok::Str(bytes) | Tok::Hex(bytes) => {
                    for cid in font.decode(bytes) {
                        if needcharspace {
                            x += charspace;
                        }
                        let char_matrix = translate(&matrix, (x, y));
                        self.emit_char(&char_matrix, &font, cid, fontsize, scaling);
                        x += font.char_width(cid) * 0.001 * fontsize * scaling;
                        if cid == 32 && wordspace != 0.0 {
                            x += wordspace;
                        }
                        needcharspace = true;
                    }
                }
                _ => {}
            }
        }
        self.ts.line_x = x;
        self.ts.line_y = y;
    }

    /// One `LTChar`: advance, bounding box, upright, size.
    fn emit_char(
        &mut self,
        m: &[f64; 6],
        font: &PdfFont,
        cid: u32,
        fontsize: f64,
        scaling: f64,
    ) {
        let text = font
            .to_unichr(cid)
            .unwrap_or_else(|| format!("(cid:{cid})"));
        let adv = font.char_width(cid) * 0.001 * fontsize * scaling;
        let descent = font.descent * 0.001;
        let rise = self.ts.rise;
        let bbox = (0.0, descent * fontsize + rise, adv, descent * fontsize + rise + fontsize);
        let upright = m[0] * m[3] * scaling > 0.0 && m[1] * m[2] <= 0.0;
        let (mut x0, mut y0, mut x1, mut y1) = apply_matrix_rect(m, bbox);
        if x1 < x0 {
            std::mem::swap(&mut x0, &mut x1);
        }
        if y1 < y0 {
            std::mem::swap(&mut y0, &mut y1);
        }
        self.chars.push(RawChar {
            text,
            x0,
            x1,
            y1,
            upright,
        });
    }
}

/// `q`/`Q` save the CTM and the whole text state, like the interpreter's
impl Interpreter<'_, '_> {
    /// Paint a form XObject: its content stream runs with the form's
    /// `/Matrix` prepended to the CTM and its own `/Resources` in scope;
    /// the whole text state is saved and restored around it, like
    /// pdfminer's push_state around forms.
    fn run_form(&mut self, name: &str) -> Result<()> {
        let Some(xobj) = self
            .resources
            .as_ref()
            .and_then(|res| res.dict_get("XObject"))
            .and_then(|table| table.dict_get(name))
            .and_then(|obj| deref(obj, self.src, 0))
        else {
            return Ok(());
        };
        let Obj::Stream { dict, data } = xobj else {
            return Ok(());
        };
        let dict_obj = Obj::Dict(dict.clone());
        if dict_obj.name_val("Subtype") != Some("Form") {
            return Ok(());
        }
        let content = crate::pdf::decode_stream(&dict, &data)?;
        let matrix = match dict_obj.dict_get("Matrix") {
            Some(Obj::Arr(items)) if items.len() == 6 => {
                let mut m = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];
                for (i, item) in items.iter().enumerate() {
                    m[i] = obj_int(item).unwrap_or(if i == 0 { 1 } else { 0 }) as f64;
                }
                m
            }
            _ => [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        };
        let saved_ctm = self.ctm;
        let saved_ts = self.ts.clone();
        let saved_resources = self.resources.clone();
        let saved_stack = std::mem::take(&mut self.stack);
        self.ctm = mult(&matrix, &self.ctm);
        if let Some(res) = dict_obj.dict_get("Resources") {
            self.resources = Some(res.clone());
        }
        // The form's fonts resolve against its own resources.
        let mut form_fonts = Fonts::new();
        if let Some(res) = &self.resources {
            if let Some(font_root) = res.dict_get("Font") {
                form_fonts.set_root(font_root.clone());
            }
        }
        std::mem::swap(self.fonts, &mut form_fonts);
        let mut nested = Interpreter {
            src: self.src,
            fonts: self.fonts,
            resources: self.resources.clone(),
            chars: std::mem::take(&mut self.chars),
            ctm: self.ctm,
            ts: self.ts.clone(),
            stack: std::mem::take(&mut self.stack),
            saved: std::mem::take(&mut self.saved),
        };
        nested.run(&content)?;
        let out = nested.chars;
        self.chars = out;
        self.ctm = nested.ctm;
        self.ts = nested.ts;
        self.stack = nested.stack;
        self.saved = nested.saved;
        std::mem::swap(self.fonts, &mut form_fonts);
        self.ctm = saved_ctm;
        self.ts = saved_ts;
        self.resources = saved_resources;
        self.stack = saved_stack;
        Ok(())
    }
}

fn shift_line(ts: &mut TextState, tx: f64, ty: f64) {
    let [a, b, c, d, e, f] = ts.matrix;
    ts.matrix = [a, b, c, d, tx * a + ty * c + e, tx * b + ty * d + f];
    ts.line_x = 0.0;
    ts.line_y = 0.0;
}

/// pdfminer's matrix order: `mult(m1, m0)` applies `m1` first, then `m0`.
fn mult(m1: &[f64; 6], m0: &[f64; 6]) -> [f64; 6] {
    let &[a1, b1, c1, d1, e1, f1] = m1;
    let &[a0, b0, c0, d0, e0, f0] = m0;
    [
        a0 * a1 + c0 * b1,
        b0 * a1 + d0 * b1,
        a0 * c1 + c0 * d1,
        b0 * c1 + d0 * d1,
        a0 * e1 + c0 * f1 + e0,
        b0 * e1 + d0 * f1 + f0,
    ]
}

fn translate(m: &[f64; 6], (x, y): (f64, f64)) -> [f64; 6] {
    let [a, b, c, d, e, f] = *m;
    [a, b, c, d, x * a + y * c + e, x * b + y * d + f]
}

fn apply_matrix_pt(m: &[f64; 6], (x, y): (f64, f64)) -> (f64, f64) {
    let [a, b, c, d, e, f] = *m;
    (a * x + c * y + e, b * x + d * y + f)
}

/// The rectangle's four corners through the matrix, tightest axis box.
fn apply_matrix_rect(m: &[f64; 6], (x0, y0, x1, y1): (f64, f64, f64, f64)) -> (f64, f64, f64, f64) {
    let corners = [
        apply_matrix_pt(m, (x0, y0)),
        apply_matrix_pt(m, (x1, y0)),
        apply_matrix_pt(m, (x1, y1)),
        apply_matrix_pt(m, (x0, y1)),
    ];
    let mut out = [f64::INFINITY, f64::INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY];
    for (px, py) in corners {
        out[0] = out[0].min(px);
        out[1] = out[1].min(py);
        out[2] = out[2].max(px);
        out[3] = out[3].max(py);
    }
    (out[0], out[1], out[2], out[3])
}

// ---------------------------------------------------------------------------
// pdfplumber's shaping
// ---------------------------------------------------------------------------

const LIGATURES: [(&str, &str); 7] = [
    ("\u{fb00}", "ff"),
    ("\u{fb03}", "ffi"),
    ("\u{fb04}", "ffl"),
    ("\u{fb01}", "fi"),
    ("\u{fb02}", "fl"),
    ("\u{fb05}", "st"),
    ("\u{fb06}", "st"),
];

fn expand_ligatures(text: &str) -> String {
    for (from, to) in LIGATURES {
        if text == from {
            return to.to_string();
        }
    }
    text.into()
}

struct Word {
    text: String,
    top: f64,
}

/// pdfplumber's `extract_text`: tops cluster into lines, characters sort
/// by x, gaps and top shifts start words, words join with spaces.
fn shape(chars: &[PChar]) -> String {
    if chars.is_empty() {
        return String::new();
    }
    let mut words: Vec<Word> = Vec::new();
    // The upright partition comes first, matching the reference's groupby.
    let mut groups: Vec<Vec<&PChar>> = Vec::new();
    for c in chars {
        match groups.last_mut() {
            Some(group) if group[0].upright == c.upright => group.push(c),
            _ => groups.push(vec![c]),
        }
    }
    for group in groups {
        for line in cluster(group, Y_TOLERANCE, |c| c.top) {
            let mut sorted = line;
            sorted.sort_by(|a, b| a.x0.partial_cmp(&b.x0).unwrap_or(std::cmp::Ordering::Equal));
            let mut current: Vec<&PChar> = Vec::new();
            for ch in sorted {
                let blank = ch.text.chars().all(char::is_whitespace);
                if blank {
                    flush_word(&mut current, &mut words);
                    continue;
                }
                if let Some(prev) = current.last() {
                    let new_word =
                        ch.x0 > prev.x1 + X_TOLERANCE || (ch.top - prev.top).abs() > Y_TOLERANCE;
                    if new_word {
                        flush_word(&mut current, &mut words);
                    }
                }
                current.push(ch);
            }
            flush_word(&mut current, &mut words);
        }
    }
    // The word-level pass keeps first-encounter order: `to_textmap` runs
    // with `presorted` on, so a word's cluster id comes from the sorted
    // distinct tops, but consecutive runs of the same id form the lines.
    let ids = cluster_ids(words.iter().map(|w| w.top), Y_TOLERANCE);
    let mut lines: Vec<Vec<&str>> = vec![Vec::new()];
    let mut current_id = ids.first().copied().unwrap_or(0);
    for (word, id) in words.iter().zip(&ids) {
        if *id != current_id {
            lines.push(Vec::new());
            current_id = *id;
        }
        let last = lines.last_mut().expect("at least one line");
        last.push(word.text.as_str());
    }
    lines
        .iter()
        .map(|line| line.join(" "))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Cluster ids for a numeric key: distinct values sort, a value within
/// `tolerance` of the running last joins the cluster.
fn cluster_ids(values: impl Iterator<Item = f64>, tolerance: f64) -> Vec<usize> {
    let mut keyed: Vec<(f64, usize)> = values.enumerate().map(|(i, v)| (v, i)).collect();
    keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut ids = vec![0usize; keyed.len()];
    let mut cluster_count = 0usize;
    let mut last = 0.0f64;
    for (i, (value, index)) in keyed.iter().enumerate() {
        if i > 0 && *value > last + tolerance {
            cluster_count += 1;
        }
        ids[*index] = cluster_count;
        last = *value;
    }
    ids
}

fn flush_word(current: &mut Vec<&PChar>, words: &mut Vec<Word>) {
    if current.is_empty() {
        return;
    }
    let text: String = current
        .iter()
        .map(|c| expand_ligatures(&c.text))
        .collect();
    let top = current.iter().map(|c| c.top).fold(f64::INFINITY, f64::min);
    words.push(Word { text, top });
    current.clear();
}

/// Cluster by a numeric key: distinct values sort, a value within
/// `tolerance` of the running last joins the cluster, and objects keep
/// their original order inside their cluster. Returns clusters in
/// ascending key order.
fn cluster<T, F>(items: Vec<T>, tolerance: f64, key: F) -> Vec<Vec<T>>
where
    F: Fn(&T) -> f64,
{
    let mut keyed: Vec<(f64, usize)> = items.iter().enumerate().map(|(i, t)| (key(t), i)).collect();
    keyed.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));
    let mut ids: Vec<Option<usize>> = vec![None; items.len()];
    let mut cluster_count = 0usize;
    let mut last = 0.0f64;
    for (i, (value, index)) in keyed.iter().enumerate() {
        if i > 0 && *value > last + tolerance {
            cluster_count += 1;
        }
        ids[*index] = Some(cluster_count);
        last = *value;
    }
    let mut out: Vec<Vec<T>> = (0..=cluster_count).map(|_| Vec::new()).collect();
    for (item, id) in items.into_iter().zip(ids) {
        out[id.unwrap_or(0)].push(item);
    }
    out.retain(|group| !group.is_empty());
    out
}



#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_bfchar_and_bfrange() {
        let cmap = b"/CIDInit /ProcSet findresource begin
12 dict begin
begincmap
/CMapName /Custom def
1 begincodespacerange
<0000> <FFFF>
endcodespacerange
2 beginbfchar
<FB01> <FB01>
<1E9E> <0053 0073>
endbfchar
2 beginbfrange
<0041> <0043> <0041>
<0061> <0063> [ <0061> <0062> <0063> ]
endbfrange
endcmap
end";
        let map = parse_to_unicode(cmap);
        assert_eq!(map.get(&0xFB01).map(String::as_str), Some("\u{fb01}"));
        assert_eq!(map.get(&0x1E9E).map(String::as_str), Some("Ss"));
        assert_eq!(map.get(&0x41).map(String::as_str), Some("A"));
        assert_eq!(map.get(&0x43).map(String::as_str), Some("C"));
        assert_eq!(map.get(&0x61).map(String::as_str), Some("a"));
        assert_eq!(map.get(&0x63).map(String::as_str), Some("c"));
    }
}

#[test]
fn crate_generated_pdf_extracts_every_span() {
    let lines: Vec<crate::pdf::Line> = vec![
        crate::pdf::Line {
            spans: vec![crate::pdf::Span {
                text: "Invoice 2024-001".into(),
                font: 1,
                size: 16.0,
            }],
            leading: 20.0,
        },
        crate::pdf::Line {
            spans: vec![crate::pdf::Span {
                text: "Customer: Northwind Traders".into(),
                font: 0,
                size: 11.0,
            }],
            leading: 16.0,
        },
        crate::pdf::Line {
            spans: vec![crate::pdf::Span {
                text: "Total due: 1,234.56 EUR".into(),
                font: 0,
                size: 11.0,
            }],
            leading: 16.0,
        },
    ];
    let pdf = crate::pdf::from_lines(&lines).unwrap();
    let text = layout_text_pages(&pdf).unwrap().join("

");
    assert!(text.contains("Invoice"), "{text}");
    assert!(text.contains("2024-001"), "{text}");
    assert!(text.contains("Customer"), "{text}");
    assert!(text.contains("Northwind Traders"), "{text}");
    assert!(text.contains("Total"), "{text}");
    assert!(text.contains("1,234.56"), "{text}");
}
