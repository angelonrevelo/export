//! Casefold for lexicon lookup.
//!
//! ASCII runs are lowercased in place, the same shape as index-text's fold: one
//! byte scan, no allocation per character. Accents stay. Spanish contrasts
//! `esta` / `está` and `el` / `él`, so this is not the search fold that strips
//! diacritics.

pub(crate) fn fold_into(s: &str, out: &mut String) {
    out.clear();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] < 0x80 {
            let at = out.len();
            let start = i;
            while i < b.len() && b[i] < 0x80 {
                i += 1;
            }
            out.push_str(&s[start..i]);
            out[at..].make_ascii_lowercase();
            continue;
        }
        let ch = s[i..].chars().next().unwrap();
        i += ch.len_utf8();
        for low in ch.to_lowercase() {
            out.push(low);
        }
    }
}
