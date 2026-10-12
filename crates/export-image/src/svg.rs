//! One SVG: a 16×16 or 32×32 black square on white, stored as a single `rect`.
//!
//! A path, circle, ellipse, line, polygon, polyline, rounded corner, or text
//! node is rejected. Cairo, vtracer, and resvg are not used. This path does
//! not start a thread.

use crate::error::{Error, INVALID, TRUNCATED, UNSUPPORTED};
use crate::raster::Raster;

const SIDE: u32 = 16;

/// Encode one supported picture: a solid black square on an otherwise white
/// 16×16 or 32×32 RGB canvas. The file is one `<rect>`. Anything else is
/// [`UNSUPPORTED`].
pub fn encode_svg(raster: &Raster) -> Result<Vec<u8>, Error> {
    if raster.width == 32 {
        return encode_svg_32(raster);
    }
    let (x, y, side) = black_square(raster).ok_or(UNSUPPORTED)?;
    let mut out = Vec::with_capacity(160);
    out.extend_from_slice(
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"><rect x=\"",
    );
    push_u32(&mut out, x);
    out.extend_from_slice(b"\" y=\"");
    push_u32(&mut out, y);
    out.extend_from_slice(b"\" width=\"");
    push_u32(&mut out, side);
    out.extend_from_slice(b"\" height=\"");
    push_u32(&mut out, side);
    out.extend_from_slice(b"\" fill=\"#000000\"/></svg>");
    Ok(out)
}

fn encode_svg_32(raster: &Raster) -> Result<Vec<u8>, Error> {
    let (x, y, side) = black_square_32(raster).ok_or(UNSUPPORTED)?;
    let mut out = Vec::with_capacity(160);
    out.extend_from_slice(
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"32\"><rect x=\"",
    );
    push_u32(&mut out, x);
    out.extend_from_slice(b"\" y=\"");
    push_u32(&mut out, y);
    out.extend_from_slice(b"\" width=\"");
    push_u32(&mut out, side);
    out.extend_from_slice(b"\" height=\"");
    push_u32(&mut out, side);
    out.extend_from_slice(b"\" fill=\"#000000\"/></svg>");
    Ok(out)
}

/// Paint the SVG written by [`encode_svg`]. The canvas is white. The one black
/// `rect` is copied on top. Another element, a curve, or text is [`UNSUPPORTED`].
pub fn rasterize_svg(bytes: &[u8]) -> Result<Raster, Error> {
    if bytes.is_empty() {
        return Err(INVALID);
    }
    if bytes.iter().any(|b| !b.is_ascii()) {
        return Err(INVALID);
    }
    let mut p = Parser { s: bytes, i: 0 };
    p.skip_ws();
    if p.eof() {
        return Err(INVALID);
    }
    if !p.eat(b"<svg") {
        return Err(INVALID);
    }
    let open = p.finish_open()?;
    let canvas = match (open.width, open.height) {
        (Some(16), Some(16)) => 16,
        (Some(32), Some(32)) => 32,
        _ => return Err(UNSUPPORTED),
    };
    if open.self_close || !open.xmlns {
        return Err(UNSUPPORTED);
    }
    p.skip_ws();
    if p.eof() {
        return Err(TRUNCATED);
    }
    if !p.eat(b"<rect") {
        return Err(UNSUPPORTED);
    }
    let rect = p.finish_rect()?;
    if !rect.self_close {
        p.skip_ws();
        if p.eof() {
            return Err(TRUNCATED);
        }
        if !p.eat(b"</rect>") {
            return Err(UNSUPPORTED);
        }
    }
    p.skip_ws();
    if p.eof() {
        return Err(TRUNCATED);
    }
    if !p.eat(b"</svg>") {
        return Err(UNSUPPORTED);
    }
    p.skip_ws();
    if !p.eof() {
        return Err(UNSUPPORTED);
    }
    paint(canvas, rect.x, rect.y, rect.width, rect.height)
}

