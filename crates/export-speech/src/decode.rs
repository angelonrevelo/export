//! Greedy CTC collapse and the transcript writers Export already emits.
//!
//! `txt` / `srt` / `vtt` / `notes` / `words` follow `app/jobs.py`: notes break
//! on a gap above 1.5 s, word times are seconds to 3 decimals, SRT uses a
//! comma, VTT uses a dot. This decoder does not search a beam and it does not
//! know a language.

use crate::mel::{HOP, PCM_RATE};

/// One timed cue.
#[derive(Clone, Debug, PartialEq)]
pub struct Segment {
    pub start: f32,
    pub end: f32,
    pub text: String,
}

/// One aligned token. `confidence` is `None` when the caller has no probability.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: f32,
    pub end: f32,
    pub confidence: Option<f32>,
}

/// Export's transcript bundle. `language` is `None` when nothing detected it.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    pub language: Option<String>,
    pub segments: Vec<Segment>,
    pub words: Vec<Word>,
}

impl Transcript {
    pub fn plain(&self) -> String {
        if let Some(seg) = self.segments.first() {
            return seg.text.clone();
        }
        let mut s = String::new();
        for w in &self.words {
            s.push_str(&w.text);
        }
        s
    }
}

/// Collapse a class-id sequence. Repeats drop. `blank` drops. Other ids stay.
pub fn ctc_greedy(classes: &[usize], blank: usize) -> Vec<usize> {
    let mut out = Vec::new();
    let mut prev = None;
    for &class in classes {
        if Some(class) == prev {
            continue;
        }
        prev = Some(class);
        if class != blank {
            out.push(class);
        }
    }
    out
}

/// Argmax each row of row-major `logits` (`frames` × `classes`), then [`ctc_greedy`].
pub fn ctc_greedy_logits(
    logits: &[i32],
    frames: usize,
    classes: usize,
    blank: usize,
) -> Result<Vec<usize>, crate::runtime::RuntimeError> {
    if classes == 0 || frames == 0 {
        return Err(crate::runtime::RuntimeError::Empty);
    }
    if logits.len() != frames * classes {
        return Err(crate::runtime::RuntimeError::Shape);
    }
    let mut ids = Vec::with_capacity(frames);
    for t in 0..frames {
        let row = &logits[t * classes..(t + 1) * classes];
        let mut best_i = 0usize;
        let mut best_v = row[0];
        for (i, &v) in row.iter().enumerate().skip(1) {
            if v > best_v {
                best_v = v;
                best_i = i;
            }
        }
        ids.push(best_i);
    }
    Ok(ctc_greedy(&ids, blank))
}

/// Map non-blank class ids through `vocab`. `vocab[blank]` is never read
/// because [`ctc_greedy`] already removed blanks. An out-of-range id becomes `?`.
pub fn tokens_to_text(tokens: &[usize], vocab: &[char]) -> String {
    let mut s = String::with_capacity(tokens.len());
    for &id in tokens {
        s.push(vocab.get(id).copied().unwrap_or('?'));
    }
    s
}

/// Online greedy CTC. State is the open token plus the finished list, not the
/// frame log-mels. One hop is `HOP / PCM_RATE` seconds.
pub struct GreedyCtc {
    blank: usize,
    vocab: &'static [char],
    prev: Option<usize>,
    open: Option<Open>,
    words: Vec<Word>,
}

struct Open {
    class_id: usize,
    start: f32,
    end: f32,
    conf_sum: i64,
    conf_n: u32,
}

impl GreedyCtc {
    pub fn new(blank: usize, vocab: &'static [char]) -> Self {
        Self {
            blank,
            vocab,
            prev: None,
            open: None,
            words: Vec::new(),
        }
    }

    pub fn push(&mut self, class_id: usize, t_sec: f32, conf_ppm: i32) {
        let hop = HOP as f32 / PCM_RATE as f32;
        if self.prev == Some(class_id) {
            if let Some(open) = self.open.as_mut() {
                open.end = t_sec + hop;
                open.conf_sum += i64::from(conf_ppm);
                open.conf_n = open.conf_n.saturating_add(1);
            }
            return;
        }
        self.close();
        self.prev = Some(class_id);
        if class_id != self.blank {
            self.open = Some(Open {
                class_id,
                start: t_sec,
                end: t_sec + hop,
                conf_sum: i64::from(conf_ppm),
                conf_n: 1,
            });
        }
    }

