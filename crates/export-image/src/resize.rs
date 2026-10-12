use crate::error::{Error, INVALID};
use crate::raster::Raster;

/// Downscale and upscale filters. Both write one output buffer and nothing else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    /// Box filter. Each output sample is the mean of the source rectangle it covers.
    Area,
    /// Triangle kernel in two axes.
    Bilinear,
}

pub fn resize(src: &Raster, width: u32, height: u32, filter: Filter) -> Result<Raster, Error> {
    if width == 0 || height == 0 {
        return Err(INVALID);
    }
    let channels = src.channels as usize;
    let mut pixels = vec![0u8; width as usize * height as usize * channels];
    match filter {
        Filter::Area => area(src, width, height, &mut pixels),
        Filter::Bilinear => bilinear(src, width, height, &mut pixels),
    }
    Raster::new(width, height, src.channels, pixels)
}

fn area(src: &Raster, dw: u32, dh: u32, dst: &mut [u8]) {
    let sw = src.width as u64;
    let sh = src.height as u64;
    let ch = src.channels as usize;
    let dwu = dw as u64;
    let dhu = dh as u64;
    for y in 0..dhu {
        let y0 = y * sh / dhu;
        let y1 = ((y + 1) * sh / dhu).max(y0 + 1).min(sh);
        for x in 0..dwu {
            let x0 = x * sw / dwu;
            let x1 = ((x + 1) * sw / dwu).max(x0 + 1).min(sw);
            let n = (x1 - x0) * (y1 - y0);
            for c in 0..ch {
                let mut sum = 0u64;
                for yy in y0..y1 {
                    for xx in x0..x1 {
                        sum += u64::from(src.sample(xx as u32, yy as u32, c as u8));
                    }
                }
                dst[((y as usize * dw as usize) + x as usize) * ch + c] =
                    ((sum + n / 2) / n) as u8;
            }
        }
    }
}

fn bilinear(src: &Raster, dw: u32, dh: u32, dst: &mut [u8]) {
    let sw = src.width as f32;
    let sh = src.height as f32;
    let ch = src.channels as usize;
    let w = src.width;
    let h = src.height;
    for y in 0..dh {
        let sy = (y as f32 + 0.5) * sh / dh as f32 - 0.5;
        let y0 = sy.floor().max(0.0) as u32;
        let y1 = (y0 + 1).min(h - 1);
        let ty = if sy <= 0.0 {
            0.0
        } else {
            (sy - y0 as f32).clamp(0.0, 1.0)
        };
        for x in 0..dw {
            let sx = (x as f32 + 0.5) * sw / dw as f32 - 0.5;
            let x0 = sx.floor().max(0.0) as u32;
            let x1 = (x0 + 1).min(w - 1);
            let tx = if sx <= 0.0 {
                0.0
            } else {
                (sx - x0 as f32).clamp(0.0, 1.0)
            };
            for c in 0..ch {
                let c8 = c as u8;
                let p00 = f32::from(src.sample(x0, y0, c8));
                let p10 = f32::from(src.sample(x1, y0, c8));
                let p01 = f32::from(src.sample(x0, y1, c8));
                let p11 = f32::from(src.sample(x1, y1, c8));
                let top = p00 + (p10 - p00) * tx;
                let bot = p01 + (p11 - p01) * tx;
                let v = (top + (bot - top) * ty).round();
                let byte = if v <= 0.0 {
                    0
                } else if v >= 255.0 {
                    255
                } else {
                    v as u8
                };
                dst[((y as usize * dw as usize) + x as usize) * ch + c] = byte;
            }
        }
    }
}
