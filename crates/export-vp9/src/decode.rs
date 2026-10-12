use crate::bits::BitReader;
use crate::bool_coder::BoolDecoder;
use crate::probs::{
    CAT_PROBS, COEF_BAND_4X4, COEF_UV, COEF_Y, ENERGY, EXTRA_BITS, KF_PARTITION, KF_UV_MODE,
    KF_Y_MODE, PARETO, SCAN_4X4, SKIP_PROB,
};
use crate::{Error, Frame};

const BLOCK_8X8: usize = 3;
const BLOCK_16X16: usize = 6;
const BLOCK_32X32: usize = 9;
const BLOCK_64X64: usize = 12;
const DC_PRED: i32 = 0;
const PARTITION_NONE: i32 = 0;
const PARTITION_SPLIT: i32 = 3;

const INTRA_MODE_TREE: [i16; 18] = [
    0, 2, -9, 4, -1, 6, 8, 12, -2, 10, -4, -5, -3, 14, -8, 16, -6, -7,
];
const PARTITION_TREE: [i16; 6] = [0, 2, -1, 4, -2, -3];
const BINARY_TREE: [i16; 2] = [0, -1];
const TOKEN_TREE: [i16; 20] = [
    0, 2, -1, 4, 6, 10, -2, 8, -3, -4, 12, 14, -5, -6, 16, 18, -7, -8, -9, -10,
];

const MI_WIDTH_LOG2: [usize; 13] = [0, 0, 0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3];
const B_WIDTH_LOG2: [u32; 13] = [0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, 4, 4];
const B_HEIGHT_LOG2: [u32; 13] = [0, 1, 0, 1, 2, 1, 2, 3, 2, 3, 4, 3, 4];

struct Pic {
    width: usize,
    height: usize,
    mi: usize,
    y: Vec<u8>,
    u: Vec<u8>,
    v: Vec<u8>,
    above_nz: [[u8; 4]; 3],
    left_nz: [[u8; 4]; 3],
    skips: [u8; 4],
    above_part: [u8; 8],
    left_part: [u8; 8],
}

/// Decode the VP9 payload of one supported keyframe.
pub fn decode_keyframe(bytes: &[u8]) -> Result<Frame, Error> {
    if bytes.is_empty() {
        return Err(Error::Truncated);
    }
    if bytes.len() > 256 {
        return Err(Error::Unsupported);
    }
    if bytes[bytes.len() - 1] & 0xe0 == 0xc0 {
        return Err(Error::Unsupported);
    }

    let mut br = BitReader::new(bytes);
    if br.f(2)? != 2 {
        return Err(Error::BadHeader);
    }
    let profile = br.f(1)? | (br.f(1)? << 1);
    if profile == 3 && br.f(1)? != 0 {
        return Err(Error::BadHeader);
    }
    if profile != 0 {
        return Err(Error::Unsupported);
    }
    if br.f(1)? != 0 {
        return Err(Error::Unsupported);
    }
    if br.f(1)? != 0 {
        return Err(Error::Unsupported);
    }
    let _show_frame = br.f(1)?;
    let error_resilient = br.f(1)?;
    let sync = [br.f(8)? as u8, br.f(8)? as u8, br.f(8)? as u8];
    if sync != [0x49, 0x83, 0x42] {
        return Err(Error::BadHeader);
    }
    let color_space = br.f(3)?;
    if color_space != 7 {
        let _color_range = br.f(1)?;
    }
    let width = (br.f(16)? as usize).saturating_add(1);
    let height = (br.f(16)? as usize).saturating_add(1);
    if br.f(1)? != 0 {
        return Err(Error::Unsupported);
    }
    if !matches!((width, height), (8, 8) | (16, 16)) {
        return Err(Error::Unsupported);
    }
    if error_resilient == 0 {
        let _refresh_frame_context = br.f(1)?;
        let _frame_parallel = br.f(1)?;
    }
    let _frame_context = br.f(2)?;
    if br.f(6)? != 0 || br.f(3)? != 0 || br.f(1)? != 0 {
        return Err(Error::Unsupported);
    }
    if br.f(8)? != 0 || delta_q(&mut br)? != 0 || delta_q(&mut br)? != 0 || delta_q(&mut br)? != 0 {
        return Err(Error::Unsupported);
    }
    if br.f(1)? != 0 {
        return Err(Error::Unsupported);
    }
    if br.f(1)? != 0 {
        return Err(Error::Unsupported);
    }
    let header_size = br.f(16)? as usize;
    while br.bits_left() % 8 != 0 {
        if br.f(1)? != 0 {
            return Err(Error::BadHeader);
        }
    }
    if header_size == 0 {
        return Err(Error::BadHeader);
    }
    {
        let mut bools = BoolDecoder::new(&mut br, header_size)?;
        if bools.literal(1)? != 0 {
            return Err(Error::Unsupported);
        }
        for _ in 0..3 {
            if bools.read(252)? != 0 {
                return Err(Error::Unsupported);
            }
        }
        bools.exit()?;
    }
    let tile_bits = br.bits_left();
    if tile_bits == 0 || tile_bits % 8 != 0 {
        return Err(Error::Truncated);
    }
    let mut bools = BoolDecoder::new(&mut br, tile_bits / 8)?;
    let frame = decode_tile(&mut bools, width, height)?;
    bools.exit()?;
    if br.bits_left() != 0 {
        return Err(Error::BadHeader);
    }
    Ok(frame)
}

