//! HTML subset to plain text and to a text-only PDF. No browser, no WeasyPrint.

use crate::error::{Error, Result};
use crate::pdf::{self, Line, Span};
use crate::xml;

const CONTENT_W: f32 = 612.0 - 108.0;

pub fn html_to_text(html: &str) -> Result<String> {
    let lines = layout(html)?;
    let mut out = String::new();
    for line in lines {
        for span in &line.spans {
            out.push_str(&span.text);
        }
        out.push('\n');
    }
    Ok(out)
}

pub fn html_to_pdf(html: &str) -> Result<Vec<u8>> {
    let lines = layout(html)?;
    pdf::from_lines(&lines)
}

struct Piece {
    text: String,
    font: u8,
    size: f32,
}

struct Block {
    pieces: Vec<Piece>,
    size: f32,
    leading: f32,
}

fn layout(html: &str) -> Result<Vec<Line>> {
    let blocks = blocks(html)?;
    let mut lines = Vec::new();
    for block in blocks {
        wrap_block(&block, &mut lines);
    }
    Ok(lines)
}

fn blocks(html: &str) -> Result<Vec<Block>> {
    let mut out = Vec::new();
    let mut cur = Block {
        pieces: Vec::new(),
        size: 11.0,
        leading: 14.0,
    };
    let mut bold = 0i32;
    let mut italic = 0i32;
    let mut skip = 0i32;
    let b = html.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'<' {
            let end = html[i..].find('>').ok_or_else(|| Error::new("html: truncated tag"))? + i;
            let raw = &html[i + 1..end];
            i = end + 1;
            let body = raw.trim();
            if body.starts_with('!') || body.starts_with('?') {
                continue;
            }
            let closing = body.starts_with('/');
            let name_src = body.trim_start_matches('/').trim_end_matches('/');
            let name = name_src.split_whitespace().next().unwrap_or("").to_ascii_lowercase();
            if name == "script" || name == "style" {
                if closing {
                    skip = (skip - 1).max(0);
                } else if !body.ends_with('/') {
                    skip += 1;
                }
                continue;
            }
            if skip > 0 {
                continue;
            }
            match name.as_str() {
                "p" | "div" | "blockquote" | "li" | "tr" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    if closing {
                        push_block(&mut out, &mut cur);
                    } else if !body.ends_with('/') {
                        push_block(&mut out, &mut cur);
                        let (size, leading) = match name.as_str() {
                            "h1" => (22.0, 28.0),
                            "h2" => (18.0, 24.0),
                            "h3" => (14.0, 18.0),
                            _ => (11.0, 14.0),
                        };
                        cur.size = size;
                        cur.leading = leading;
                        if name == "li" {
                            cur.pieces.push(Piece {
                                text: "- ".into(),
                                font: 0,
                                size,
                            });
                        }
                    }
                }
                "br" | "hr" => {
                    push_block(&mut out, &mut cur);
                }
                "strong" | "b" => {
                    if closing {
                        bold = (bold - 1).max(0);
                    } else {
                        bold += 1;
                    }
                }
                "em" | "i" => {
                    if closing {
                        italic = (italic - 1).max(0);
                    } else {
                        italic += 1;
                    }
                }
                "td" | "th" if !closing => {
                    if !cur.pieces.is_empty() {
                        cur.pieces.push(Piece {
                            text: " ".into(),
                            font: font_of(bold, italic),
                            size: cur.size,
                        });
                    }
                }
                _ => {}
            }
            continue;
        }
        if skip > 0 {
            i += 1;
            continue;
        }
        let end = html[i..].find('<').map(|n| n + i).unwrap_or(html.len());
        let raw = &html[i..end];
        i = end;
        let decoded = xml::unescape(raw).unwrap_or_else(|_| raw.to_string());
        if decoded.trim().is_empty() && cur.pieces.is_empty() {
            continue;
        }
        cur.pieces.push(Piece {
            text: decoded,
            font: font_of(bold, italic),
            size: cur.size,
        });
    }
    push_block(&mut out, &mut cur);
    Ok(out)
}

fn font_of(bold: i32, italic: i32) -> u8 {
    match (bold > 0, italic > 0) {
        (true, true) => 3,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 0,
    }
}

fn push_block(out: &mut Vec<Block>, cur: &mut Block) {
    if cur.pieces.iter().all(|p| p.text.trim().is_empty()) {
        cur.pieces.clear();
        return;
    }
    out.push(Block {
        pieces: std::mem::take(&mut cur.pieces),
        size: cur.size,
        leading: cur.leading,
    });
    cur.size = 11.0;
    cur.leading = 14.0;
}

fn wrap_block(block: &Block, lines: &mut Vec<Line>) {
    let mut spans: Vec<Span> = Vec::new();
    let mut width = 0.0f32;
    let flush = |spans: &mut Vec<Span>, lines: &mut Vec<Line>, leading: f32| {
        if spans.is_empty() {
            return;
        }
        lines.push(Line {
            spans: std::mem::take(spans),
            leading,
        });
    };
    for piece in &block.pieces {
        for word in split_words(&piece.text) {
            let w = pdf::text_width(word, piece.size);
            if width + w > CONTENT_W && width > 0.0 {
                flush(&mut spans, lines, block.leading);
                width = 0.0;
            }
            spans.push(Span {
                text: word.to_string(),
                font: piece.font,
                size: piece.size,
            });
            width += w;
        }
    }
    flush(&mut spans, lines, block.leading);
}

fn split_words(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            if start < i {
                out.push(&text[start..i]);
            }
            let ws = i;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            out.push(&text[ws..i]);
            start = i;
        } else {
            i += 1;
        }
    }
    if start < text.len() {
        out.push(&text[start..]);
    }
    out
}

pub fn text_to_html(text: &str) -> String {
    let mut out = String::from("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head><body>\n");
    for para in text.split("\n\n") {
        let para = para.trim();
        if para.is_empty() {
            continue;
        }
        out.push_str("<p>");
        for ch in para.chars() {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '\n' => out.push(' '),
                _ => out.push(ch),
            }
        }
        out.push_str("</p>\n");
    }
    out.push_str("</body></html>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_becomes_a_pdf_with_the_words() {
        let pdf = html_to_pdf("<h1>Hello</h1><p>A <strong>bold</strong> line.</p>").unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        let text = pdf::plain_text(&pdf).unwrap();
        assert!(text.contains("Hello"), "{text}");
        assert!(text.contains("bold"), "{text}");
        assert_eq!(pdf::page_count(&pdf).unwrap(), 1);
    }
}
