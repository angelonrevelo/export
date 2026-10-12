//! Walk one IDR slice far enough to know the Annex B buffer was not cut off.
//!
//! Coefficient bits are consumed so the next macroblock starts on the right
//! bit. This is not an encoder and it does not reconstruct pixels.

use crate::bits::{rbsp, Bits};
use crate::error::Error;
use crate::param::{Pps, Sps};

const BLK_X: [usize; 16] = [0, 1, 0, 1, 2, 3, 2, 3, 0, 1, 0, 1, 2, 3, 2, 3];
const BLK_Y: [usize; 16] = [0, 0, 1, 1, 0, 0, 1, 1, 2, 2, 3, 3, 2, 2, 3, 3];

const INTER_CBP: [u8; 48] = [
    0, 16, 1, 2, 4, 8, 32, 3, 5, 10, 12, 15, 47, 7, 11, 13, 14, 6, 9, 31, 35, 37, 42, 44, 33, 34,
    36, 40, 39, 43, 45, 46, 17, 18, 20, 24, 19, 21, 26, 28, 23, 27, 29, 30, 22, 25, 38, 41,
];

const INTRA_CBP: [u8; 48] = [
    47, 31, 15, 0, 23, 27, 29, 30, 7, 11, 13, 14, 39, 43, 45, 46, 16, 3, 5, 10, 12, 19, 21, 26, 28,
    35, 37, 42, 44, 1, 2, 4, 8, 17, 18, 20, 24, 6, 9, 22, 25, 32, 33, 34, 36, 40, 38, 41,
];

pub fn idr(sps: &Sps, pps: &Pps, nal: &[u8]) -> Result<(), Error> {
    if pps.entropy_cabac {
        return Err(Error::Frame);
    }
    let raw = rbsp(nal)?;
    let mut bits = Bits::new(&raw);
    header(&mut bits, sps, pps)?;
    let mb_w = sps.mb_w as usize;
    let mb_h = sps.mb_h as usize;
    let mut luma = vec![0u8; mb_w * 4 * mb_h * 4];
    let mut chroma_u = vec![0u8; mb_w * 2 * mb_h * 2];
    let mut chroma_v = vec![0u8; mb_w * 2 * mb_h * 2];
    for my in 0..mb_h {
        for mx in 0..mb_w {
            let mb_type = bits.ue()?;
            if mb_type != 0 {
                return Err(Error::Frame);
            }
            for _ in 0..16 {
                if bits.bit()? == 0 {
                    let _rem = bits.bits(3)?;
                }
            }
            let _chroma_pred = bits.ue()?;
            let cbp_code = bits.ue()?;
            if cbp_code >= 48 {
                return Err(Error::Frame);
            }
            let cbp = u32::from(INTRA_CBP[cbp_code as usize]);
            let cbpl = cbp & 15;
            let cbpc = cbp >> 4;
            if cbp != 0 {
                let _qp_delta = bits.se()?;
            }
            if cbpl != 0 {
                for i8 in 0..4 {
                    if cbpl & (1 << i8) == 0 {
                        continue;
                    }
                    for k in 0..4 {
                        let i = i8 * 4 + k;
                        let bx = mx * 4 + BLK_X[i];
                        let by = my * 4 + BLK_Y[i];
                        let nc = nc_at(&luma, mb_w * 4, bx, by);
                        luma[by * mb_w * 4 + bx] = read_block(&mut bits, nc, 16)? as u8;
                    }
                }
            }
            if cbpc > 0 {
                let _ = read_block(&mut bits, -1, 4)?;
                let _ = read_block(&mut bits, -1, 4)?;
            }
            if cbpc == 2 {
                for plane in [&mut chroma_u, &mut chroma_v] {
                    for i in 0..4 {
                        let bx = mx * 2 + (i & 1);
                        let by = my * 2 + (i >> 1);
                        let nc = nc_at(plane, mb_w * 2, bx, by);
                        plane[by * mb_w * 2 + bx] = read_block(&mut bits, nc, 15)? as u8;
                    }
                }
            }
        }
    }
    bits.finish()
}