fn delta_q(br: &mut BitReader<'_>) -> Result<i32, Error> {
    if br.f(1)? == 0 {
        Ok(0)
    } else {
        Err(Error::Unsupported)
    }
}

fn decode_tile(
    bools: &mut BoolDecoder<'_, '_>,
    width: usize,
    height: usize,
) -> Result<Frame, Error> {
    let chroma = (width / 2) * (height / 2);
    let mut pic = Pic {
        width,
        height,
        mi: width / 8,
        y: vec![0; width * height],
        u: vec![0; chroma],
        v: vec![0; chroma],
        above_nz: [[0; 4]; 3],
        left_nz: [[0; 4]; 3],
        skips: [0; 4],
        above_part: [0; 8],
        left_part: [0; 8],
    };
    decode_partition(bools, &mut pic, 0, 0, BLOCK_64X64)?;
    Ok(Frame {
        width: pic.width,
        height: pic.height,
        y: pic.y,
        u: pic.u,
        v: pic.v,
    })
}

fn decode_partition(
    bools: &mut BoolDecoder<'_, '_>,
    pic: &mut Pic,
    row: usize,
    col: usize,
    bsize: usize,
) -> Result<(), Error> {
    if row >= pic.mi || col >= pic.mi {
        return Ok(());
    }
    let num8 = match bsize {
        BLOCK_64X64 => 8,
        BLOCK_32X32 => 4,
        BLOCK_16X16 => 2,
        BLOCK_8X8 => 1,
        _ => return Err(Error::Unsupported),
    };
    let half = num8 >> 1;
    let has_rows = row + half < pic.mi;
    let has_cols = col + half < pic.mi;
    let partition = if !has_rows && !has_cols {
        PARTITION_SPLIT
    } else if has_rows && has_cols {
        let ctx = partition_ctx(pic, bsize, row, col, num8)?;
        bools.tree(&PARTITION_TREE, |node| {
            if node >= 3 {
                128
            } else {
                KF_PARTITION[ctx][node]
            }
        })?
    } else {
        return Err(Error::Unsupported);
    };

    if bsize == BLOCK_8X8 {
        if partition != PARTITION_NONE {
            return Err(Error::Unsupported);
        }
        decode_block(bools, pic, row, col)?;
        update_partition(pic, row, col, BLOCK_8X8, num8)?;
        return Ok(());
    }
    if partition != PARTITION_SPLIT {
        return Err(Error::Unsupported);
    }
    let child = match bsize {
        BLOCK_64X64 => BLOCK_32X32,
        BLOCK_32X32 => BLOCK_16X16,
        BLOCK_16X16 => BLOCK_8X8,
        _ => return Err(Error::Unsupported),
    };
    decode_partition(bools, pic, row, col, child)?;
    decode_partition(bools, pic, row, col + half, child)?;
    decode_partition(bools, pic, row + half, col, child)?;
    decode_partition(bools, pic, row + half, col + half, child)?;
    Ok(())
}

