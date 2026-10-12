//! ASS/SSA dialogue cues. Override tags are stripped so the shared cue is plain text.
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
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '{' && chars.peek() == Some(&'\\') {
            for next in chars.by_ref() {
                if next == '}' {
                    break;
                }
            }
            continue;
        }
        if c == '\\' && matches!(chars.peek(), Some('N' | 'n' | 'h')) {
            let kind = chars.next().unwrap();
            out.push(if kind == 'h' { ' ' } else { '\n' });
            continue;
        }
        out.push(c);
    }
    out
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
}