/// One P-slice after the I-frame. A cut-off slice returns [`Error::Truncated`].
pub fn predicted(sps: &Sps, pps: &Pps, nal: &[u8]) -> Result<(), Error> {
    if pps.entropy_cabac || nal.is_empty() {
        return Err(Error::Frame);
    }
    let ref_idc = nal[0] >> 5;
    let raw = rbsp(nal)?;
    let mut bits = Bits::new(&raw);
    p_header(&mut bits, sps, pps, ref_idc)?;
    let mb_w = sps.mb_w as usize;
    let mb_h = sps.mb_h as usize;
    let mb_count = mb_w * mb_h;
    let mut luma = vec![0u8; mb_w * 4 * mb_h * 4];
    let mut addr = 0usize;
    while addr < mb_count {
        let skip = bits.ue()? as usize;
        if skip > mb_count - addr {
            return Err(Error::Frame);
        }
        addr += skip;
        if addr == mb_count {
            break;
        }
        let mx = addr % mb_w;
        let my = addr / mb_w;
        let mb_type = bits.ue()?;
        if mb_type != 0 {
            return Err(Error::Frame);
        }
        let _mvd_x = bits.se()?;
        let _mvd_y = bits.se()?;
        let cbp_code = bits.ue()?;
        if cbp_code >= 48 {
            return Err(Error::Frame);
        }
        let cbp = u32::from(INTER_CBP[cbp_code as usize]);
        let cbpl = cbp & 15;
        let cbpc = cbp >> 4;
        if cbp != 0 {
            let _qp_delta = bits.se()?;
        }
        if cbpl != 0 {
            for i8 in 0..4 {
                if cbpl & (1 << i8) == 0 {
                    continue;
                }
                for k in 0..4 {
                    let i = i8 * 4 + k;
                    let bx = mx * 4 + BLK_X[i];
                    let by = my * 4 + BLK_Y[i];
                    let nc = nc_at(&luma, mb_w * 4, bx, by);
                    luma[by * mb_w * 4 + bx] = read_block(&mut bits, nc, 16)? as u8;
                }
            }
        }
        if cbpc > 0 {
            let _ = read_block(&mut bits, -1, 4)?;
            let _ = read_block(&mut bits, -1, 4)?;
        }
        addr += 1;
    }
    bits.finish()
}

fn p_header(bits: &mut Bits<'_>, sps: &Sps, pps: &Pps, ref_idc: u8) -> Result<(), Error> {
    if bits.ue()? != 0 {
        return Err(Error::Frame);
    }
    let slice_type = bits.ue()?;
    if slice_type != 0 && slice_type != 5 {
        return Err(Error::Frame);
    }
    if bits.ue()? != pps.pps_id {
        return Err(Error::Frame);
    }
    let _frame_num = bits.bits(sps.log2_max_frame_num)?;
    if sps.poc_type == 0 {
        let _ = bits.bits(sps.log2_max_poc_lsb)?;
        if pps.bottom_field_pic_order {
            let _ = bits.se()?;
        }
    } else if sps.poc_type == 1 && !sps.delta_pic_order_always_zero {
        let _ = bits.se()?;
    }
    if bits.bit()? != 0 || bits.bit()? != 0 {
        return Err(Error::Frame);
    }
    if ref_idc != 0 && bits.bit()? != 0 {
        return Err(Error::Frame);
    }
    if pps.redundant_pic_cnt {
        let _ = bits.ue()?;
    }
    let _qp_delta = bits.se()?;
    if pps.deblocking_control {
        let disable = bits.ue()?;
        if disable != 1 {
            let _ = bits.se()?;
            let _ = bits.se()?;
        }
    }
    Ok(())
}

fn header(bits: &mut Bits<'_>, sps: &Sps, pps: &Pps) -> Result<(), Error> {
    if bits.ue()? != 0 {
        return Err(Error::Frame);
    }
    let slice_type = bits.ue()?;
    if slice_type != 2 && slice_type != 7 {
        return Err(Error::Frame);
    }
    if bits.ue()? != pps.pps_id {
        return Err(Error::Frame);
    }
    let _frame_num = bits.bits(sps.log2_max_frame_num)?;
    let _idr_pic_id = bits.ue()?;
    if sps.poc_type == 0 {
        let _ = bits.bits(sps.log2_max_poc_lsb)?;
        if pps.bottom_field_pic_order {
            let _ = bits.se()?;
        }
    } else if sps.poc_type == 1 && !sps.delta_pic_order_always_zero {
        let _ = bits.se()?;
    }
    let _no_output = bits.bit()?;
    let _long_term = bits.bit()?;
    if pps.redundant_pic_cnt {
        let _ = bits.ue()?;
    }
    let _qp_delta = bits.se()?;
    if pps.deblocking_control {
        let disable = bits.ue()?;
        if disable != 1 {
            let _ = bits.se()?;
            let _ = bits.se()?;
        }
    }
    Ok(())
}