fn partition_ctx(
    pic: &Pic,
    bsize: usize,
    row: usize,
    col: usize,
    num8: usize,
) -> Result<usize, Error> {
    let bsl = MI_WIDTH_LOG2[bsize];
    let boffset = MI_WIDTH_LOG2[BLOCK_64X64] - bsl;
    let mut above_acc = 0u8;
    let mut left_acc = 0u8;
    for i in 0..num8 {
        above_acc |= *pic.above_part.get(col + i).ok_or(Error::Unsupported)?;
        left_acc |= *pic.left_part.get(row + i).ok_or(Error::Unsupported)?;
    }
    let above_bit = u8::from(above_acc & (1 << boffset) != 0);
    let left_bit = u8::from(left_acc & (1 << boffset) != 0);
    Ok(bsl * 4 + usize::from(left_bit) * 2 + usize::from(above_bit))
}

fn update_partition(
    pic: &mut Pic,
    row: usize,
    col: usize,
    subsize: usize,
    num8: usize,
) -> Result<(), Error> {
    let above = 15u8 >> B_WIDTH_LOG2[subsize];
    let left = 15u8 >> B_HEIGHT_LOG2[subsize];
    for i in 0..num8 {
        *pic.above_part.get_mut(col + i).ok_or(Error::Unsupported)? = above;
        *pic.left_part.get_mut(row + i).ok_or(Error::Unsupported)? = left;
    }
    Ok(())
}

fn decode_block(
    bools: &mut BoolDecoder<'_, '_>,
    pic: &mut Pic,
    row: usize,
    col: usize,
) -> Result<(), Error> {
    let skip_ctx = neighbor_skip(pic, row, col)?;
    let skip = bools.tree(&BINARY_TREE, |_| SKIP_PROB[skip_ctx])?;
    *pic.skips
        .get_mut(row * pic.mi + col)
        .ok_or(Error::Unsupported)? = u8::from(skip != 0);
    let above_mode = DC_PRED;
    let left_mode = DC_PRED;
    let y_mode = bools.tree(&INTRA_MODE_TREE, |node| {
        KF_Y_MODE[above_mode as usize][left_mode as usize][node]
    })?;
    let uv_mode = bools.tree(&INTRA_MODE_TREE, |node| KF_UV_MODE[y_mode as usize][node])?;
    if y_mode != DC_PRED || uv_mode != DC_PRED {
        return Err(Error::Unsupported);
    }
    residual(bools, pic, row, col, skip != 0)?;
    Ok(())
}

fn neighbor_skip(pic: &Pic, row: usize, col: usize) -> Result<usize, Error> {
    let mut ctx = 0usize;
    if row > 0 {
        ctx += usize::from(
            *pic.skips
                .get((row - 1) * pic.mi + col)
                .ok_or(Error::Unsupported)?,
        );
    }
    if col > 0 {
        ctx += usize::from(
            *pic.skips
                .get(row * pic.mi + col - 1)
                .ok_or(Error::Unsupported)?,
        );
    }
    Ok(ctx)
}

fn residual(
    bools: &mut BoolDecoder<'_, '_>,
    pic: &mut Pic,
    row: usize,
    col: usize,
    skip: bool,
) -> Result<(), Error> {
    // Each 8x8 luma block is four 4x4 transforms. 4:2:0 chroma is one 4x4 each.
    let x0 = col * 8;
    let y0 = row * 8;
    let luma = [(x0, y0), (x0 + 4, y0), (x0, y0 + 4), (x0 + 4, y0 + 4)];
    for (x, y) in luma {
        transform(bools, pic, 0, x, y, skip)?;
    }
    transform(bools, pic, 1, col * 4, row * 4, skip)?;
    transform(bools, pic, 2, col * 4, row * 4, skip)?;
    Ok(())
}

