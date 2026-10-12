use crate::bitstream::{BitReader, BitWriter};
use crate::error::Error;

// coeff_token: index = trailing_ones + 4 * (total_coeff - trailing_ones).
// (code, bit length). nC >= 8 is a fixed 6-bit code stored as the code value.

const VLC0: [(u32, u8); 65] = [
    (1, 1), (1, 2), (1, 3), (3, 5), (5, 6), (4, 6), (5, 7), (3, 6),
    (7, 8), (6, 8), (5, 8), (4, 7), (7, 9), (6, 9), (5, 9), (4, 8),
    (7, 10), (6, 10), (5, 10), (4, 9), (7, 11), (6, 11), (5, 11), (4, 10),
    (15, 13), (14, 13), (13, 13), (4, 11), (11, 13), (10, 13), (9, 13), (12, 13),
    (8, 13), (14, 14), (13, 14), (12, 14), (15, 14), (10, 14), (9, 14), (8, 14),
    (11, 14), (14, 15), (13, 15), (12, 15), (15, 15), (10, 15), (9, 15), (8, 15),
    (11, 15), (1, 15), (13, 16), (12, 16), (15, 16), (14, 16), (9, 16), (8, 16),
    (11, 16), (10, 16), (5, 16), (0, 0), (7, 16), (6, 16), (0, 0), (0, 0), (4, 16),
];

const VLC1: [(u32, u8); 65] = [
    (3, 2), (2, 2), (3, 3), (5, 4), (11, 6), (7, 5), (9, 6), (4, 4),
    (7, 6), (10, 6), (5, 6), (6, 5), (7, 7), (6, 6), (5, 7), (8, 6),
    (7, 8), (6, 7), (5, 8), (4, 6), (4, 8), (6, 8), (5, 9), (4, 7),
    (7, 9), (6, 9), (13, 11), (4, 9), (15, 11), (14, 11), (9, 11), (12, 11),
    (11, 11), (10, 11), (13, 12), (8, 11), (15, 12), (14, 12), (9, 12), (12, 12),
    (11, 12), (10, 12), (13, 13), (12, 13), (8, 12), (14, 13), (9, 13), (8, 13),
    (15, 13), (10, 13), (6, 13), (1, 13), (11, 13), (11, 14), (10, 14), (4, 14),
    (7, 13), (8, 14), (5, 14), (0, 0), (9, 14), (6, 14), (0, 0), (0, 0), (7, 14),
];

const VLC2: [(u32, u8); 65] = [
    (15, 4), (14, 4), (13, 4), (12, 4), (15, 6), (15, 5), (14, 5), (11, 4),
    (11, 6), (12, 5), (11, 5), (10, 4), (8, 6), (10, 5), (9, 5), (9, 4),
    (15, 7), (8, 5), (13, 6), (8, 4), (11, 7), (14, 6), (9, 6), (13, 5),
    (9, 7), (10, 6), (13, 7), (12, 6), (8, 7), (14, 7), (10, 7), (12, 7),
    (15, 8), (14, 8), (13, 8), (12, 8), (11, 8), (10, 8), (9, 8), (8, 8),
    (15, 9), (14, 9), (13, 9), (12, 9), (11, 9), (10, 9), (9, 9), (10, 10),
    (8, 9), (7, 9), (11, 10), (6, 10), (13, 10), (12, 10), (7, 10), (2, 10),
    (9, 10), (8, 10), (3, 10), (0, 0), (5, 10), (4, 10), (0, 0), (0, 0), (1, 10),
];

const FLC: [u32; 65] = [
    3, 1, 6, 11, 0, 5, 10, 15, 4, 9, 14, 19, 8, 13, 18, 23, 12, 17, 22, 27, 16, 21, 26, 31, 20,
    25, 30, 35, 24, 29, 34, 39, 28, 33, 38, 43, 32, 37, 42, 47, 36, 41, 46, 51, 40, 45, 50, 55,
    44, 49, 54, 59, 48, 53, 58, 63, 52, 57, 62, 0, 56, 61, 0, 0, 60,
];