fn nc_at(nnz: &[u8], stride: usize, x: usize, y: usize) -> i32 {
    let left = if x > 0 {
        Some(i32::from(nnz[y * stride + x - 1]))
    } else {
        None
    };
    let up = if y > 0 {
        Some(i32::from(nnz[(y - 1) * stride + x]))
    } else {
        None
    };
    match (left, up) {
        (Some(a), Some(b)) => (a + b + 1) >> 1,
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => 0,
    }
}

fn read_block(bits: &mut Bits<'_>, nc: i32, maxn: usize) -> Result<usize, Error> {
    let (total, trailing) = read_token(bits, nc)?;
    if total > maxn {
        return Err(Error::Frame);
    }
    if total == 0 {
        return Ok(0);
    }
    for _ in 0..trailing {
        let _sign = bits.bit()?;
    }
    let nlevels = total - trailing;
    let mut suffix: i32 = if total > 10 && trailing < 3 { 1 } else { 0 };
    for i in 0..nlevels {
        let mut code = read_level(bits, suffix)?;
        if i == 0 && trailing < 3 {
            code = code.saturating_add(2);
        }
        let level = if code % 2 == 0 {
            ((code + 2) / 2) as i32
        } else {
            -(code.div_ceil(2) as i32)
        };
        if suffix == 0 {
            suffix = 1;
        }
        let mag = level.unsigned_abs();
        if mag > (3u32 << (suffix as u32 - 1)) && suffix < 6 {
            suffix += 1;
        }
    }
    if total < maxn {
        let mut zeros_left = read_total_zeros(bits, total, maxn)?;
        for _ in (1..total).rev() {
            if zeros_left == 0 {
                break;
            }
            let run = read_run(bits, zeros_left)?;
            if run > zeros_left {
                return Err(Error::Frame);
            }
            zeros_left -= run;
        }
    }
    Ok(total)
}

fn read_token(bits: &mut Bits<'_>, nc: i32) -> Result<(usize, usize), Error> {
    if nc >= 8 {
        let code = bits.bits(6)?;
        for (i, &entry) in FLC.iter().enumerate() {
            if entry == code {
                if let Some(pair) = coeff_pair(i, 16) {
                    return Ok(pair);
                }
            }
        }
        return Err(Error::Truncated);
    }
    let table: &[(u32, u8)] = if nc < 0 {
        &CHROMA_DC
    } else if nc < 2 {
        &VLC0
    } else if nc < 4 {
        &VLC1
    } else {
        &VLC2
    };
    let max_total = if nc < 0 { 4 } else { 16 };
    for length in 1u32..=16 {
        if !enough(bits, length) {
            break;
        }
        let peeked = bits.peek(length)?;
        let mut found = None;
        for (i, &(code, len)) in table.iter().enumerate() {
            if u32::from(len) == length && code == peeked {
                if let Some(pair) = coeff_pair(i, max_total) {
                    if found.is_some() {
                        return Err(Error::Frame);
                    }
                    found = Some(pair);
                }
            }
        }
        if let Some(pair) = found {
            let _ = bits.bits(length)?;
            return Ok(pair);
        }
    }
    Err(Error::Truncated)
}

fn enough(bits: &Bits<'_>, n: u32) -> bool {
    bits.peek(n).is_ok()
}

fn coeff_pair(idx: usize, max_total: usize) -> Option<(usize, usize)> {
    if idx == 0 {
        return Some((0, 0));
    }
    for total in 1..=max_total {
        let trailing_max = total.min(3);
        for trailing in 0..=trailing_max {
            if trailing + 4 * (total - trailing) == idx {
                return Some((total, trailing));
            }
        }
    }
    None
}

fn read_level(bits: &mut Bits<'_>, suffix: i32) -> Result<u32, Error> {
    let mut prefix = 0u32;
    while bits.bit()? == 0 {
        prefix += 1;
        if prefix > 20 {
            return Err(Error::Truncated);
        }
    }
    if suffix == 0 {
        if prefix < 14 {
            return Ok(prefix);
        }
        if prefix == 14 {
            return Ok(14 + bits.bits(4)?);
        }
        return Ok(30 + bits.bits(12)?);
    }
    if prefix < 15 {
        Ok((prefix << (suffix as u32)) + bits.bits(suffix as u32)?)
    } else {
        Ok((15u32 << (suffix as u32)) + bits.bits(12)?)
    }
}

