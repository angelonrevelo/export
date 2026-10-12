//! Fixed-pitch 5×7 printed Latin classifier.
//!
//! Thirty-eight glyphs: `A`–`Z`, `0`–`9`, and lowercase `a` and `b`. Each
//! glyph is five columns, then one blank column. The score is Hamming
//! distance to one stored prototype. A glyph farther than half the cell from
//! every template is `?`.
//!
//! This is not Tesseract. It does not do page layout, any lowercase letter
//! other than `a` and `b`, rotation, noise beyond a one-bit edit, or any
//! size other than this font.

/// Rows per glyph.
pub const GLYPH_H: usize = 7;
/// Columns per glyph.
pub const GLYPH_W: usize = 5;
/// Columns from one glyph origin to the next (glyph plus a one-column gap).
pub const PITCH: usize = GLYPH_W + 1;

/// Why a bitmap was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OcrError {
    /// Height was not 7.
    Height,
    /// Width was not `6n - 1` for some `n ≥ 1`.
    Pitch,
    /// `pixels.len()` was not `width * height`.
    Length,
    /// `render_line` was asked for a character outside the fixture alphabet.
    Glyph(char),
}

impl std::fmt::Display for OcrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OcrError::Height => write!(f, "height must be 7"),
            OcrError::Pitch => write!(f, "width must be 6n-1"),
            OcrError::Length => write!(f, "pixel buffer length"),
            OcrError::Glyph(c) => write!(f, "unsupported glyph {c}"),
        }
    }
}

impl std::error::Error for OcrError {}