    fn close(&mut self) {
        if let Some(open) = self.open.take() {
            let ch = self.vocab.get(open.class_id).copied().unwrap_or('?');
            let conf = if open.conf_n == 0 {
                None
            } else {
                Some(open.conf_sum as f32 / open.conf_n as f32 / 1_000_000.0)
            };
            self.words.push(Word {
                text: ch.to_string(),
                start: open.start,
                end: open.end.max(open.start),
                confidence: conf,
            });
        }
    }

    pub fn finish(mut self) -> Transcript {
        self.close();
        let text: String = self.words.iter().map(|w| w.text.as_str()).collect();
        let segments = if self.words.is_empty() {
            Vec::new()
        } else {
            let start = self.words[0].start;
            let end = self.words.last().map(|w| w.end).unwrap_or(start);
            vec![Segment { start, end, text }]
        };
        Transcript {
            language: None,
            segments,
            words: self.words,
        }
    }
}

fn format_ts(seconds: f32, comma: bool) -> String {
    let s = if seconds.is_finite() {
        seconds.max(0.0)
    } else {
        0.0
    };
    let h = (s / 3600.0) as u32;
    let m = ((s % 3600.0) / 60.0) as u32;
    let sec = s % 60.0;
    let mut out = format!("{h:02}:{m:02}:{sec:06.3}");
    if comma {
        out = out.replace('.', ",");
    }
    out
}

pub fn write_txt(transcript: &Transcript) -> String {
    let mut lines = Vec::new();
    if transcript.segments.is_empty() {
        lines.push(transcript.plain());
    } else {
        for seg in &transcript.segments {
            lines.push(seg.text.trim().to_string());
        }
    }
    let mut body = lines.join("\n");
    body.push('\n');
    body
}

pub fn write_srt(transcript: &Transcript) -> String {
    let mut lines: Vec<String> = Vec::new();
    for (i, seg) in transcript.segments.iter().enumerate() {
        lines.push((i + 1).to_string());
        lines.push(format!(
            "{} --> {}",
            format_ts(seg.start, true),
            format_ts(seg.end, true)
        ));
        lines.push(seg.text.trim().to_string());
        lines.push(String::new());
    }
    lines.join("\n")
}

pub fn write_vtt(transcript: &Transcript) -> String {
    let mut lines = vec!["WEBVTT".to_string(), String::new()];
    for seg in &transcript.segments {
        lines.push(format!(
            "{} --> {}",
            format_ts(seg.start, false),
            format_ts(seg.end, false)
        ));
        lines.push(seg.text.trim().to_string());
        lines.push(String::new());
    }
    lines.join("\n")
}

fn language_name(code: &str) -> &str {
    match code {
        "en" => "English",
        "tl" | "fil" => "Filipino",
        "ceb" => "Cebuano",
        "ilo" => "Ilocano",
        "hil" => "Hiligaynon",
        "war" => "Waray",
        "bcl" => "Bikol",
        other => other,
    }
}

/// Note-style markdown. Paragraphs split when the next cue starts more than
/// 1.5 s after the previous cue ended.
pub fn write_notes(stem: &str, transcript: &Transcript) -> String {
    let title = stem.replace(['_', '-'], " ").trim().to_string();
    let mut paragraphs: Vec<Vec<String>> = vec![Vec::new()];
    let mut last_end = 0.0f32;
    for seg in &transcript.segments {
        if seg.start - last_end > 1.5 && paragraphs.last().is_some_and(|p| !p.is_empty()) {
            paragraphs.push(Vec::new());
        }
        paragraphs
            .last_mut()
            .expect("paragraph buffer")
            .push(seg.text.trim().to_string());
        last_end = seg.end;
    }
    let body = paragraphs
        .iter()
        .filter(|p| !p.is_empty())
        .map(|p| p.join(" "))
        .collect::<Vec<_>>()
        .join("\n\n");
    let lang_line = match transcript.language.as_deref() {
        Some(code) => format!("Detected language: {} (`{code}`)\n\n", language_name(code)),
        None => String::new(),
    };
    format!("# {title}\n\n{lang_line}## Transcript\n\n{body}\n")
}