fn transform(
    bools: &mut BoolDecoder<'_, '_>,
    pic: &mut Pic,
    plane: usize,
    x: usize,
    y: usize,
    skip: bool,
) -> Result<(), Error> {
    let stride = if plane == 0 { pic.width } else { pic.width / 2 };
    predict_dc(pic, plane, x, y);
    let mut nonzero = 0u8;
    if !skip {
        let mut coefs = [0i32; 16];
        nonzero = read_tokens(bools, plane, x, y, pic, &mut coefs)?;
        if nonzero != 0 {
            add_wht(pic, plane, x, y, stride, &coefs);
        }
    }
    let x4 = x >> 2;
    let y4 = y >> 2;
    if plane >= 3 || x4 >= 4 || y4 >= 4 {
        return Err(Error::Unsupported);
    }
    pic.above_nz[plane][x4] = nonzero;
    pic.left_nz[plane][y4] = nonzero;
    Ok(())
}

fn predict_dc(pic: &mut Pic, plane: usize, x: usize, y: usize) {
    let have_left = x > 0;
    let have_above = y > 0;
    let (stride, max_x, max_y) = if plane == 0 {
        (pic.width, pic.width - 1, pic.height - 1)
    } else {
        (pic.width / 2, pic.width / 2 - 1, pic.height / 2 - 1)
    };
    let size = 4usize;
    let mut above = [0i32; 4];
    let mut left = [0i32; 4];
    for i in 0..size {
        let above_x = (x + i).min(max_x);
        let left_y = (y + i).min(max_y);
        above[i] = if have_above {
            i32::from(px(pic, plane, y - 1, above_x))
        } else {
            127
        };
        left[i] = if have_left {
            i32::from(px(pic, plane, left_y, x - 1))
        } else {
            129
        };
    }
    let log2 = 2i32;
    let pred = if have_left && have_above {
        let mut sum = 0i32;
        for k in 0..size {
            sum += left[k] + above[k];
        }
        (sum + size as i32) >> (log2 + 1)
    } else if have_left {
        let sum: i32 = left.iter().sum();
        (sum + (1 << (log2 - 1))) >> log2
    } else if have_above {
        let sum: i32 = above.iter().sum();
        (sum + (1 << (log2 - 1))) >> log2
    } else {
        128
    };
    let value = pred.clamp(0, 255) as u8;
    for row in 0..size {
        for col in 0..size {
            set_px(pic, plane, y + row, x + col, stride, value);
        }
    }
}

fn px(pic: &Pic, plane: usize, y: usize, x: usize) -> u8 {
    let stride = if plane == 0 { pic.width } else { pic.width / 2 };
    let samples = match plane {
        0 => pic.y.as_slice(),
        1 => pic.u.as_slice(),
        _ => pic.v.as_slice(),
    };
    samples[y * stride + x]
}

fn set_px(pic: &mut Pic, plane: usize, y: usize, x: usize, stride: usize, value: u8) {
    let samples = match plane {
        0 => pic.y.as_mut_slice(),
        1 => pic.u.as_mut_slice(),
        _ => pic.v.as_mut_slice(),
    };
    samples[y * stride + x] = value;
}

fn read_tokens(
    bools: &mut BoolDecoder<'_, '_>,
    plane: usize,
    start_x: usize,
    start_y: usize,
    pic: &Pic,
    coefs: &mut [i32; 16],
) -> Result<u8, Error> {
    let probs = if plane == 0 { &COEF_Y } else { &COEF_UV };
    let mut cache = [0u8; 16];
    let mut check_eob = true;
    let mut c = 0usize;
    while c < 16 {
        let pos = SCAN_4X4[c];
        let band = COEF_BAND_4X4[c];
        let ctx = if c == 0 {
            let x4 = start_x >> 2;
            let y4 = start_y >> 2;
            usize::from(pic.above_nz[plane][x4] + pic.left_nz[plane][y4])
        } else {
            let nb = neighbors(pos);
            usize::from((1 + cache[nb[0]] + cache[nb[1]]) >> 1)
        };
        if ctx > 5 || band > 5 {
            return Err(Error::BadHeader);
        }
        if check_eob && bools.read(probs[band][ctx][0])? == 0 {
            break;
        }
        let token = bools.tree(&TOKEN_TREE, |node| token_prob(&probs[band][ctx], node))? as usize;
        if token >= ENERGY.len() {
            return Err(Error::BadHeader);
        }
        cache[pos] = ENERGY[token];
        if token == 0 {
            check_eob = false;
        } else {
            let mag = read_coef(bools, token)?;
            let sign = bools.read(128)?;
            coefs[pos] = if sign == 1 { -mag } else { mag };
            check_eob = true;
        }
        c += 1;
    }
    Ok(u8::from(c > 0))
}