/// nC == -1, chroma DC 4:2:0, at most 4 coefficients.
const CHROMA_DC_TOKEN: [(u32, u8); 17] = [
    (1, 2), (1, 1), (1, 3), (5, 6), (7, 6), (6, 6), (2, 7), (0, 7), (4, 6),
    (3, 7), (2, 8), (0, 0), (3, 6), (3, 8), (0, 0), (0, 0), (2, 6),
];

fn token(total: usize, trailing: usize, nc: i32) -> (u32, u8) {
    let idx = if total == 0 {
        0
    } else {
        trailing + 4 * (total - trailing)
    };
    if nc < 0 {
        CHROMA_DC_TOKEN[idx]
    } else if nc < 2 {
        VLC0[idx]
    } else if nc < 4 {
        VLC1[idx]
    } else if nc < 8 {
        VLC2[idx]
    } else {
        (FLC[idx], 6)
    }
}

fn write_level_code(bs: &mut BitWriter, code: i32, suffix: i32) {
    if suffix == 0 {
        if code < 14 {
            bs.write_zeros(code as u32);
            bs.write_bit(1);
        } else if code < 30 {
            bs.write_zeros(14);
            bs.write_bit(1);
            bs.write_bits((code - 14) as u32, 4);
        } else {
            bs.write_zeros(15);
            bs.write_bit(1);
            bs.write_bits((code - 30) as u32, 12);
        }
        return;
    }
    let prefix = code >> suffix;
    if prefix < 15 {
        bs.write_zeros(prefix as u32);
        bs.write_bit(1);
        let mask = (1u32 << suffix) - 1;
        bs.write_bits((code as u32) & mask, suffix as u32);
    } else {
        bs.write_zeros(15);
        bs.write_bit(1);
        let esc = code - (15 << suffix);
        bs.write_bits(esc as u32, 12);
    }
}

fn write_levels(bs: &mut BitWriter, levels_hi_to_lo: &[i32], trailing_ones: usize, total: usize) {
    let mut suffix: i32 = if total > 10 && trailing_ones < 3 { 1 } else { 0 };
    for (i, &level) in levels_hi_to_lo.iter().enumerate() {
        let mut code = if level > 0 {
            (level << 1) - 2
        } else {
            ((-level) << 1) - 1
        };
        if i == 0 && trailing_ones < 3 {
            code -= 2;
        }
        write_level_code(bs, code, suffix);
        if suffix == 0 {
            suffix = 1;
        }
        if level.abs() > (3 << (suffix - 1)) && suffix < 6 {
            suffix += 1;
        }
    }
}

// total_zeros for a 4x4 (max 16 coeffs). Row is total_coeff-1, column is total_zeros.
fn total_zeros_16(total: usize, zeros: usize) -> (u32, u8) {
    const T0: [(u32, u8); 16] = [
        (1, 1), (3, 3), (2, 3), (3, 4), (2, 4), (3, 5), (2, 5), (3, 6), (2, 6), (3, 7), (2, 7),
        (3, 8), (2, 8), (3, 9), (2, 9), (1, 9),
    ];
    const T1: [(u32, u8); 15] = [
        (7, 3), (6, 3), (5, 3), (4, 3), (3, 3), (5, 4), (4, 4), (3, 4), (2, 4), (3, 5), (2, 5),
        (3, 6), (2, 6), (1, 6), (0, 6),
    ];
    const T2: [(u32, u8); 14] = [
        (5, 4), (7, 3), (6, 3), (5, 3), (4, 4), (3, 4), (4, 3), (3, 3), (2, 4), (3, 5), (2, 5),
        (1, 6), (1, 5), (0, 6),
    ];
    const T3: [(u32, u8); 13] = [
        (3, 5), (7, 3), (5, 4), (4, 4), (6, 3), (5, 3), (4, 3), (3, 4), (3, 3), (2, 4), (2, 5),
        (1, 5), (0, 5),
    ];
    const T4: [(u32, u8); 12] = [
        (5, 4), (4, 4), (3, 4), (7, 3), (6, 3), (5, 3), (4, 3), (3, 3), (2, 4), (1, 5), (1, 4),
        (0, 5),
    ];
    const T5: [(u32, u8); 11] = [
        (1, 6), (1, 5), (7, 3), (6, 3), (5, 3), (4, 3), (3, 3), (2, 3), (1, 4), (1, 3), (0, 6),
    ];
    const T6: [(u32, u8); 10] = [
        (1, 6), (1, 5), (5, 3), (4, 3), (3, 3), (3, 2), (2, 3), (1, 4), (1, 3), (0, 6),
    ];
    const T7: [(u32, u8); 9] = [
        (1, 6), (1, 4), (1, 5), (3, 3), (3, 2), (2, 2), (2, 3), (1, 3), (0, 6),
    ];
    const T8: [(u32, u8); 8] = [(1, 6), (0, 6), (1, 4), (3, 2), (2, 2), (1, 3), (1, 2), (1, 5)];
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
        _ => &T14,
    };
    row[zeros]
}

