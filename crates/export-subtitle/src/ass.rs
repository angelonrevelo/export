//! ASS/SSA dialogue cues. One `{\b1}`...`{\b0}` pair on a dialogue line is kept,
//! so that span stays bold. On a line that does not keep that bold pair, one
//! `{\i1}`...`{\i0}` pair is kept. On a line that keeps neither, one
//! `{\u1}`...`{\u0}` pair is kept. On a line that keeps none of those, one
//! `{\s1}`...`{\s0}` pair is kept. Every other override is stripped.
//! Style, actor, and margin fields are not preserved. That is the honest limit.

use crate::parse::{Cue, Error};

pub fn parse_ass(input: &str) -> Result<Vec<Cue>, Error> {
    let mut cues = Vec::new();
    let mut in_events = false;
    let mut text_index = 9usize;
    for line in input.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_events = line.eq_ignore_ascii_case("[Events]");
            continue;
        }
        if !in_events {
            continue;
        }
        if let Some(rest) = line.strip_prefix("Format:") {
            text_index = format_text_index(rest);
            continue;
        }
        let Some(rest) = line.strip_prefix("Dialogue:") else {
            continue;
        };
        let rest = rest.trim();
        if rest.is_empty() {
            return Err(Error::Truncated);
        }
        let fields = split_fields(rest, text_index);
        if fields.len() <= text_index {
            return Err(Error::Truncated);
        }
        let start = parse_ass_stamp(fields[1].trim())?;
        let end = parse_ass_stamp(fields[2].trim())?;
        let text = strip_overrides(fields[text_index].trim());
        if text.is_empty() {
            return Err(Error::Truncated);
        }
        cues.push(Cue {
            start_ms: start,
            end_ms: end,
            text,
        });
    }
    Ok(cues)
}

pub fn cues_to_ass(cues: &[Cue]) -> String {
    let mut out = String::from(
        "[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n",
    );
    for cue in cues {
        out.push_str("Dialogue: 0,");
        out.push_str(&stamp(cue.start_ms));
        out.push(',');
        out.push_str(&stamp(cue.end_ms));
        out.push_str(",Default,,0,0,0,,");
        out.push_str(&cue.text.replace('\n', "\\N"));
        out.push('\n');
    }
    out
}

fn format_text_index(format: &str) -> usize {
    format
        .split(',')
        .position(|name| name.trim().eq_ignore_ascii_case("Text"))
        .unwrap_or(9)
}

fn split_fields(line: &str, text_index: usize) -> Vec<&str> {
    let mut fields = Vec::new();
    let mut start = 0;
    let mut commas = 0;
    for (i, byte) in line.bytes().enumerate() {
        if byte == b',' {
            fields.push(&line[start..i]);
            start = i + 1;
            commas += 1;
            if commas == text_index {
                fields.push(&line[start..]);
                return fields;
            }
        }
    }
    fields.push(&line[start..]);
    fields
}

fn parse_ass_stamp(raw: &str) -> Result<u64, Error> {
    if raw.is_empty() {
        return Err(Error::Truncated);
    }
    let (hms, frac) = raw.split_once('.').ok_or(Error::BadTime)?;
    if frac.is_empty() || !frac.bytes().all(|c| c.is_ascii_digit()) {
        return Err(Error::Truncated);
    }
    let frac_ms = match frac.len() {
        1 => frac.parse::<u64>().map_err(|_| Error::BadTime)? * 100,
        2 => frac.parse::<u64>().map_err(|_| Error::BadTime)? * 10,
        _ => frac[..3].parse::<u64>().map_err(|_| Error::BadTime)?,
    };
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [m, s] => (0u64, num(m)?, num(s)?),
        [h, m, s] => (num(h)?, num(m)?, num(s)?),
        _ => return Err(Error::BadTime),
    };
    Ok(((h * 60 + m) * 60 + s) * 1000 + frac_ms)
}

fn num(s: &str) -> Result<u64, Error> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
        return Err(Error::BadTime);
    }
    s.parse().map_err(|_| Error::BadTime)
}

fn stamp(ms: u64) -> String {
    let h = ms / 3_600_000;
    let m = (ms / 60_000) % 60;
    let s = (ms / 1000) % 60;
    let cs = (ms % 1000) / 10;
    format!("{h}:{m:02}:{s:02}.{cs:02}")
}