fn black_square(raster: &Raster) -> Option<(u32, u32, u32)> {
    if raster.width != SIDE
        || raster.height != SIDE
        || raster.channels != 3
        || raster.x != 0
        || raster.y != 0
    {
        return None;
    }
    let mut min_x = SIDE;
    let mut min_y = SIDE;
    let mut max_x = 0u32;
    let mut max_y = 0u32;
    let mut black = 0u32;
    for y in 0..SIDE {
        for x in 0..SIDE {
            let i = ((y * SIDE + x) * 3) as usize;
            let px = &raster.pixels[i..i + 3];
            if px == [0, 0, 0] {
                black += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            } else if px != [255, 255, 255] {
                return None;
            }
        }
    }
    if black == 0 || black == SIDE * SIDE {
        return None;
    }
    let w = max_x - min_x + 1;
    let h = max_y - min_y + 1;
    if w != h || black != w * h {
        return None;
    }
    Some((min_x, min_y, w))
}

fn black_square_32(raster: &Raster) -> Option<(u32, u32, u32)> {
    const N: usize = 32;
    const STRIDE: usize = N * 3;
    if raster.width != N as u32
        || raster.height != N as u32
        || raster.channels != 3
        || raster.x != 0
        || raster.y != 0
    {
        return None;
    }
    let px = raster.pixels.as_slice();
    let white = [255u8; STRIDE];
    let mut top = 0usize;
    while top < N && row32(px, top) == white.as_slice() {
        top += 1;
    }
    if top == N {
        return None;
    }
    let first = row32(px, top);
    let mut left = 0usize;
    while left < N {
        let i = left * 3;
        let r = first[i];
        let g = first[i + 1];
        let b = first[i + 2];
        if r | g | b == 0 {
            break;
        }
        if r & g & b != 255 {
            return None;
        }
        left += 1;
    }
    if left == N {
        return None;
    }
    let mut right = left;
    while right < N && first[right * 3] | first[right * 3 + 1] | first[right * 3 + 2] == 0 {
        right += 1;
    }
    let side = right - left;
    if side == N || top + side > N {
        return None;
    }
    let mut expect = white;
    for col in left..right {
        let i = col * 3;
        expect[i] = 0;
        expect[i + 1] = 0;
        expect[i + 2] = 0;
    }
    if first != expect.as_slice() {
        return None;
    }
    let bottom = top + side;
    for y in (top + 1)..bottom {
        if row32(px, y) != expect.as_slice() {
            return None;
        }
    }
    for y in bottom..N {
        if row32(px, y) != white.as_slice() {
            return None;
        }
    }
    Some((left as u32, top as u32, side as u32))
}

fn row32(px: &[u8], y: usize) -> &[u8] {
    let start = y * 96;
    &px[start..start + 96]
}

fn paint(canvas: u32, x: u32, y: u32, w: u32, h: u32) -> Result<Raster, Error> {
    let Some(right) = x.checked_add(w) else {
        return Err(UNSUPPORTED);
    };
    let Some(bottom) = y.checked_add(h) else {
        return Err(UNSUPPORTED);
    };
    if w == 0 || h == 0 || right > canvas || bottom > canvas {
        return Err(UNSUPPORTED);
    }
    let mut pixels = vec![255u8; (canvas * canvas * 3) as usize];
    for row in y..y + h {
        for col in x..x + w {
            let i = ((row * canvas + col) * 3) as usize;
            pixels[i] = 0;
            pixels[i + 1] = 0;
            pixels[i + 2] = 0;
        }
    }
    Raster::new(canvas, canvas, 3, pixels)
}

fn push_u32(out: &mut Vec<u8>, n: u32) {
    let mut buf = [0u8; 10];
    let mut v = n;
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[i..]);
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

type Attr<'a> = (&'a [u8], &'a [u8]);