fn read_total_zeros(bits: &mut Bits<'_>, total: usize, maxn: usize) -> Result<usize, Error> {
    let max_zeros = maxn - total;
    for length in 1u32..=12 {
        if !enough(bits, length) {
            break;
        }
        let peeked = bits.peek(length)?;
        let mut found = None;
        for zeros in 0..=max_zeros {
            let Some((code, len)) = total_zeros_code(total, zeros, maxn) else {
                continue;
            };
            if u32::from(len) == length && code == peeked {
                if found.is_some() {
                    return Err(Error::Frame);
                }
                found = Some(zeros);
            }
        }
        if let Some(zeros) = found {
            let _ = bits.bits(length)?;
            return Ok(zeros);
        }
    }
    Err(Error::Truncated)
}

fn read_run(bits: &mut Bits<'_>, zeros_left: usize) -> Result<usize, Error> {
    let limit = if zeros_left >= 7 {
        zeros_left.min(14)
    } else {
        zeros_left
    };
    for length in 1u32..=12 {
        if !enough(bits, length) {
            break;
        }
        let peeked = bits.peek(length)?;
        let mut found = None;
        for run in 0..=limit {
            let Some((code, len)) = run_before_code(run, zeros_left) else {
                continue;
            };
            if u32::from(len) == length && code == peeked {
                if found.is_some() {
                    return Err(Error::Frame);
                }
                found = Some(run);
            }
        }
        if let Some(run) = found {
            let _ = bits.bits(length)?;
            return Ok(run);
        }
    }
    Err(Error::Truncated)
}

fn total_zeros_code(total: usize, zeros: usize, maxn: usize) -> Option<(u32, u8)> {
    if maxn == 4 {
        return chroma_dc_zeros(total, zeros);
    }
    luma_zeros(total, zeros)
}

fn chroma_dc_zeros(total: usize, zeros: usize) -> Option<(u32, u8)> {
    const T0: [(u32, u8); 4] = [(1, 1), (1, 2), (1, 3), (0, 3)];
    const T1: [(u32, u8); 3] = [(1, 1), (1, 2), (0, 2)];
    const T2: [(u32, u8); 2] = [(1, 1), (0, 1)];
    let row: &[(u32, u8)] = match total {
        1 => &T0,
        2 => &T1,
        3 => &T2,
        _ => return None,
    };
    row.get(zeros).copied()
}