fn total_zeros_chroma_dc(total: usize, zeros: usize) -> (u32, u8) {
    const T0: [(u32, u8); 4] = [(1, 1), (1, 2), (1, 3), (0, 3)];
    const T1: [(u32, u8); 3] = [(1, 1), (1, 2), (0, 2)];
    const T2: [(u32, u8); 2] = [(1, 1), (0, 1)];
    match total {
        1 => T0[zeros],
        2 => T1[zeros],
        _ => T2[zeros],
    }
}

fn run_before(run: usize, zeros_left: usize) -> (u32, u8) {
    const Z1: [(u32, u8); 2] = [(1, 1), (0, 1)];
    const Z2: [(u32, u8); 3] = [(1, 1), (1, 2), (0, 2)];
    const Z3: [(u32, u8); 4] = [(3, 2), (2, 2), (1, 2), (0, 2)];
    const Z4: [(u32, u8); 5] = [(3, 2), (2, 2), (1, 2), (1, 3), (0, 3)];
    const Z5: [(u32, u8); 6] = [(3, 2), (2, 2), (3, 3), (2, 3), (1, 3), (0, 3)];
    const Z6: [(u32, u8); 7] = [(3, 2), (0, 3), (1, 3), (3, 3), (2, 3), (5, 3), (4, 3)];
    const Z7: [(u32, u8); 15] = [
        (7, 3), (6, 3), (5, 3), (4, 3), (3, 3), (2, 3), (1, 3), (1, 4), (1, 5), (1, 6), (1, 7),
        (1, 8), (1, 9), (1, 10), (1, 11),
    ];
    match zeros_left {
        1 => Z1[run],
        2 => Z2[run],
        3 => Z3[run],
        4 => Z4[run],
        5 => Z5[run],
        6 => Z6[run],
        _ => Z7[run],
    }
}

/// CAVLC residual. `nc` is -1 for chroma DC. `coeffs` is zigzag order.
pub fn write_block(bs: &mut BitWriter, coeffs: &[i32], nc: i32) {
    let maxn = coeffs.len();
    let mut pos = [0usize; 16];
    let mut val = [0i32; 16];
    let mut total = 0usize;
    for (i, &c) in coeffs.iter().enumerate() {
        if c != 0 {
            pos[total] = i;
            val[total] = c;
            total += 1;
        }
    }
    let mut trailing = 0usize;
    while trailing < total && trailing < 3 && val[total - 1 - trailing].abs() == 1 {
        trailing += 1;
    }
    let (code, len) = token(total, trailing, nc);
    bs.write_bits(code, len as u32);
    if total == 0 {
        return;
    }
    for k in 0..trailing {
        let sign = if val[total - 1 - k] < 0 { 1 } else { 0 };
        bs.write_bit(sign);
    }
    let nlevels = total - trailing;
    if nlevels > 0 {
        let mut hi = Vec::with_capacity(nlevels);
        for k in 0..nlevels {
            hi.push(val[nlevels - 1 - k]);
        }
        write_levels(bs, &hi, trailing, total);
    }
    if total < maxn {
        let last = pos[total - 1];
        let zeros = last + 1 - total;
        let (zc, zl) = if nc < 0 {
            total_zeros_chroma_dc(total, zeros)
        } else {
            total_zeros_16(total, zeros)
        };
        bs.write_bits(zc, zl as u32);
        let mut zeros_left = zeros;
        for k in (1..total).rev() {
            if zeros_left == 0 {
                break;
            }
            let run = pos[k] - pos[k - 1] - 1;
            let (rc, rl) = run_before(run, zeros_left);
            bs.write_bits(rc, rl as u32);
            zeros_left -= run;
        }
    }
}