struct Open {
    self_close: bool,
    width: Option<u32>,
    height: Option<u32>,
    xmlns: bool,
}

struct RectTag {
    self_close: bool,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

impl<'a> Parser<'a> {
    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }

    fn skip_ws(&mut self) {
        while self.i < self.s.len() && self.s[self.i].is_ascii_whitespace() {
            self.i += 1;
        }
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn name(&mut self) -> Result<&'a [u8], Error> {
        let start = self.i;
        if self.eof() || !self.s[self.i].is_ascii_alphabetic() {
            return Err(if self.eof() { TRUNCATED } else { INVALID });
        }
        self.i += 1;
        while self.i < self.s.len()
            && (self.s[self.i].is_ascii_alphanumeric() || self.s[self.i] == b':')
        {
            self.i += 1;
        }
        Ok(&self.s[start..self.i])
    }

    fn quoted(&mut self) -> Result<&'a [u8], Error> {
        if self.eof() {
            return Err(TRUNCATED);
        }
        let q = self.s[self.i];
        if q != b'"' && q != b'\'' {
            return Err(INVALID);
        }
        self.i += 1;
        let start = self.i;
        while self.i < self.s.len() && self.s[self.i] != q {
            self.i += 1;
        }
        if self.eof() {
            return Err(TRUNCATED);
        }
        let value = &self.s[start..self.i];
        self.i += 1;
        Ok(value)
    }

    fn attr(&mut self) -> Result<Option<Attr<'a>>, Error> {
        self.skip_ws();
        if self.eof() {
            return Err(TRUNCATED);
        }
        if self.s[self.i] == b'/' || self.s[self.i] == b'>' {
            return Ok(None);
        }
        let name = self.name()?;
        self.skip_ws();
        if !self.eat(b"=") {
            return Err(INVALID);
        }
        self.skip_ws();
        let value = self.quoted()?;
        Ok(Some((name, value)))
    }

    fn end_tag(&mut self) -> Result<bool, Error> {
        self.skip_ws();
        if self.eat(b"/>") {
            return Ok(true);
        }
        if self.eat(b">") {
            return Ok(false);
        }
        Err(if self.eof() { TRUNCATED } else { INVALID })
    }

    fn finish_open(&mut self) -> Result<Open, Error> {
        let mut open = Open {
            self_close: false,
            width: None,
            height: None,
            xmlns: false,
        };
        while let Some((name, value)) = self.attr()? {
            match name {
                b"width" if open.width.is_none() => open.width = Some(canvas_px(value)?),
                b"height" if open.height.is_none() => open.height = Some(canvas_px(value)?),
                b"xmlns" if !open.xmlns && value == b"http://www.w3.org/2000/svg" => {
                    open.xmlns = true;
                }
                _ => return Err(UNSUPPORTED),
            }
        }
        open.self_close = self.end_tag()?;
        Ok(open)
    }

    fn finish_rect(&mut self) -> Result<RectTag, Error> {
        let mut x = None;
        let mut y = None;
        let mut width = None;
        let mut height = None;
        let mut fill = false;
        while let Some((name, value)) = self.attr()? {
            match name {
                b"x" if x.is_none() => x = Some(plain_u32(value)?),
                b"y" if y.is_none() => y = Some(plain_u32(value)?),
                b"width" if width.is_none() => width = Some(plain_u32(value)?),
                b"height" if height.is_none() => height = Some(plain_u32(value)?),
                b"fill" if !fill && value == b"#000000" => fill = true,
                _ => return Err(UNSUPPORTED),
            }
        }
        let self_close = self.end_tag()?;
        if !fill {
            return Err(UNSUPPORTED);
        }
        let (Some(x), Some(y), Some(width), Some(height)) = (x, y, width, height) else {
            return Err(UNSUPPORTED);
        };
        Ok(RectTag {
            self_close,
            x,
            y,
            width,
            height,
        })
    }
}