/// `(char, 7 row masks, low 5 bits, bit 4 = leftmost column)`.
const GLYPHS: [(char, [u8; GLYPH_H]); 38] = [
    (
        'A',
        [
            0b01110, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
    ),
    (
        'B',
        [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10001, 0b10001, 0b11110,
        ],
    ),
    (
        'C',
        [
            0b01110, 0b10001, 0b10000, 0b10000, 0b10000, 0b10001, 0b01110,
        ],
    ),
    (
        'D',
        [
            0b11110, 0b10001, 0b10001, 0b10101, 0b10001, 0b10001, 0b11110,
        ],
    ),
    (
        'E',
        [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b11111,
        ],
    ),
    (
        'F',
        [
            0b11111, 0b10000, 0b10000, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
    ),
    (
        'G',
        [
            0b01110, 0b10001, 0b10000, 0b10111, 0b10001, 0b10001, 0b01111,
        ],
    ),
    (
        'H',
        [
            0b10001, 0b10001, 0b10001, 0b11111, 0b10001, 0b10001, 0b10001,
        ],
    ),
    (
        'I',
        [
            0b01110, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
    ),
    (
        'J',
        [
            0b00111, 0b00010, 0b00010, 0b00010, 0b10010, 0b10010, 0b01100,
        ],
    ),
    (
        'K',
        [
            0b10001, 0b10010, 0b10100, 0b11000, 0b10100, 0b10010, 0b10001,
        ],
    ),
    (
        'L',
        [
            0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b10000, 0b11111,
        ],
    ),
    (
        'M',
        [
            0b10001, 0b11011, 0b10101, 0b10101, 0b10001, 0b10001, 0b10001,
        ],
    ),
    (
        'N',
        [
            0b10001, 0b11001, 0b10101, 0b10011, 0b10001, 0b10001, 0b10001,
        ],
    ),
    (
        'O',
        [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
    ),
    (
        'P',
        [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10000, 0b10000, 0b10000,
        ],
    ),
    (
        'Q',
        [
            0b01110, 0b10001, 0b10001, 0b10001, 0b10101, 0b10010, 0b01101,
        ],
    ),
    (
        'R',
        [
            0b11110, 0b10001, 0b10001, 0b11110, 0b10100, 0b10010, 0b10001,
        ],
    ),
    (
        'S',
        [
            0b01111, 0b10000, 0b10010, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
    ),
    (
        'T',
        [
            0b11111, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
    ),
    (
        'U',
        [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
    ),
    (
        'V',
        [
            0b10001, 0b10001, 0b10001, 0b10001, 0b10001, 0b01010, 0b00100,
        ],
    ),
    (
        'W',
        [
            0b10001, 0b10001, 0b10001, 0b10101, 0b10101, 0b10101, 0b01010,
        ],
    ),
    (
        'X',
        [
            0b10001, 0b10001, 0b01010, 0b00100, 0b01010, 0b10001, 0b10001,
        ],
    ),
    (
        'Y',
        [
            0b10001, 0b10001, 0b01010, 0b00100, 0b00100, 0b00100, 0b00100,
        ],
    ),
    (
        'Z',
        [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b10000, 0b11111,
        ],
    ),
    (
        '0',
        [
            0b01110, 0b10001, 0b10011, 0b10101, 0b11001, 0b10001, 0b01110,
        ],
    ),
    (
        '1',
        [
            0b00100, 0b01100, 0b00100, 0b00100, 0b00100, 0b00100, 0b01110,
        ],
    ),
    (
        '2',
        [
            0b01110, 0b10001, 0b00001, 0b00010, 0b00100, 0b01000, 0b11111,
        ],
    ),
    (
        '3',
        [
            0b11110, 0b00001, 0b00001, 0b01110, 0b00001, 0b00001, 0b11110,
        ],
    ),
    (
        '4',
        [
            0b10001, 0b10001, 0b10001, 0b11111, 0b00001, 0b00001, 0b00001,
        ],
    ),
    (
        '5',
        [
            0b11111, 0b10000, 0b10000, 0b11110, 0b00001, 0b00001, 0b11110,
        ],
    ),
    (
        '6',
        [
            0b01110, 0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b01110,
        ],
    ),
    (
        '7',
        [
            0b11111, 0b00001, 0b00010, 0b00100, 0b01000, 0b01000, 0b01000,
        ],
    ),
    (
        '8',
        [
            0b01110, 0b10001, 0b10001, 0b01110, 0b10001, 0b10001, 0b01110,
        ],
    ),
    (
        '9',
        [
            0b01110, 0b10001, 0b10001, 0b01111, 0b00001, 0b00001, 0b01110,
        ],
    ),
    // Double-story `a` on the x-height: two blank rows, then the bowl.
    // Same Hamming prototype match as the uppercase glyphs.
    (
        'a',
        [
            0b00000, 0b00000, 0b01110, 0b00001, 0b01111, 0b10001, 0b01111,
        ],
    ),
    // Ascender plus a lower bowl. Same Hamming prototype match.
    (
        'b',
        [
            0b10000, 0b10000, 0b11110, 0b10001, 0b10001, 0b10001, 0b01110,
        ],
    ),
];

fn popcount_rows(rows: &[u8; GLYPH_H]) -> i32 {
    rows.iter().map(|r| (r & 0b11111).count_ones() as i32).sum()
}

/// Bit mismatches over the 5×7 cell. Lower is a closer glyph.
fn hamming(a: &[u8; GLYPH_H], b: &[u8; GLYPH_H]) -> i32 {
    let mut dist = 0i32;
    for row in 0..GLYPH_H {
        dist += ((a[row] ^ b[row]) & 0b11111).count_ones() as i32;
    }
    dist
}

/// Nearest template by Hamming distance. Blank ink is `(' ', 0)`.
///
/// A dot product treats a subset glyph as a perfect match for a heavier
/// template (`O` inside `D`). Hamming charges the extra bits, which is the
/// whole classifier: one prototype per character, no connected components.
pub fn classify_glyph(rows: &[u8; GLYPH_H]) -> (char, i32) {
    if popcount_rows(rows) == 0 {
        return (' ', 0);
    }
    let mut best_ch = '?';
    let mut best_dist = i32::MAX;
    for (ch, tmpl) in GLYPHS {
        let dist = hamming(rows, &tmpl);
        if dist < best_dist {
            best_dist = dist;
            best_ch = ch;
        }
    }
    // Half the cell is 17 bits. Farther than that is not this font.
    if best_dist > 17 {
        return ('?', best_dist);
    }
    (best_ch, best_dist)
}

/// Read a single-line bitmap. `pixels` is row-major, ink when `> 127`.
pub fn read_fixed_pitch(pixels: &[u8], width: usize, height: usize) -> Result<String, OcrError> {
    if height != GLYPH_H {
        return Err(OcrError::Height);
    }
    if width < GLYPH_W || (width + 1) % PITCH != 0 {
        return Err(OcrError::Pitch);
    }
    if pixels.len() != width * height {
        return Err(OcrError::Length);
    }
    let n = (width + 1) / PITCH;
    let mut out = String::with_capacity(n);
    for g in 0..n {
        let x0 = g * PITCH;
        let mut rows = [0u8; GLYPH_H];
        for y in 0..GLYPH_H {
            let mut bits = 0u8;
            for x in 0..GLYPH_W {
                if pixels[y * width + x0 + x] > 127 {
                    bits |= 1 << (4 - x);
                }
            }
            rows[y] = bits;
        }
        out.push(classify_glyph(&rows).0);
    }
    Ok(out)
}

/// Rasterize `text` in the fixture font. Width is `6n - 1`, height is 7.
pub fn render_line(text: &str) -> Result<(Vec<u8>, usize, usize), OcrError> {
    if text.is_empty() {
        return Err(OcrError::Glyph('\0'));
    }
    let chars: Vec<char> = text.chars().collect();
    let n = chars.len();
    let width = n * PITCH - 1;
    let mut pixels = vec![0u8; width * GLYPH_H];
    for (g, ch) in chars.into_iter().enumerate() {
        if ch == ' ' {
            continue;
        }
        let tmpl = GLYPHS
            .iter()
            .find(|(c, _)| *c == ch)
            .map(|(_, rows)| *rows)
            .ok_or(OcrError::Glyph(ch))?;
        let x0 = g * PITCH;
        for y in 0..GLYPH_H {
            for x in 0..GLYPH_W {
                if tmpl[y] & (1 << (4 - x)) != 0 {
                    pixels[y * width + x0 + x] = 255;
                }
            }
        }
    }
    Ok((pixels, width, GLYPH_H))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_at_least_three_bits_apart() {
        let mut close = String::new();
        for (i, (ch, rows)) in GLYPHS.iter().enumerate() {
            assert_eq!(classify_glyph(rows).0, *ch);
            for (other, tmpl) in GLYPHS.iter().skip(i + 1) {
                let dist = hamming(rows, tmpl);
                if dist < 3 {
                    close.push_str(&format!("{ch}/{other}={dist} "));
                }
            }
        }
        assert!(close.is_empty(), "pairs closer than 3 bits: {close}");
    }

    #[test]
    fn line_roundtrip_and_one_bit_flip() {
        let text = "HELLO 42";
        let (pixels, w, h) = render_line(text).unwrap();
        assert_eq!(read_fixed_pitch(&pixels, w, h).unwrap(), text);

        let alphabet = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
        let (pixels, w, h) = render_line(alphabet).unwrap();
        assert_eq!(read_fixed_pitch(&pixels, w, h).unwrap(), alphabet);

        let (mut pixels, w, h) = render_line("H").unwrap();
        // Flip one ink pixel off (row 0 of H is 10001, leftmost is ink).
        pixels[0] = 0;
        assert_eq!(read_fixed_pitch(&pixels, w, h).unwrap(), "H");
    }

    #[test]
    fn lowercase_a_reads_back_and_other_lowercase_is_refused() {
        let alone = "a";
        let (pixels, w, h) = render_line(alone).unwrap();
        assert_eq!(read_fixed_pitch(&pixels, w, h).unwrap(), "a");
        assert_eq!(alone.chars().count(), 1);

        // Uppercase, digits, and `a` on one line. 36 existing glyphs plus `a`.
        let mixed = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789a";
        assert_eq!(mixed.chars().count(), 37);
        let (pixels, w, h) = render_line(mixed).unwrap();
        assert_eq!(read_fixed_pitch(&pixels, w, h).unwrap(), mixed);

        assert_eq!(render_line("Hi"), Err(OcrError::Glyph('i')));
        assert_eq!(render_line("c"), Err(OcrError::Glyph('c')));
    }

    #[test]
    fn lowercase_b_reads_back() {
        let alone = "b";
        let (pixels, w, h) = render_line(alone).unwrap();
        assert_eq!(read_fixed_pitch(&pixels, w, h).unwrap(), "b");
        assert_eq!(alone.chars().count(), 1);

        let line = "ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789ab";
        assert_eq!(line.chars().count(), 38);
        let (pixels, w, h) = render_line(line).unwrap();
        assert_eq!(read_fixed_pitch(&pixels, w, h).unwrap(), line);
    }
}