fn luma_zeros(total: usize, zeros: usize) -> Option<(u32, u8)> {
    const T0: [(u32, u8); 16] = [
        (1, 1),
        (3, 3),
        (2, 3),
        (3, 4),
        (2, 4),
        (3, 5),
        (2, 5),
        (3, 6),
        (2, 6),
        (3, 7),
        (2, 7),
        (3, 8),
        (2, 8),
        (3, 9),
        (2, 9),
        (1, 9),
    ];
    const T1: [(u32, u8); 15] = [
        (7, 3),
        (6, 3),
        (5, 3),
        (4, 3),
        (3, 3),
        (5, 4),
        (4, 4),
        (3, 4),
        (2, 4),
        (3, 5),
        (2, 5),
        (3, 6),
        (2, 6),
        (1, 6),
        (0, 6),
    ];
    const T2: [(u32, u8); 14] = [
        (5, 4),
        (7, 3),
        (6, 3),
        (5, 3),
        (4, 4),
        (3, 4),
        (4, 3),
        (3, 3),
        (2, 4),
        (3, 5),
        (2, 5),
        (1, 6),
        (1, 5),
        (0, 6),
    ];
    const T3: [(u32, u8); 13] = [
        (3, 5),
        (7, 3),
        (5, 4),
        (4, 4),
        (6, 3),
        (5, 3),
        (4, 3),
        (3, 4),
        (3, 3),
        (2, 4),
        (2, 5),
        (1, 5),
        (0, 5),
    ];
    const T4: [(u32, u8); 12] = [
        (5, 4),
        (4, 4),
        (3, 4),
        (7, 3),
        (6, 3),
        (5, 3),
        (4, 3),
        (3, 3),
        (2, 4),
        (1, 5),
        (1, 4),
        (0, 5),
    ];
    const T5: [(u32, u8); 11] = [
        (1, 6),
        (1, 5),
        (7, 3),
        (6, 3),
        (5, 3),
        (4, 3),
        (3, 3),
        (2, 3),
        (1, 4),
        (1, 3),
        (0, 6),
    ];
    const T6: [(u32, u8); 10] = [
        (1, 6),
        (1, 5),
        (5, 3),
        (4, 3),
        (3, 3),
        (3, 2),
        (2, 3),
        (1, 4),
        (1, 3),
        (0, 6),
    ];
    const T7: [(u32, u8); 9] = [
        (1, 6),
        (1, 4),
        (1, 5),
        (3, 3),
        (3, 2),
        (2, 2),
        (2, 3),
        (1, 3),
        (0, 6),
    ];
    const T8: [(u32, u8); 8] = [
        (1, 6),
        (0, 6),
        (1, 4),
        (3, 2),
        (2, 2),
        (1, 3),
        (1, 2),
        (1, 5),
    ];
    const T9: [(u32, u8); 7] = [(1, 5), (0, 5), (1, 3), (3, 2), (2, 2), (1, 2), (1, 4)];
    const T10: [(u32, u8); 6] = [(0, 4), (1, 4), (1, 3), (2, 3), (1, 1), (3, 3)];
    const T11: [(u32, u8); 5] = [(0, 4), (1, 4), (1, 2), (1, 1), (1, 3)];
    const T12: [(u32, u8); 4] = [(0, 3), (1, 3), (1, 1), (1, 2)];
    const T13: [(u32, u8); 3] = [(0, 2), (1, 2), (1, 1)];
    const T14: [(u32, u8); 2] = [(0, 1), (1, 1)];
    let row: &[(u32, u8)] = match total {
        1 => &T0,
        2 => &T1,
        3 => &T2,
        4 => &T3,
        5 => &T4,
        6 => &T5,
        7 => &T6,
        8 => &T7,
        9 => &T8,
        10 => &T9,
        11 => &T10,
        12 => &T11,
        13 => &T12,
        14 => &T13,
        15 => &T14,
        _ => return None,
    };
    row.get(zeros).copied()
}

fn run_before_code(run: usize, zeros_left: usize) -> Option<(u32, u8)> {
    const Z1: [(u32, u8); 2] = [(1, 1), (0, 1)];
    const Z2: [(u32, u8); 3] = [(1, 1), (1, 2), (0, 2)];
    const Z3: [(u32, u8); 4] = [(3, 2), (2, 2), (1, 2), (0, 2)];
    const Z4: [(u32, u8); 5] = [(3, 2), (2, 2), (1, 2), (1, 3), (0, 3)];
    const Z5: [(u32, u8); 6] = [(3, 2), (2, 2), (3, 3), (2, 3), (1, 3), (0, 3)];
    const Z6: [(u32, u8); 7] = [(3, 2), (0, 3), (1, 3), (3, 3), (2, 3), (5, 3), (4, 3)];
    const Z7: [(u32, u8); 15] = [
        (7, 3),
        (6, 3),
        (5, 3),
        (4, 3),
        (3, 3),
        (2, 3),
        (1, 3),
        (1, 4),
        (1, 5),
        (1, 6),
        (1, 7),
        (1, 8),
        (1, 9),
        (1, 10),
        (1, 11),
    ];
    let row: &[(u32, u8)] = match zeros_left {
        1 => &Z1,
        2 => &Z2,
        3 => &Z3,
        4 => &Z4,
        5 => &Z5,
        6 => &Z6,
        _ => &Z7,
    };
    row.get(run).copied()
}