fn canvas_px(value: &[u8]) -> Result<u32, Error> {
    if value == b"16" {
        Ok(16)
    } else if value == b"32" {
        Ok(32)
    } else {
        Err(UNSUPPORTED)
    }
}

fn plain_u32(value: &[u8]) -> Result<u32, Error> {
    if value.is_empty() || value.iter().any(|b| !b.is_ascii_digit()) {
        return Err(UNSUPPORTED);
    }
    let mut n = 0u32;
    for b in value {
        n = n
            .checked_mul(10)
            .and_then(|n| n.checked_add((*b - b'0') as u32))
            .ok_or(UNSUPPORTED)?;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::UNSUPPORTED;

    fn square_on_white(x: u32, y: u32, side: u32) -> Raster {
        let mut pixels = vec![255u8; (SIDE * SIDE * 3) as usize];
        for row in y..y + side {
            for col in x..x + side {
                let i = ((row * SIDE + col) * 3) as usize;
                pixels[i] = 0;
                pixels[i + 1] = 0;
                pixels[i + 2] = 0;
            }
        }
        Raster::new(SIDE, SIDE, 3, pixels).unwrap()
    }

    #[test]
    fn black_square_on_white_round_trips() {
        let src = square_on_white(4, 4, 8);
        let file = encode_svg(&src).unwrap();
        assert_eq!(
            file,
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\"><rect x=\"4\" y=\"4\" width=\"8\" height=\"8\" fill=\"#000000\"/></svg>"
        );
        let back = rasterize_svg(&file).unwrap();
        assert_eq!(back.width, 16);
        assert_eq!(back.height, 16);
        assert_eq!(back.channels, 3);
        assert_eq!(back.x, 0);
        assert_eq!(back.y, 0);
        assert_eq!(back.pixels, src.pixels);
    }

    #[test]
    fn other_pictures_stay_unsupported() {
        assert_eq!(
            encode_svg(&square_on_white(0, 0, 16)).unwrap_err(),
            UNSUPPORTED
        );
        let mut bar = vec![255u8; (SIDE * SIDE * 3) as usize];
        for x in 2..10 {
            let i = x * 3;
            bar[i] = 0;
            bar[i + 1] = 0;
            bar[i + 2] = 0;
        }
        assert_eq!(
            encode_svg(&Raster::new(16, 16, 3, bar).unwrap()).unwrap_err(),
            UNSUPPORTED
        );
        let mut gray = vec![255u8; (SIDE * SIDE * 3) as usize];
        gray[0] = 1;
        assert_eq!(
            encode_svg(&Raster::new(16, 16, 3, gray).unwrap()).unwrap_err(),
            UNSUPPORTED
        );
        assert_eq!(
            encode_svg(&Raster::new(8, 8, 3, vec![255; 8 * 8 * 3]).unwrap()).unwrap_err(),
            UNSUPPORTED
        );
        let mut two = square_on_white(1, 1, 2);
        let i = ((10 * SIDE + 10) * 3) as usize;
        two.pixels[i] = 0;
        two.pixels[i + 1] = 0;
        two.pixels[i + 2] = 0;
        assert_eq!(encode_svg(&two).unwrap_err(), UNSUPPORTED);
    }

    #[test]
    fn other_svg_shapes_stay_unsupported() {
        let head = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"16\" height=\"16\">";
        for body in [
            b"<circle cx=\"8\" cy=\"8\" r=\"4\"/>".as_slice(),
            b"<path d=\"M0 0 L1 1\"/>".as_slice(),
            b"<text x=\"0\" y=\"8\">a</text>".as_slice(),
            b"<ellipse cx=\"8\" cy=\"8\" rx=\"4\" ry=\"4\"/>".as_slice(),
            b"<polygon points=\"0,0 1,1 0,1\"/>".as_slice(),
            b"<rect x=\"4\" y=\"4\" width=\"8\" height=\"8\" rx=\"2\" fill=\"#000000\"/>"
                .as_slice(),
        ] {
            let mut file = head.to_vec();
            file.extend_from_slice(body);
            file.extend_from_slice(b"</svg>");
            assert_eq!(rasterize_svg(&file).unwrap_err(), UNSUPPORTED);
        }
        assert_eq!(rasterize_svg(b"").unwrap_err(), INVALID);
        assert_eq!(rasterize_svg(b"<svg width=\"16\"").unwrap_err(), TRUNCATED);
    }

    fn square32(x: u32, y: u32, side: u32) -> Raster {
        let mut pixels = vec![255u8; 32 * 32 * 3];
        for row in y..y + side {
            for col in x..x + side {
                let i = ((row * 32 + col) * 3) as usize;
                pixels[i] = 0;
                pixels[i + 1] = 0;
                pixels[i + 2] = 0;
            }
        }
        Raster::new(32, 32, 3, pixels).unwrap()
    }

    #[test]
    fn black_square_32_on_white_round_trips() {
        let src = square32(8, 8, 16);
        let file = encode_svg(&src).unwrap();
        assert_eq!(
            file,
            b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"32\"><rect x=\"8\" y=\"8\" width=\"16\" height=\"16\" fill=\"#000000\"/></svg>"
        );
        let back = rasterize_svg(&file).unwrap();
        assert_eq!(back.width, 32);
        assert_eq!(back.height, 32);
        assert_eq!(back.channels, 3);
        assert_eq!(back.x, 0);
        assert_eq!(back.y, 0);
        assert_eq!(back.pixels, src.pixels);
        let corner = square32(0, 0, 8);
        let again = rasterize_svg(&encode_svg(&corner).unwrap()).unwrap();
        assert_eq!(again.pixels, corner.pixels);
    }

    #[test]
    fn other_32_pictures_and_shapes_stay_unsupported() {
        assert_eq!(encode_svg(&square32(0, 0, 32)).unwrap_err(), UNSUPPORTED);
        let mut bar = vec![255u8; 32 * 32 * 3];
        for x in 2..20 {
            let i = x * 3;
            bar[i] = 0;
            bar[i + 1] = 0;
            bar[i + 2] = 0;
        }
        assert_eq!(
            encode_svg(&Raster::new(32, 32, 3, bar).unwrap()).unwrap_err(),
            UNSUPPORTED
        );
        let mut gray = vec![255u8; 32 * 32 * 3];
        gray[0] = 1;
        assert_eq!(
            encode_svg(&Raster::new(32, 32, 3, gray).unwrap()).unwrap_err(),
            UNSUPPORTED
        );
        let mut two = square32(1, 1, 4);
        let i = ((20 * 32 + 20) * 3) as usize;
        two.pixels[i] = 0;
        two.pixels[i + 1] = 0;
        two.pixels[i + 2] = 0;
        assert_eq!(encode_svg(&two).unwrap_err(), UNSUPPORTED);
        assert_eq!(
            encode_svg(&Raster::new(64, 64, 3, vec![255; 64 * 64 * 3]).unwrap()).unwrap_err(),
            UNSUPPORTED
        );
        let head = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"32\">";
        for body in [
            b"<circle cx=\"16\" cy=\"16\" r=\"8\"/>".as_slice(),
            b"<path d=\"M0 0 L1 1\"/>".as_slice(),
            b"<text x=\"0\" y=\"8\">a</text>".as_slice(),
        ] {
            let mut file = head.to_vec();
            file.extend_from_slice(body);
            file.extend_from_slice(b"</svg>");
            assert_eq!(rasterize_svg(&file).unwrap_err(), UNSUPPORTED);
        }
        assert_eq!(
            rasterize_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"32\" height=\"16\"><rect x=\"0\" y=\"0\" width=\"8\" height=\"8\" fill=\"#000000\"/></svg>").unwrap_err(),
            UNSUPPORTED
        );
    }
}