fn neighbors(pos: usize) -> [usize; 2] {
    let n = 4usize;
    let i = pos / n;
    let j = pos % n;
    if i > 0 && j > 0 {
        [(i - 1) * n + j, i * n + j - 1]
    } else if i > 0 {
        let a = (i - 1) * n + j;
        [a, a]
    } else {
        let a = j - 1;
        [a, a]
    }
}

fn token_prob(probs: &[u8; 3], node: usize) -> u8 {
    let index = (1 + node).min(2);
    pareto(node, probs[index])
}

fn pareto(node: usize, prob: u8) -> u8 {
    if node < 2 {
        return prob;
    }
    let x = (usize::from(prob) - 1) / 2;
    let column = node - 2;
    if column >= 8 || x >= PARETO.len() {
        return prob;
    }
    if prob & 1 == 1 {
        PARETO[x][column]
    } else if x + 1 < PARETO.len() {
        ((u16::from(PARETO[x][column]) + u16::from(PARETO[x + 1][column])) >> 1) as u8
    } else {
        PARETO[x][column]
    }
}

fn read_coef(bools: &mut BoolDecoder<'_, '_>, token: usize) -> Result<i32, Error> {
    let (cat, extra, mut coef) = EXTRA_BITS[token];
    let cat_probs = CAT_PROBS[usize::from(cat)];
    for (e, prob) in cat_probs.iter().enumerate().take(usize::from(extra)) {
        let bit = bools.read(*prob)?;
        coef += i32::from(bit) << (i32::from(extra) - 1 - e as i32);
    }
    Ok(coef)
}

fn add_wht(pic: &mut Pic, plane: usize, x: usize, y: usize, stride: usize, tokens: &[i32; 16]) {
    let mut rows = [[0i32; 4]; 4];
    for (i, row) in rows.iter_mut().enumerate() {
        for (j, coef) in row.iter_mut().enumerate() {
            *coef = tokens[i * 4 + j] * 4;
        }
    }
    rows[0][0] = tokens[0] * 4;
    for row in &mut rows {
        iwht4(row, 2);
    }
    for j in 0..4 {
        let mut col = [rows[0][j], rows[1][j], rows[2][j], rows[3][j]];
        iwht4(&mut col, 0);
        for i in 0..4 {
            rows[i][j] = col[i];
        }
    }
    for (i, row) in rows.iter().enumerate() {
        for (j, residual) in row.iter().enumerate() {
            let index = (y + i) * stride + (x + j);
            let samples = match plane {
                0 => pic.y.as_mut_slice(),
                1 => pic.u.as_mut_slice(),
                _ => pic.v.as_mut_slice(),
            };
            let value = i32::from(samples[index]) + residual;
            samples[index] = value.clamp(0, 255) as u8;
        }
    }
}

fn iwht4(t: &mut [i32; 4], shift: i32) {
    let mut a = t[0] >> shift;
    let mut c = t[1] >> shift;
    let mut d = t[2] >> shift;
    let mut b = t[3] >> shift;
    a += c;
    d -= b;
    let e = (a - d) >> 1;
    b = e - b;
    c = e - c;
    a -= b;
    d += c;
    t[0] = a;
    t[1] = b;
    t[2] = c;
    t[3] = d;
}