/// Intra coded_block_pattern (0..=47) to the Exp-Golomb codeNum.
/// codeNum 0 is the all-coded pattern 47. Pattern 0 is codeNum 3.
pub fn intra_cbp_code(cbp: usize) -> u32 {
    const MAP: [u32; 48] = [
        3, 29, 30, 17, 31, 18, 37, 8, 32, 38, 19, 9, 20, 10, 11, 2, 16, 33, 34, 21, 35, 22, 39, 4,
        36, 40, 23, 5, 24, 6, 7, 1, 41, 42, 43, 25, 44, 26, 46, 12, 45, 47, 27, 13, 28, 14, 15, 0,
    ];
    MAP[cbp]
}

/// Inter coded_block_pattern (0..=47) to the Exp-Golomb codeNum.
/// Pattern 0 is codeNum 0.
pub fn inter_cbp_code(cbp: usize) -> u32 {
    const MAP: [u32; 48] = [
        0, 2, 3, 7, 4, 8, 17, 13, 5, 18, 9, 14, 10, 15, 16, 11, 1, 32, 33, 36, 34, 37, 44, 40, 35,
        45, 38, 41, 39, 42, 43, 19, 6, 24, 25, 20, 26, 21, 46, 28, 27, 47, 22, 29, 23, 30, 31, 12,
    ];
    MAP[cbp]
}

pub(crate) fn intra_cbp_from_code(code: u32) -> Option<usize> {
    (0..48).find(|&cbp| intra_cbp_code(cbp) == code)
}

fn take_code(
    bits: &mut BitReader<'_>,
    max_len: u32,
    mut matches: impl FnMut(u32, u32) -> Option<usize>,
) -> Result<usize, Error> {
    for len in 1..=max_len {
        let peeked = match bits.peek_bits(len) {
            Ok(v) => v,
            Err(_) => break,
        };
        if let Some(found) = matches(len, peeked) {
            bits.read_bits(len)?;
            return Ok(found);
        }
    }
    Err(Error::Bytes)
}

fn read_token(bits: &mut BitReader<'_>, nc: i32) -> Result<(usize, usize), Error> {
    let max_bits: u32 = if nc < 0 {
        8
    } else if nc >= 8 {
        6
    } else {
        16
    };
    let max_total = if nc < 0 { 4 } else { 16 };
    let packed = take_code(bits, max_bits, |len, peeked| {
        let mut found = None;
        for total in 0..=max_total {
            for trailing in 0..=total.min(3) {
                let (code, clen) = token(total, trailing, nc);
                if clen == 0 || clen as u32 != len || code != peeked {
                    continue;
                }
                if found.is_some() {
                    return None;
                }
                found = Some(trailing + (total << 2));
            }
        }
        found
    })?;
    Ok((packed >> 2, packed & 3))
}

fn read_level_code(bits: &mut BitReader<'_>, suffix: i32) -> Result<i32, Error> {
    let mut zeros = 0i32;
    loop {
        if bits.read_bit()? == 1 {
            break;
        }
        zeros += 1;
        if zeros > 15 {
            return Err(Error::Bytes);
        }
    }
    if suffix == 0 {
        if zeros < 14 {
            return Ok(zeros);
        }
        if zeros == 14 {
            return Ok(14 + bits.read_bits(4)? as i32);
        }
        return Ok(30 + bits.read_bits(12)? as i32);
    }
    if zeros < 15 {
        let low = bits.read_bits(suffix as u32)? as i32;
        return Ok((zeros << suffix) | low);
    }
    let esc = bits.read_bits(12)? as i32;
    Ok((15 << suffix) + esc)
}

