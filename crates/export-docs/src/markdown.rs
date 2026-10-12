//! A small markdown subset: headings, lists, fences, emphasis, links.
//! Enough for export's notes → html hop. Not a pandoc clone.

pub fn markdown_to_html(input: &str) -> String {
    let mut out = String::from("<!DOCTYPE html>\n<html><head><meta charset=\"utf-8\"></head><body>\n");
    render(input, &mut out, true);
    out.push_str("</body></html>\n");
    out
}

pub fn markdown_to_text(input: &str) -> String {
    let mut out = String::new();
    render(input, &mut out, false);
    out
}

fn render(input: &str, out: &mut String, html: bool) {
    let lines: Vec<&str> = input.split('\n').collect();
    let mut i = 0;
    let mut para: Vec<String> = Vec::new();
    let flush_para = |para: &mut Vec<String>, out: &mut String| {
        if para.is_empty() {
            return;
        }
        let text = para.join(" ");
        if html {
            out.push_str("<p>");
            inline(&text, out, true);
            out.push_str("</p>\n");
        } else {
            inline(&text, out, false);
            out.push('\n');
        }
        para.clear();
    };
    while i < lines.len() {
        let line = lines[i].trim_end_matches('\r');
        if let Some(rest) = line.trim_start().strip_prefix("```") {
            let _lang = rest;
            flush_para(&mut para, out);
            i += 1;
            let mut body = String::new();
            while i < lines.len() && !lines[i].trim_end_matches('\r').trim_start().starts_with("```") {
                body.push_str(lines[i].trim_end_matches('\r'));
                body.push('\n');
                i += 1;
            }
            if i < lines.len() {
                i += 1;
            }
            if html {
                out.push_str("<pre><code>");
                escape(&body, out);
                out.push_str("</code></pre>\n");
            } else {
                out.push_str(&body);
                if !body.ends_with('\n') {
                    out.push('\n');
                }
            }
            continue;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            flush_para(&mut para, out);
            i += 1;
            continue;
        }
        if trimmed.starts_with('#') {
            flush_para(&mut para, out);
            let mut level = 0u8;
            let bytes = trimmed.as_bytes();
            while (level as usize) < bytes.len() && bytes[level as usize] == b'#' && level < 6 {
                level += 1;
            }
            let boundary_ok = match trimmed.as_bytes().get(level as usize) {
                None | Some(b' ') => true,
                Some(_) => false,
            };
            if level > 0 && boundary_ok {
                let text = trimmed[level as usize..].trim();
                if html {
                    out.push_str(&format!("<h{level}>"));
                    inline(text, out, true);
                    out.push_str(&format!("</h{level}>\n"));
                } else {
                    inline(text, out, false);
                    out.push('\n');
                }
                i += 1;
                continue;
            }
        }
        if trimmed == "---" || trimmed == "***" {
            flush_para(&mut para, out);
            if html {
                out.push_str("<hr>\n");
            } else {
                out.push('\n');
            }
            i += 1;
            continue;
        }
        if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            flush_para(&mut para, out);
            if html {
                out.push_str("<ul>\n");
            }
            while i < lines.len() {
                let item = lines[i].trim_end_matches('\r').trim();
                let Some(text) = item.strip_prefix("- ").or_else(|| item.strip_prefix("* ")) else {
                    break;
                };
                if html {
                    out.push_str("<li>");
                    inline(text, out, true);
                    out.push_str("</li>\n");
                } else {
                    out.push_str("- ");
                    inline(text, out, false);
                    out.push('\n');
                }
                i += 1;
            }
            if html {
                out.push_str("</ul>\n");
            }
            continue;
        }
        if let Some(text) = trimmed.strip_prefix("> ") {
            flush_para(&mut para, out);
            if html {
                out.push_str("<blockquote><p>");
                inline(text, out, true);
                out.push_str("</p></blockquote>\n");
            } else {
                inline(text, out, false);
                out.push('\n');
            }
            i += 1;
            continue;
        }
        para.push(trimmed.to_string());
        i += 1;
    }
    flush_para(&mut para, out);
}

fn inline(text: &str, out: &mut String, html: bool) {
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if html && b[i..].starts_with(b"**") {
            if let Some(end) = text[i + 2..].find("**") {
                out.push_str("<strong>");
                escape(&text[i + 2..i + 2 + end], out);
                out.push_str("</strong>");
                i += 4 + end;
                continue;
            }
        }
        if html && b[i] == b'*' {
            if let Some(end) = text[i + 1..].find('*') {
                out.push_str("<em>");
                escape(&text[i + 1..i + 1 + end], out);
                out.push_str("</em>");
                i += 2 + end;
                continue;
            }
        }
        if html && b[i] == b'`' {
            if let Some(end) = text[i + 1..].find('`') {
                out.push_str("<code>");
                escape(&text[i + 1..i + 1 + end], out);
                out.push_str("</code>");
                i += 2 + end;
                continue;
            }
        }
        if html && b[i] == b'[' {
            if let Some(mid) = text[i + 1..].find("](") {
                let label = &text[i + 1..i + 1 + mid];
                let rest = &text[i + 1 + mid + 2..];
                if let Some(end) = rest.find(')') {
                    let href = &rest[..end];
                    out.push_str("<a href=\"");
                    escape(href, out);
                    out.push_str("\">");
                    escape(label, out);
                    out.push_str("</a>");
                    i = i + 1 + mid + 2 + end + 1;
                    continue;
                }
            }
        }
        if !html && (b[i] == b'*' || b[i] == b'`') {
            i += 1;
            continue;
        }
        let ch = text[i..].chars().next().unwrap_or(' ');
        if html {
            push_escaped(ch, out);
        } else {
            out.push(ch);
        }
        i += ch.len_utf8();
    }
}

fn escape(text: &str, out: &mut String) {
    for ch in text.chars() {
        push_escaped(ch, out);
    }
}

fn push_escaped(ch: char, out: &mut String) {
    match ch {
        '&' => out.push_str("&amp;"),
        '<' => out.push_str("&lt;"),
        '>' => out.push_str("&gt;"),
        '"' => out.push_str("&quot;"),
        _ => out.push(ch),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_emphasis_and_list() {
        let html = markdown_to_html("# Title\n\nA **bold** word.\n\n- one\n- two\n");
        assert!(html.contains("<h1>Title</h1>"));
        assert!(html.contains("<strong>bold</strong>"));
        assert!(html.contains("<li>one</li>"));
        let text = markdown_to_text("# Title\n\nA **bold** word.\n");
        assert!(text.contains("Title"));
        assert!(text.contains("bold"));
        assert!(!text.contains("**"));
    }
}