fn strip_overrides(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let bytes = text.as_bytes();
    let mut i = 0;
    let mut bold_at: Option<usize> = None;
    let mut bold_kept = false;
    let mut italic_open = false;
    let mut italic_kept = false;
    let mut underline_open = false;
    let mut underline_kept = false;
    let mut strike_open = false;
    let mut strike_kept = false;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'\\' {
            if let Some(rel) = bytes[i + 2..].iter().position(|&b| b == b'}') {
                let end = i + 2 + rel + 1;
                let tag = &text[i..end];
                if !bold_kept && bold_at.is_none() && tag == "{\\b1}" {
                    bold_at = Some(out.len());
                    out.push_str(tag);
                } else if bold_at.is_some() && tag == "{\\b0}" {
                    out.push_str(tag);
                    bold_at = None;
                    bold_kept = true;
                } else if !bold_kept && !italic_kept && !italic_open && tag == "{\\i1}" {
                    italic_open = true;
                    out.push_str(tag);
                } else if !bold_kept && italic_open && tag == "{\\i0}" {
                    out.push_str(tag);
                    italic_open = false;
                    italic_kept = true;
                } else if !bold_kept
                    && !italic_kept
                    && !underline_kept
                    && !underline_open
                    && tag == "{\\u1}"
                {
                    underline_open = true;
                    out.push_str(tag);
                } else if !bold_kept && !italic_kept && underline_open && tag == "{\\u0}" {
                    out.push_str(tag);
                    underline_open = false;
                    underline_kept = true;
                } else if !bold_kept
                    && !italic_kept
                    && !underline_kept
                    && !strike_kept
                    && !strike_open
                    && tag == "{\\s1}"
                {
                    strike_open = true;
                    out.push_str(tag);
                } else if !bold_kept
                    && !italic_kept
                    && !underline_kept
                    && strike_open
                    && tag == "{\\s0}"
                {
                    out.push_str(tag);
                    strike_open = false;
                    strike_kept = true;
                }
                i = end;
                continue;
            }
            break;
        }
        if bytes[i] == b'\\' && i + 1 < bytes.len() {
            match bytes[i + 1] {
                b'N' | b'n' => {
                    out.push('\n');
                    i += 2;
                    continue;
                }
                b'h' => {
                    out.push(' ');
                    i += 2;
                    continue;
                }
                _ => {}
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    if let Some(at) = bold_at {
        let end = at + "{\\b1}".len();
        out.replace_range(at..end, "");
    }
    if bold_kept {
        if italic_open || italic_kept {
            drop_tag(&mut out, "{\\i1}");
            drop_tag(&mut out, "{\\i0}");
        }
        if underline_open || underline_kept {
            drop_tag(&mut out, "{\\u1}");
            drop_tag(&mut out, "{\\u0}");
        }
        if strike_open || strike_kept {
            drop_tag(&mut out, "{\\s1}");
            drop_tag(&mut out, "{\\s0}");
        }
    } else if italic_kept {
        if underline_open || underline_kept {
            drop_tag(&mut out, "{\\u1}");
            drop_tag(&mut out, "{\\u0}");
        }
        if strike_open || strike_kept {
            drop_tag(&mut out, "{\\s1}");
            drop_tag(&mut out, "{\\s0}");
        }
    } else if underline_kept {
        if strike_open || strike_kept {
            drop_tag(&mut out, "{\\s1}");
            drop_tag(&mut out, "{\\s0}");
        }
    } else if italic_open {
        drop_tag(&mut out, "{\\i1}");
    }
    if !bold_kept && !italic_kept && underline_open {
        drop_tag(&mut out, "{\\u1}");
    }
    if !bold_kept && !italic_kept && !underline_kept && strike_open {
        drop_tag(&mut out, "{\\s1}");
    }
    out
}

fn drop_tag(out: &mut String, tag: &str) {
    if let Some(at) = out.find(tag) {
        out.replace_range(at..at + tag.len(), "");
    }
}

#[cfg(test)]
mod tests {
    use super::{cues_to_ass, parse_ass};
    use crate::parse::Error;

    const ASS: &str = "[Script Info]\nScriptType: v4.00+\n\n[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.50,Default,,0,0,0,,Hello {\\b1}there\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,Next\\Nline\n";

    #[test]
    fn strips_tags_and_round_trips_plain_text() {
        let cues = parse_ass(ASS).unwrap();
        assert_eq!(cues[0].start_ms, 1000);
        assert_eq!(cues[0].end_ms, 2500);
        assert_eq!(cues[0].text, "Hello there");
        assert_eq!(cues[1].text, "Next\nline");
        let again = parse_ass(&cues_to_ass(&cues)).unwrap();
        assert_eq!(again, cues);
    }

    #[test]
    fn truncated_dialogue_errors() {
        let input = "[Events]\nDialogue: 0,0:00:01.00\n";
        assert_eq!(parse_ass(input), Err(Error::Truncated));
    }

    #[test]
    fn keeps_one_bold_pair_and_leaves_a_plain_cue() {
        let input = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\an8}Hello {\\b1}there{\\b0} {\\i1}pal{\\i0} {\\b1}two{\\b0}\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,Plain cue\n";
        let cues = parse_ass(input).unwrap();
        assert_eq!(cues[0].text, "Hello {\\b1}there{\\b0} pal two");
        assert_eq!(cues[1].text, "Plain cue");
        let written = cues_to_ass(&cues);
        assert!(written.contains("{\\b1}there{\\b0}"));
        assert!(!written.contains("{\\i"));
        assert!(!written.contains("{\\an"));
        let again = parse_ass(&written).unwrap();
        assert_eq!(again, cues);
        assert_eq!(again[1].text, "Plain cue");
    }

    #[test]
    fn keeps_one_italic_pair_and_the_bold_line() {
        let input = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\an8}Hello {\\b1}there{\\b0} {\\i1}pal{\\i0} {\\b1}two{\\b0}\nDialogue: 0,0:00:02.00,0:00:03.00,Default,,0,0,0,,{\\an8}Hello {\\i1}there{\\i0} {\\u1}pal{\\u0} {\\i1}two{\\i0}\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,Plain cue\n";
        let cues = parse_ass(input).unwrap();
        assert_eq!(cues[0].text, "Hello {\\b1}there{\\b0} pal two");
        assert_eq!(cues[1].text, "Hello {\\i1}there{\\i0} pal two");
        assert_eq!(cues[2].text, "Plain cue");
        let written = cues_to_ass(&cues);
        assert!(written.contains("{\\b1}there{\\b0}"));
        assert!(written.contains("{\\i1}there{\\i0}"));
        assert!(!written.contains("{\\u"));
        assert!(!written.contains("{\\an"));
        let again = parse_ass(&written).unwrap();
        assert_eq!(again, cues);
        assert_eq!(again[2].text, "Plain cue");
    }

    #[test]
    fn keeps_one_underline_pair_bold_and_italic_lines() {
        let input = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\an8}Hello {\\b1}there{\\b0} {\\i1}pal{\\i0} {\\b1}two{\\b0}\nDialogue: 0,0:00:02.00,0:00:03.00,Default,,0,0,0,,{\\an8}Hello {\\i1}there{\\i0} {\\u1}pal{\\u0} {\\i1}two{\\i0}\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,{\\an8}Hello {\\u1}there{\\u0} {\\s1}pal{\\s0} {\\u1}two{\\u0}\nDialogue: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,Plain cue\n";
        let cues = parse_ass(input).unwrap();
        assert_eq!(cues[0].text, "Hello {\\b1}there{\\b0} pal two");
        assert_eq!(cues[1].text, "Hello {\\i1}there{\\i0} pal two");
        assert_eq!(cues[2].text, "Hello {\\u1}there{\\u0} pal two");
        assert_eq!(cues[3].text, "Plain cue");
        let written = cues_to_ass(&cues);
        assert!(written.contains("{\\b1}there{\\b0}"));
        assert!(written.contains("{\\i1}there{\\i0}"));
        assert!(written.contains("{\\u1}there{\\u0}"));
        assert!(!written.contains("{\\s"));
        assert!(!written.contains("{\\an"));
        let again = parse_ass(&written).unwrap();
        assert_eq!(again, cues);
        assert_eq!(again[3].text, "Plain cue");
    }

    #[test]
    fn keeps_one_strikeout_pair_and_earlier_lines() {
        let input = "[Events]\nFormat: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\nDialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,{\\an8}Hello {\\b1}there{\\b0} {\\i1}pal{\\i0} {\\b1}two{\\b0}\nDialogue: 0,0:00:02.00,0:00:03.00,Default,,0,0,0,,{\\an8}Hello {\\i1}there{\\i0} {\\u1}pal{\\u0} {\\i1}two{\\i0}\nDialogue: 0,0:00:03.00,0:00:04.00,Default,,0,0,0,,{\\an8}Hello {\\u1}there{\\u0} {\\s1}pal{\\s0} {\\u1}two{\\u0}\nDialogue: 0,0:00:04.00,0:00:05.00,Default,,0,0,0,,{\\an8}Hello {\\s1}there{\\s0} {\\a1}pal{\\a2} {\\s1}two{\\s0}\nDialogue: 0,0:00:05.00,0:00:06.00,Default,,0,0,0,,Plain cue\n";
        let cues = parse_ass(input).unwrap();
        assert_eq!(cues[0].text, "Hello {\\b1}there{\\b0} pal two");
        assert_eq!(cues[1].text, "Hello {\\i1}there{\\i0} pal two");
        assert_eq!(cues[2].text, "Hello {\\u1}there{\\u0} pal two");
        assert_eq!(cues[3].text, "Hello {\\s1}there{\\s0} pal two");
        assert_eq!(cues[4].text, "Plain cue");
        let written = cues_to_ass(&cues);
        assert!(written.contains("{\\b1}there{\\b0}"));
        assert!(written.contains("{\\i1}there{\\i0}"));
        assert!(written.contains("{\\u1}there{\\u0}"));
        assert!(written.contains("{\\s1}there{\\s0}"));
        assert!(!written.contains("{\\a"));
        assert!(!written.contains("{\\an"));
        let again = parse_ass(&written).unwrap();
        assert_eq!(again, cues);
        assert_eq!(again[4].text, "Plain cue");
    }
}
