//! Byte scanner. Words are Unicode alphanumeric runs; everything else is punctuation.
//! Whitespace is dropped. No regex crate.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Raw<'a> {
    Word(&'a str),
    Punct(&'a str),
}

pub(crate) fn tokenize(s: &str) -> Vec<Raw<'_>> {
    let b = s.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < b.len() {
        if b[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if is_word_byte(b[i]) {
            let start = i;
            i = scan_word(s, i);
            out.push(Raw::Word(&s[start..i]));
            continue;
        }
        if b[i] >= 0x80 {
            let (ch, n) = next_char(s, i);
            if ch.is_alphanumeric() {
                let start = i;
                i = scan_word(s, i);
                out.push(Raw::Word(&s[start..i]));
            } else {
                out.push(Raw::Punct(&s[i..n]));
                i = n;
            }
            continue;
        }
        out.push(Raw::Punct(&s[i..i + 1]));
        i += 1;
    }
    out
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'\''
}

fn scan_word(s: &str, mut i: usize) -> usize {
    let b = s.as_bytes();
    while i < b.len() {
        if is_word_byte(b[i]) {
            i += 1;
            continue;
        }
        if b[i] >= 0x80 {
            let (ch, n) = next_char(s, i);
            if ch.is_alphanumeric() || ch == '\'' {
                i = n;
                continue;
            }
        }
        break;
    }
    i
}

fn next_char(s: &str, i: usize) -> (char, usize) {
    let ch = s[i..].chars().next().unwrap();
    (ch, i + ch.len_utf8())
}