fn json_escape(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out
}

fn fmt3(v: f32) -> String {
    format!("{:.3}", v.max(0.0))
}

/// `{ "language": …, "word": [ { "text", "start", "end", "confidence" } ] }`
/// with Python's default `json.dumps` separators.
pub fn write_words(transcript: &Transcript) -> String {
    let lang = match transcript.language.as_deref() {
        Some(code) => format!("\"{}\"", json_escape(code)),
        None => "null".to_string(),
    };
    let mut words = String::from("[");
    for (i, w) in transcript.words.iter().enumerate() {
        if i > 0 {
            words.push_str(", ");
        }
        let conf = match w.confidence {
            Some(c) => fmt3(c),
            None => "null".to_string(),
        };
        words.push_str(&format!(
            "{{\"text\": \"{}\", \"start\": {}, \"end\": {}, \"confidence\": {}}}",
            json_escape(&w.text),
            fmt3(w.start),
            fmt3(w.end.max(w.start)),
            conf
        ));
    }
    words.push(']');
    format!("{{\"language\": {lang}, \"word\": {words}}}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn greedy_collapses_repeats_and_blanks() {
        // blank=0, letters 1 and 2. The repeat of 1 across a blank is a new token.
        let ids = [1usize, 1, 0, 2, 2, 2, 0, 1];
        assert_eq!(ctc_greedy(&ids, 0), vec![1, 2, 1]);
    }

    #[test]
    fn greedy_logits_hand() {
        // classes: blank, a, b. Four frames spell a, a, blank, b → "ab".
        let logits = [
            0, 5, 1, // a
            0, 4, 1, // a (repeat)
            9, 0, 0, // blank
            1, 0, 8, // b
        ];
        let tokens = ctc_greedy_logits(&logits, 4, 3, 0).unwrap();
        assert_eq!(tokens, vec![1, 2]);
        assert_eq!(tokens_to_text(&tokens, &[' ', 'a', 'b']), "ab");
    }

    #[test]
    fn tie_on_blank_emits_nothing() {
        let logits = [1, 1, 1, 1];
        let tokens = ctc_greedy_logits(&logits, 2, 2, 0).unwrap();
        assert!(tokens.is_empty());
    }

    #[test]
    fn online_matches_batch_collapse() {
        let classes = [1usize, 1, 0, 2, 2, 0, 1];
        let mut dec = GreedyCtc::new(0, &[' ', 'H', 'I']);
        for (i, &c) in classes.iter().enumerate() {
            dec.push(c, i as f32 * 0.01, 500_000);
        }
        let t = dec.finish();
        assert_eq!(t.plain(), "HIH");
        assert_eq!(t.words.len(), 3);
        assert!(t.words[0].confidence.unwrap() > 0.49);
        assert!(t.words[0].end > t.words[0].start);
    }

    #[test]
    fn writers_match_export_shape() {
        let transcript = Transcript {
            language: Some("en".to_string()),
            segments: vec![
                Segment {
                    start: 0.12,
                    end: 0.58,
                    text: "Maayong".to_string(),
                },
                Segment {
                    start: 2.20,
                    end: 2.80,
                    text: "buntag".to_string(),
                },
            ],
            words: vec![Word {
                text: "Maayong".to_string(),
                start: 0.12,
                end: 0.58,
                confidence: Some(0.9344),
            }],
        };
        assert_eq!(write_txt(&transcript), "Maayong\nbuntag\n");
        assert_eq!(
            write_srt(&transcript),
            "1\n00:00:00,120 --> 00:00:00,580\nMaayong\n\n2\n00:00:02,200 --> 00:00:02,800\nbuntag\n"
        );
        assert!(write_vtt(&transcript).starts_with("WEBVTT\n\n00:00:00.120 --> 00:00:00.580\n"));
        let notes = write_notes("input_clip", &transcript);
        assert!(notes
            .starts_with("# input clip\n\nDetected language: English (`en`)\n\n## Transcript\n\n"));
        assert!(notes.contains("Maayong\n\nbuntag\n"));
        assert_eq!(
            write_words(&transcript),
            "{\"language\": \"en\", \"word\": [{\"text\": \"Maayong\", \"start\": 0.120, \"end\": 0.580, \"confidence\": 0.934}]}"
        );
    }
}