fn level_from_code(mut code: i32, first: bool, trailing: usize) -> i32 {
    if first && trailing < 3 {
        code += 2;
    }
    if code & 1 == 0 {
        (code + 2) >> 1
    } else {
        -((code + 1) >> 1)
    }
}

fn read_total_zeros(bits: &mut BitReader<'_>, total: usize, maxn: usize) -> Result<usize, Error> {
    let max_zeros = maxn - total;
    take_code(bits, 9, |len, peeked| {
        let mut found = None;
        for zeros in 0..=max_zeros {
            let (code, clen) = if maxn == 4 {
                total_zeros_chroma_dc(total, zeros)
            } else {
                total_zeros_16(total, zeros)
            };
            if clen as u32 != len || code != peeked {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(zeros);
        }
        found
    })
}

fn read_run_before(bits: &mut BitReader<'_>, zeros_left: usize) -> Result<usize, Error> {
    let max_run = if zeros_left >= 7 {
        14.min(zeros_left)
    } else {
        zeros_left
    };
    take_code(bits, 11, |len, peeked| {
        let mut found = None;
        for run in 0..=max_run {
            let (code, clen) = run_before(run, zeros_left);
            if clen as u32 != len || code != peeked {
                continue;
            }
            if found.is_some() {
                return None;
            }
            found = Some(run);
        }
        found
    })
}

/// Inverse of [`write_block`]. `maxn` is 16, 15, or 4.
pub(crate) fn read_block(
    bits: &mut BitReader<'_>,
    nc: i32,
    maxn: usize,
) -> Result<[i32; 16], Error> {
    let (total, trailing) = read_token(bits, nc)?;
    let mut scan = [0i32; 16];
    if total == 0 {
        return Ok(scan);
    }
    if total > maxn || trailing > total {
        return Err(Error::Bytes);
    }
    let mut high_to_low = [0i32; 16];
    for level in high_to_low.iter_mut().take(trailing) {
        *level = if bits.read_bit()? == 1 { -1 } else { 1 };
    }
    let nlevels = total - trailing;
    let mut suffix: i32 = if total > 10 && trailing < 3 { 1 } else { 0 };
    for i in 0..nlevels {
        let code = read_level_code(bits, suffix)?;
        let level = level_from_code(code, i == 0, trailing);
        high_to_low[trailing + i] = level;
        if suffix == 0 {
            suffix = 1;
        }
        if level.abs() > (3 << (suffix - 1)) && suffix < 6 {
            suffix += 1;
        }
    }
    let mut val = [0i32; 16];
    for i in 0..total {
        val[total - 1 - i] = high_to_low[i];
    }
    let mut pos = [0usize; 16];
    if total < maxn {
        let zeros = read_total_zeros(bits, total, maxn)?;
        let last = zeros + total - 1;
        if last >= maxn {
            return Err(Error::Bytes);
        }
        pos[total - 1] = last;
        let mut zeros_left = zeros;
        for k in (1..total).rev() {
            let run = if zeros_left > 0 {
                let run = read_run_before(bits, zeros_left)?;
                if run > zeros_left {
                    return Err(Error::Bytes);
                }
                zeros_left -= run;
                run
            } else {
                0
            };
            let prev = pos[k].checked_sub(run + 1).ok_or(Error::Bytes)?;
            pos[k - 1] = prev;
        }
        if zeros_left != pos[0] {
            return Err(Error::Bytes);
        }
    } else {
        for i in 0..total {
            pos[i] = i;
        }
    }
    for i in 0..total {
        if pos[i] >= maxn {
            return Err(Error::Bytes);
        }
        scan[pos[i]] = val[i];
    }
    Ok(scan)
}
