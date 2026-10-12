//! Compact English ↔ Spanish transfer.
//!
//! This is not NLLB. There is no downloaded model and no 1.2 GB resident set.
//! The hot path is a byte scanner, a binary search over a closed lexicon, and a
//! shallow clause transfer — all in this crate, with zero dependencies.
//!
//! [`PAIR_LIMIT`] is the language coverage. Anything outside en ↔ es fails.

#![forbid(unsafe_code)]

mod fold;
mod lexicon;
mod scan;
mod transfer;

use std::fmt;

/// What this build will and will not translate.
pub const PAIR_LIMIT: &str = "\
Only en → es and es → en are translated. en → en and es → es return the input \
unchanged (same language, not a translation). Every other code — including tl, \
fil, zh, ja, fr, pt, and de — returns an error. Coverage is a closed lexicon and \
one clause shape (subject, copula or one verb, one object, estar+gerund, \
tú-imperative, trailing adjuncts). It is not NLLB-200 and it does not echo an \
unsupported pair as if it had been translated.";

/// Pairs this crate actually translates. Same-language passthrough is not listed.
pub fn supported_pairs() -> &'static [(&'static str, &'static str)] {
    &[("en", "es"), ("es", "en")]
}

pub use lexicon::lexicon_static_bytes;

#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    /// `source` → `target` is outside [`PAIR_LIMIT`].
    Unsupported { source: String, target: String },
    /// The pair is supported, the input has letters, and not one token was in the lexicon.
    NothingTranslated { source: String, target: String },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported { source, target } => write!(
                f,
                "unsupported language pair '{source}' → '{target}'; this crate translates en ↔ es only"
            ),
            Error::NothingTranslated { source, target } => write!(
                f,
                "nothing translated for {source} → {target}; input is outside the lexicon and was not echoed"
            ),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lang {
    En,
    Es,
}

/// Translate `text` from `source` into `target`.
///
/// Language codes: `en` / `eng` / `en-us`, and `es` / `spa` / `es-mx`. Same language
/// returns the input unchanged. Any other pair is [`Error::Unsupported`].
pub fn translate(text: &str, source: &str, target: &str) -> Result<String, Error> {
    let src = canon(source);
    let tgt = canon(target);
    match (src, tgt) {
        (Some(a), Some(b)) if a == b => Ok(text.to_string()),
        (Some(Lang::En), Some(Lang::Es)) => convert(text, source, target, transfer::en_to_es),
        (Some(Lang::Es), Some(Lang::En)) => convert(text, source, target, transfer::es_to_en),
        _ => Err(Error::Unsupported {
            source: norm_code(source),
            target: norm_code(target),
        }),
    }
}

/// Translate each string. An unsupported pair fails the whole batch.
pub fn translate_batch(texts: &[&str], source: &str, target: &str) -> Result<Vec<String>, Error> {
    let mut out = Vec::with_capacity(texts.len());
    let mut i = 0;
    while i < texts.len() {
        out.push(translate(texts[i], source, target)?);
        i += 1;
    }
    Ok(out)
}

fn convert(
    text: &str,
    source: &str,
    target: &str,
    one: fn(&str) -> Result<String, &'static str>,
) -> Result<String, Error> {
    if text.is_empty() || !has_letter(text) {
        return Ok(text.to_string());
    }
    let pieces = split_pieces(text);
    let mut out = String::with_capacity(text.len() + 8);
    let mut any = false;
    let mut i = 0;
    while i < pieces.len() {
        let piece = pieces[i];
        if !has_letter(piece) {
            out.push_str(piece);
            i += 1;
            continue;
        }
        let lead = piece.len() - piece.trim_start().len();
        let trail = piece.len() - piece.trim_end().len();
        let core = piece.trim();
        out.push_str(&piece[..lead]);
        match one(core) {
            Ok(body) => {
                any = true;
                out.push_str(&finish(&body, core));
            }
            Err(_) => {
                return Err(Error::NothingTranslated {
                    source: norm_code(source),
                    target: norm_code(target),
                });
            }
        }
        if trail > 0 {
            out.push_str(&piece[piece.len() - trail..]);
        }
        i += 1;
    }
    if !any && has_letter(text) {
        return Err(Error::NothingTranslated {
            source: norm_code(source),
            target: norm_code(target),
        });
    }
    Ok(out)
}

fn finish(body: &str, src: &str) -> String {
    let mut s = body.trim().to_string();
    cap_first(&mut s);
    let t = src.trim_end();
    if t.ends_with('?') && !s.ends_with('?') {
        s.push('?');
    } else if t.ends_with('!') && !s.ends_with('!') {
        s.push('!');
    } else if t.ends_with('.') && !s.ends_with('.') && !s.ends_with('?') && !s.ends_with('!') {
        s.push('.');
    }
    s
}

fn cap_first(s: &mut String) {
    let mut seen = false;
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        if !seen && ch.is_alphabetic() {
            for up in ch.to_uppercase() {
                out.push(up);
            }
            seen = true;
        } else {
            out.push(ch);
        }
    }
    *s = out;
}

fn has_letter(s: &str) -> bool {
    for ch in s.chars() {
        if ch.is_alphabetic() {
            return true;
        }
    }
    false
}

/// Split on `.` `!` `?` when the dot is not inside a number. Whitespace between
/// sentences stays as its own piece so it is not fed to the lexicon.
fn split_pieces(text: &str) -> Vec<&str> {
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut start = 0;
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c == b'.' || c == b'!' || c == b'?' {
            let prev_digit = i > 0 && b[i - 1].is_ascii_digit();
            let next = i + 1;
            let next_digit = next < b.len() && b[next].is_ascii_digit();
            if c == b'.' && prev_digit && next_digit {
                i += 1;
                continue;
            }
            i += 1;
            out.push(&text[start..i]);
            start = i;
            let ws = i;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            if i > ws {
                out.push(&text[ws..i]);
                start = i;
            }
            continue;
        }
        if c >= 0x80 {
            i += 1;
            while i < b.len() && b[i] & 0xC0 == 0x80 {
                i += 1;
            }
        } else {
            i += 1;
        }
    }
    if start < b.len() {
        out.push(&text[start..]);
    }
    out
}

fn canon(code: &str) -> Option<Lang> {
    match norm_code(code).as_str() {
        "en" | "eng" | "en-us" | "en-gb" => Some(Lang::En),
        "es" | "spa" | "es-es" | "es-mx" | "es-419" => Some(Lang::Es),
        _ => None,
    }
}

fn norm_code(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    for ch in code.chars() {
        if ch == '_' {
            out.push('-');
        } else {
            for low in ch.to_lowercase() {
                out.push(low);
            }
        }
    }
    let t = out.trim();
    if t.len() == out.len() {
        out
    } else {
        t.to_string()
    }
}