const VLC0: [(u32, u8); 65] = [
    (1, 1),
    (1, 2),
    (1, 3),
    (3, 5),
    (5, 6),
    (4, 6),
    (5, 7),
    (3, 6),
    (7, 8),
    (6, 8),
    (5, 8),
    (4, 7),
    (7, 9),
    (6, 9),
    (5, 9),
    (4, 8),
    (7, 10),
    (6, 10),
    (5, 10),
    (4, 9),
    (7, 11),
    (6, 11),
    (5, 11),
    (4, 10),
    (15, 13),
    (14, 13),
    (13, 13),
    (4, 11),
    (11, 13),
    (10, 13),
    (9, 13),
    (12, 13),
    (8, 13),
    (14, 14),
    (13, 14),
    (12, 14),
    (15, 14),
    (10, 14),
    (9, 14),
    (8, 14),
    (11, 14),
    (14, 15),
    (13, 15),
    (12, 15),
    (15, 15),
    (10, 15),
    (9, 15),
    (8, 15),
    (11, 15),
    (1, 15),
    (13, 16),
    (12, 16),
    (15, 16),
    (14, 16),
    (9, 16),
    (8, 16),
    (11, 16),
    (10, 16),
    (5, 16),
    (0, 0),
    (7, 16),
    (6, 16),
    (0, 0),
    (0, 0),
    (4, 16),
];

const VLC1: [(u32, u8); 65] = [
    (3, 2),
    (2, 2),
    (3, 3),
    (5, 4),
    (11, 6),
    (7, 5),
    (9, 6),
    (4, 4),
    (7, 6),
    (10, 6),
    (5, 6),
    (6, 5),
    (7, 7),
    (6, 6),
    (5, 7),
    (8, 6),
    (7, 8),
    (6, 7),
    (5, 8),
    (4, 6),
    (4, 8),
    (6, 8),
    (5, 9),
    (4, 7),
    (7, 9),
    (6, 9),
    (13, 11),
    (4, 9),
    (15, 11),
    (14, 11),
    (9, 11),
    (12, 11),
    (11, 11),
    (10, 11),
    (13, 12),
    (8, 11),
    (15, 12),
    (14, 12),
    (9, 12),
    (12, 12),
    (11, 12),
    (10, 12),
    (13, 13),
    (12, 13),
    (8, 12),
    (14, 13),
    (9, 13),
    (8, 13),
    (15, 13),
    (10, 13),
    (6, 13),
    (1, 13),
    (11, 13),
    (11, 14),
    (10, 14),
    (4, 14),
    (7, 13),
    (8, 14),
    (5, 14),
    (0, 0),
    (9, 14),
    (6, 14),
    (0, 0),
    (0, 0),
    (7, 14),
];

const VLC2: [(u32, u8); 65] = [
    (15, 4),
    (14, 4),
    (13, 4),
    (12, 4),
    (15, 6),
    (15, 5),
    (14, 5),
    (11, 4),
    (11, 6),
    (12, 5),
    (11, 5),
    (10, 4),
    (8, 6),
    (10, 5),
    (9, 5),
    (9, 4),
    (15, 7),
    (8, 5),
    (13, 6),
    (8, 4),
    (11, 7),
    (14, 6),
    (9, 6),
    (13, 5),
    (9, 7),
    (10, 6),
    (13, 7),
    (12, 6),
    (8, 7),
    (14, 7),
    (10, 7),
    (12, 7),
    (15, 8),
    (14, 8),
    (13, 8),
    (12, 8),
    (11, 8),
    (10, 8),
    (9, 8),
    (8, 8),
    (15, 9),
    (14, 9),
    (13, 9),
    (12, 9),
    (11, 9),
    (10, 9),
    (9, 9),
    (10, 10),
    (8, 9),
    (7, 9),
    (11, 10),
    (6, 10),
    (13, 10),
    (12, 10),
    (7, 10),
    (2, 10),
    (9, 10),
    (8, 10),
    (3, 10),
    (0, 0),
    (5, 10),
    (4, 10),
    (0, 0),
    (0, 0),
    (1, 10),
];

const FLC: [u32; 65] = [
    3, 1, 6, 11, 0, 5, 10, 15, 4, 9, 14, 19, 8, 13, 18, 23, 12, 17, 22, 27, 16, 21, 26, 31, 20, 25,
    30, 35, 24, 29, 34, 39, 28, 33, 38, 43, 32, 37, 42, 47, 36, 41, 46, 51, 40, 45, 50, 55, 44, 49,
    54, 59, 48, 53, 58, 63, 52, 57, 62, 0, 56, 61, 0, 0, 60,
];

const CHROMA_DC: [(u32, u8); 17] = [
    (1, 2),
    (1, 1),
    (1, 3),
    (5, 6),
    (7, 6),
    (6, 6),
    (2, 7),
    (0, 7),
    (4, 6),
    (3, 7),
    (2, 8),
    (0, 0),
    (3, 6),
    (3, 8),
    (0, 0),
    (0, 0),
    (2, 6),
];
