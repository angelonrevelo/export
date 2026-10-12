#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cue {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

impl Cue {
    pub fn shift(&mut self, delta_ms: i64) {
        self.start_ms = add_ms(self.start_ms, delta_ms);
        self.end_ms = add_ms(self.end_ms, delta_ms);
    }
}

fn add_ms(ms: u64, delta: i64) -> u64 {
    if delta >= 0 {
        ms.saturating_add(delta as u64)
    } else {
        ms.saturating_sub(delta.unsigned_abs())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    Truncated,
    BadTime,
    BadCue,
}

pub fn parse_srt(input: &str) -> Result<Vec<Cue>, Error> {
    parse_blocks(input, false)
}

pub fn parse_vtt(input: &str) -> Result<Vec<Cue>, Error> {
    let rest = input.trim_start();
    let rest = rest
        .strip_prefix("WEBVTT")
        .ok_or(Error::BadCue)?;
    parse_blocks(rest, true)
}

fn parse_blocks(input: &str, _vtt: bool) -> Result<Vec<Cue>, Error> {
    let mut cues = Vec::new();
    for block in input.split("\n\n") {
        let block = block.trim();
        if block.is_empty() || block.starts_with("NOTE") || block.starts_with("STYLE") {
            continue;
        }
        let mut lines = block.lines().peekable();
        let first = lines.next().unwrap_or("");
        let time_line = if first.contains("-->") {
            first
        } else {
            lines.next().unwrap_or("")
        };
        if !time_line.contains("-->") {
            if block.chars().all(|c| c.is_ascii_digit() || c == '\n') {
                continue;
            }
            return Err(Error::Truncated);
        }
        let (start, end) = parse_range(time_line)?;
        let text: Vec<&str> = lines.collect();
        if text.is_empty() {
            return Err(Error::Truncated);
        }
        cues.push(Cue {
            start_ms: start,
            end_ms: end,
            text: text.join("\n"),
        });
    }
    Ok(cues)
}

fn parse_range(line: &str) -> Result<(u64, u64), Error> {
    let line = line.trim();
    let (left, right) = line.split_once("-->").ok_or(Error::BadTime)?;
    Ok((parse_stamp(left)?, parse_stamp(right)?))
}

fn parse_stamp(raw: &str) -> Result<u64, Error> {
    let raw = raw.split_whitespace().next().unwrap_or("");
    if raw.is_empty() {
        return Err(Error::Truncated);
    }
    let split_at = raw.find([',', '.']).ok_or(Error::BadTime)?;
    let hms = &raw[..split_at];
    let frac = &raw[split_at + 1..];
    if frac.len() < 3 || !frac.as_bytes()[..3].iter().all(|c| c.is_ascii_digit()) {
        return Err(Error::Truncated);
    }
    let ms: u64 = frac[..3].parse().map_err(|_| Error::BadTime)?;
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, s) = match parts.as_slice() {
        [m, s] => (0u64, parse_num(m)?, parse_num(s)?),
        [h, m, s] => (parse_num(h)?, parse_num(m)?, parse_num(s)?),
        _ => return Err(Error::BadTime),
    };
    Ok(((h * 60 + m) * 60 + s) * 1000 + ms)
}

fn parse_num(s: &str) -> Result<u64, Error> {
    if s.is_empty() || !s.bytes().all(|c| c.is_ascii_digit()) {
        return Err(Error::BadTime);
    }
    s.parse().map_err(|_| Error::BadTime)
}

pub fn cues_to_srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{}\n", i + 1));
        out.push_str(&stamp(cue.start_ms, ','));
        out.push_str(" --> ");
        out.push_str(&stamp(cue.end_ms, ','));
        out.push('\n');
        out.push_str(&cue.text);
        out.push('\n');
    }
    out
}

pub fn cues_to_vtt(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues {
        out.push_str(&stamp(cue.start_ms, '.'));
        out.push_str(" --> ");
        out.push_str(&stamp(cue.end_ms, '.'));
        out.push('\n');
        out.push_str(&cue.text);
        out.push_str("\n\n");
    }
    out
}

fn stamp(ms: u64, sep: char) -> String {
    let h = ms / 3_600_000;
    let m = (ms / 60_000) % 60;
    let s = (ms / 1000) % 60;
    let milli = ms % 1000;
    format!("{h:02}:{m:02}:{s:02}{sep}{milli:03}")
}
