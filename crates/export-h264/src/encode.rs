use crate::bitstream::{nal_unit, BitWriter};
use crate::cavlc::{inter_cbp_code, intra_cbp_code, write_block};
use crate::error::Error;
use crate::Yuv420;

const ZIGZAG: [usize; 16] = [0, 1, 4, 8, 5, 2, 3, 6, 9, 12, 13, 10, 7, 11, 14, 15];
const BLK_X: [usize; 16] = [0, 1, 0, 1, 2, 3, 2, 3, 0, 1, 0, 1, 2, 3, 2, 3];
const BLK_Y: [usize; 16] = [0, 0, 1, 1, 0, 0, 1, 1, 2, 2, 3, 3, 2, 2, 3, 3];

const MF: [[i32; 6]; 3] = [
    [13107, 11916, 10082, 9362, 8192, 7282],
    [8066, 7490, 6554, 5825, 5243, 4559],
    [5243, 4660, 4194, 3647, 3355, 2893],
];
const DEQ: [[i32; 6]; 3] = [
    [10, 11, 13, 14, 16, 18],
    [13, 14, 16, 18, 20, 23],
    [16, 18, 20, 23, 25, 29],
];

fn pos_class(i: usize, j: usize) -> usize {
    if (i & 1) == 0 && (j & 1) == 0 {
        0
    } else if (i & 1) == 1 && (j & 1) == 1 {
        2
    } else {
        1
    }
}

fn fwd4(r: &[[i32; 4]; 4]) -> [[i32; 4]; 4] {
    let mut tmp = [[0i32; 4]; 4];
    for i in 0..4 {
        let (a, b, c, d) = row_fwd(r[i][0], r[i][1], r[i][2], r[i][3]);
        tmp[i] = [a, b, c, d];
    }
    let mut out = [[0i32; 4]; 4];
    for j in 0..4 {
        let (a, b, c, d) = row_fwd(tmp[0][j], tmp[1][j], tmp[2][j], tmp[3][j]);
        out[0][j] = a;
        out[1][j] = b;
        out[2][j] = c;
        out[3][j] = d;
    }
    out
}

fn row_fwd(d0: i32, d1: i32, d2: i32, d3: i32) -> (i32, i32, i32, i32) {
    let t0 = d0 + d3;
    let t1 = d0 - d3;
    let t2 = d1 + d2;
    let t3 = d1 - d2;
    (t0 + t2, (t1 << 1) + t3, t0 - t2, t1 - (t3 << 1))
}

fn quant4(w: &[[i32; 4]; 4], qp: i32) -> [[i32; 4]; 4] {
    let qbits = 15 + qp / 6;
    let f = (1i32 << qbits) / 3;
    let m = (qp as usize) % 6;
    let mut z = [[0i32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let mf = MF[pos_class(i, j)][m];
            let v = w[i][j];
            let mag = (v.abs() * mf + f) >> qbits;
            let mag = mag.min(2063);
            z[i][j] = if v < 0 { -mag } else { mag };
        }
    }
    z
}

fn dequant4(z: &[[i32; 4]; 4], qp: i32) -> [[i32; 4]; 4] {
    let shift = qp / 6;
    let m = (qp as usize) % 6;
    let mut d = [[0i32; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            let v = DEQ[pos_class(i, j)][m];
            d[i][j] = (z[i][j] * v) << shift;
        }
    }
    d
}

fn idct4(d: &[[i32; 4]; 4]) -> [[i32; 4]; 4] {
    let mut tmp = [[0i32; 4]; 4];
    for i in 0..4 {
        let e0 = d[i][0] + d[i][2];
        let e1 = d[i][0] - d[i][2];
        let e2 = (d[i][1] >> 1) - d[i][3];
        let e3 = d[i][1] + (d[i][3] >> 1);
        tmp[i][0] = e0 + e3;
        tmp[i][1] = e1 + e2;
        tmp[i][2] = e1 - e2;
        tmp[i][3] = e0 - e3;
    }
    let mut out = [[0i32; 4]; 4];
    for j in 0..4 {
        let g0 = tmp[0][j] + tmp[2][j];
        let g1 = tmp[0][j] - tmp[2][j];
        let g2 = (tmp[1][j] >> 1) - tmp[3][j];
        let g3 = tmp[1][j] + (tmp[3][j] >> 1);
        out[0][j] = (g0 + g3 + 32) >> 6;
        out[1][j] = (g1 + g2 + 32) >> 6;
        out[2][j] = (g1 - g2 + 32) >> 6;
        out[3][j] = (g0 - g3 + 32) >> 6;
    }
    out
}

fn to_scan(z: &[[i32; 4]; 4]) -> [i32; 16] {
    let mut s = [0i32; 16];
    for (si, &ri) in ZIGZAG.iter().enumerate() {
        s[si] = z[ri / 4][ri % 4];
    }
    s
}

fn clip8(v: i32) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

fn hadamard2(x: [i32; 4]) -> [i32; 4] {
    let a = x[0];
    let b = x[1];
    let c = x[2];
    let d = x[3];
    [a + b + c + d, a - b + c - d, a + b - c - d, a - b - c + d]
}

fn quant_chroma_dc(dcs: [i32; 4], qp: i32) -> [i32; 4] {
    let h = hadamard2(dcs);
    let qbits = 16 + qp / 6;
    let f = (1i32 << qbits) / 3;
    let mf = MF[0][(qp as usize) % 6];
    let mut z = [0i32; 4];
    for i in 0..4 {
        let v = h[i];
        let mag = ((v.abs() * mf + f) >> qbits).min(2063);
        z[i] = if v < 0 { -mag } else { mag };
    }
    z
}

fn dequant_chroma_dc(z: [i32; 4], qp: i32) -> [i32; 4] {
    let h = hadamard2(z);
    let v = DEQ[0][(qp as usize) % 6];
    let mut out = [0i32; 4];
    for i in 0..4 {
        out[i] = if qp >= 6 {
            (h[i] * v) << (qp / 6 - 1)
        } else {
            (h[i] * v) >> 1
        };
    }
    out
}

fn pred_luma_dc(rec: &[u8], stride: usize, x: usize, y: usize) -> [u8; 16] {
    let left = x > 0;
    let top = y > 0;
    let sample = if left && top {
        let mut s = 0i32;
        for i in 0..4 {
            s += rec[(y + i) * stride + (x - 1)] as i32;
            s += rec[(y - 1) * stride + (x + i)] as i32;
        }
        ((s + 4) >> 3) as u8
    } else if left {
        let mut s = 0i32;
        for i in 0..4 {
            s += rec[(y + i) * stride + (x - 1)] as i32;
        }
        ((s + 2) >> 2) as u8
    } else if top {
        let mut s = 0i32;
        for i in 0..4 {
            s += rec[(y - 1) * stride + (x + i)] as i32;
        }
        ((s + 2) >> 2) as u8
    } else {
        128
    };
    [sample; 16]
}

fn pred_chroma_dc(rec: &[u8], stride: usize, x: usize, y: usize) -> [u8; 64] {
    let left = x > 0;
    let top = y > 0;
    let blk = if left && top {
        let mut s0 = 0i32;
        let mut s1 = 0i32;
        let mut s2 = 0i32;
        let mut s3 = 0i32;
        for i in 0..4 {
            s0 += rec[(y - 1) * stride + x + i] as i32;
            s1 += rec[(y - 1) * stride + x + 4 + i] as i32;
            s2 += rec[(y + i) * stride + (x - 1)] as i32;
            s3 += rec[(y + 4 + i) * stride + (x - 1)] as i32;
        }
        [
            ((s0 + s2 + 4) >> 3) as u8,
            ((s1 + 2) >> 2) as u8,
            ((s3 + 2) >> 2) as u8,
            ((s1 + s3 + 4) >> 3) as u8,
        ]
    } else if left {
        let mut s2 = 0i32;
        let mut s3 = 0i32;
        for i in 0..4 {
            s2 += rec[(y + i) * stride + (x - 1)] as i32;
            s3 += rec[(y + 4 + i) * stride + (x - 1)] as i32;
        }
        let a = ((s2 + 2) >> 2) as u8;
        let b = ((s3 + 2) >> 2) as u8;
        [a, a, b, b]
    } else if top {
        let mut s0 = 0i32;
        let mut s1 = 0i32;
        for i in 0..4 {
            s0 += rec[(y - 1) * stride + x + i] as i32;
            s1 += rec[(y - 1) * stride + x + 4 + i] as i32;
        }
        let a = ((s0 + 2) >> 2) as u8;
        let b = ((s1 + 2) >> 2) as u8;
        [a, b, a, b]
    } else {
        [128u8; 4]
    };
    let mut p = [0u8; 64];
    for by in 0..2 {
        for bx in 0..2 {
            let sample = blk[by * 2 + bx];
            for r in 0..4 {
                for c in 0..4 {
                    p[(by * 4 + r) * 8 + bx * 4 + c] = sample;
                }
            }
        }
    }
    p
}

#[derive(Clone, Copy)]
struct LumaBlock {
    scan: [i32; 16],
    total: u8,
}

#[derive(Clone, Copy)]
struct ChromaMb {
    dc: [i32; 4],
    ac: [[i32; 15]; 4],
    ac_total: [u8; 4],
}

fn nnz_of(scan: &[i32]) -> u8 {
    scan.iter().filter(|c| **c != 0).count() as u8
}

struct Plane<'a> {
    src: &'a [u8],
    rec: &'a mut [u8],
    stride: usize,
    nnz: &'a mut [u8],
    nnz_stride: usize,
}

fn encode_luma_mb(plane: &mut Plane<'_>, mb_x: usize, mb_y: usize, qp: i32) -> [LumaBlock; 16] {
    let mut blocks = [LumaBlock {
        scan: [0; 16],
        total: 0,
    }; 16];
    for i in 0..16 {
        let x = mb_x * 16 + BLK_X[i] * 4;
        let y = mb_y * 16 + BLK_Y[i] * 4;
        let pred = pred_luma_dc(plane.rec, plane.stride, x, y);
        let mut residual = [[0i32; 4]; 4];
        for (r, row) in residual.iter_mut().enumerate() {
            for (c, sample) in row.iter_mut().enumerate() {
                let s = plane.src[(y + r) * plane.stride + x + c] as i32;
                *sample = s - pred[r * 4 + c] as i32;
            }
        }
        let w = fwd4(&residual);
        let z = quant4(&w, qp);
        let scan = to_scan(&z);
        let dq = dequant4(&z, qp);
        let idct = idct4(&dq);
        for (r, row) in idct.iter().enumerate() {
            for (c, coef) in row.iter().enumerate() {
                let pix = clip8(pred[r * 4 + c] as i32 + coef);
                plane.rec[(y + r) * plane.stride + x + c] = pix;
            }
        }
        let bx = mb_x * 4 + BLK_X[i];
        let by = mb_y * 4 + BLK_Y[i];
        plane.nnz[by * plane.nnz_stride + bx] = nnz_of(&scan);
        blocks[i] = LumaBlock {
            scan,
            total: plane.nnz[by * plane.nnz_stride + bx],
        };
    }
    blocks
}

fn encode_chroma_mb(plane: &mut Plane<'_>, mb_x: usize, mb_y: usize, qp: i32) -> ChromaMb {
    let x0 = mb_x * 8;
    let y0 = mb_y * 8;
    let pred = pred_chroma_dc(plane.rec, plane.stride, x0, y0);
    let mut dcs = [0i32; 4];
    for (i, dc) in dcs.iter_mut().enumerate() {
        let bx = i & 1;
        let by = i >> 1;
        let mut residual = [[0i32; 4]; 4];
        for (r, row) in residual.iter_mut().enumerate() {
            for (c, sample) in row.iter_mut().enumerate() {
                let py = by * 4 + r;
                let px = bx * 4 + c;
                let s = plane.src[(y0 + py) * plane.stride + x0 + px] as i32;
                *sample = s - pred[py * 8 + px] as i32;
            }
        }
        *dc = fwd4(&residual)[0][0];
    }
    // Chroma AC is not sent. Only the DC coefficient is quantized at this QP.
    let dc_q = quant_chroma_dc(dcs, qp);
    let dc_r = dequant_chroma_dc(dc_q, qp);
    let ac = [[0i32; 15]; 4];
    let ac_total = [0u8; 4];
    for (i, coef) in dc_r.iter().enumerate() {
        let mut dq = [[0i32; 4]; 4];
        dq[0][0] = *coef;
        let idct = idct4(&dq);
        let bx = i & 1;
        let by = i >> 1;
        for (r, row) in idct.iter().enumerate() {
            for (c, coef) in row.iter().enumerate() {
                let py = by * 4 + r;
                let px = bx * 4 + c;
                let pix = clip8(pred[py * 8 + px] as i32 + coef);
                plane.rec[(y0 + py) * plane.stride + x0 + px] = pix;
            }
        }
        let cx = mb_x * 2 + bx;
        let cy = mb_y * 2 + by;
        plane.nnz[cy * plane.nnz_stride + cx] = 0;
    }
    ChromaMb {
        dc: dc_q,
        ac,
        ac_total,
    }
}

fn nc_at(nnz: &[u8], stride: usize, x: usize, y: usize) -> i32 {
    let left = if x > 0 { Some(nnz[y * stride + x - 1] as i32) } else { None };
    let up = if y > 0 { Some(nnz[(y - 1) * stride + x] as i32) } else { None };
    match (left, up) {
        (Some(a), Some(b)) => (a + b + 1) >> 1,
        (Some(a), None) => a,
        (None, Some(b)) => b,
        (None, None) => 0,
    }
}

fn write_sps(width: u32, height: u32) -> Vec<u8> {
    let mut b = BitWriter::new();
    b.write_bits(66, 8);
    b.write_bit(1);
    b.write_bit(1);
    b.write_bits(0, 6);
    b.write_bits(10, 8);
    b.write_ue(0);
    b.write_ue(0);
    b.write_ue(2);
    b.write_ue(1);
    b.write_bit(0);
    b.write_ue(width / 16 - 1);
    b.write_ue(height / 16 - 1);
    b.write_bit(1);
    b.write_bit(1);
    b.write_bit(0);
    b.write_bit(0);
    b.rbsp_trailing();
    nal_unit(3, 7, &b.into_bytes())
}

fn write_pps(qp: i32) -> Vec<u8> {
    let mut b = BitWriter::new();
    b.write_ue(0);
    b.write_ue(0);
    b.write_bit(0);
    b.write_bit(0);
    b.write_ue(0);
    b.write_ue(0);
    b.write_ue(0);
    b.write_bit(0);
    b.write_bits(0, 2);
    b.write_se(qp - 26);
    b.write_se(0);
    b.write_se(0);
    b.write_bit(1);
    b.write_bit(0);
    b.write_bit(0);
    b.rbsp_trailing();
    nal_unit(3, 8, &b.into_bytes())
}

fn write_aud() -> Vec<u8> {
    nal_unit(0, 9, &[0x10])
}

struct Slice<'a> {
    width: u32,
    height: u32,
    luma: &'a [LumaBlock],
    chroma_u: &'a [ChromaMb],
    chroma_v: &'a [ChromaMb],
    luma_nnz: &'a [u8],
    u_nnz: &'a [u8],
    v_nnz: &'a [u8],
}

fn write_slice(slice: &Slice<'_>) -> Vec<u8> {
    let mb_w = (slice.width / 16) as usize;
    let mb_h = (slice.height / 16) as usize;
    let mut b = BitWriter::new();
    b.write_ue(0);
    b.write_ue(7);
    b.write_ue(0);
    b.write_bits(0, 4);
    b.write_ue(0);
    b.write_bit(0);
    b.write_bit(0);
    b.write_se(0);
    b.write_ue(1);

    for my in 0..mb_h {
        for mx in 0..mb_w {
            let mb = my * mb_w + mx;
            let blocks = &slice.luma[mb * 16..(mb + 1) * 16];
            b.write_ue(0);
            // Every luma block is intra DC, which is also the predicted mode
            // when a neighbor is missing and when both neighbors are DC.
            for _ in 0..16 {
                b.write_bit(1);
            }
            b.write_ue(0);

            let mut cbpl = 0u32;
            for i8 in 0..4 {
                let nz = (0..4).any(|k| blocks[i8 * 4 + k].total != 0);
                if nz {
                    cbpl |= 1 << i8;
                }
            }
            let cu = &slice.chroma_u[mb];
            let cv = &slice.chroma_v[mb];
            let ac = cu.ac_total.iter().any(|&t| t != 0) || cv.ac_total.iter().any(|&t| t != 0);
            let dc = cu.dc.iter().any(|&c| c != 0) || cv.dc.iter().any(|&c| c != 0);
            let cbpc = if ac { 2 } else if dc { 1 } else { 0 };
            let cbp = (cbpl as usize) + (cbpc << 4);
            b.write_ue(intra_cbp_code(cbp));
            if cbp != 0 {
                b.write_se(0);
            }
            if cbpl != 0 {
                for i8 in 0..4 {
                    if (cbpl & (1 << i8)) == 0 {
                        continue;
                    }
                    for k in 0..4 {
                        let i = i8 * 4 + k;
                        let bx = mx * 4 + BLK_X[i];
                        let by = my * 4 + BLK_Y[i];
                        let nc = nc_at(slice.luma_nnz, mb_w * 4, bx, by);
                        write_block(&mut b, &blocks[i].scan, nc);
                    }
                }
            }
            if cbpc > 0 {
                write_block(&mut b, &cu.dc, -1);
                write_block(&mut b, &cv.dc, -1);
            }
            if cbpc == 2 {
                for (plane, nz_plane) in [(cu, slice.u_nnz), (cv, slice.v_nnz)] {
                    for i in 0..4 {
                        let bx = mx * 2 + (i & 1);
                        let by = my * 2 + (i >> 1);
                        let nc = nc_at(nz_plane, mb_w * 2, bx, by);
                        write_block(&mut b, &plane.ac[i], nc);
                    }
                }
            }
        }
    }
    b.rbsp_trailing();
    nal_unit(3, 5, &b.into_bytes())
}

pub fn encode_iframe(frame: &Yuv420, qp: u8) -> Result<(Vec<u8>, Yuv420), Error> {
    let width = frame.width;
    let height = frame.height;
    let qp_i = qp as i32;
    let mb_w = (width / 16) as usize;
    let mb_h = (height / 16) as usize;
    let mut rec_y = vec![0u8; frame.y.len()];
    let mut rec_u = vec![0u8; frame.u.len()];
    let mut rec_v = vec![0u8; frame.v.len()];
    let mut luma_nnz = vec![0u8; mb_w * 4 * mb_h * 4];
    let mut u_nnz = vec![0u8; mb_w * 2 * mb_h * 2];
    let mut v_nnz = vec![0u8; mb_w * 2 * mb_h * 2];
    let mut luma_blocks = Vec::with_capacity(mb_w * mb_h * 16);
    let mut chroma_u = Vec::with_capacity(mb_w * mb_h);
    let mut chroma_v = Vec::with_capacity(mb_w * mb_h);
    let y_stride = width as usize;
    let c_stride = (width / 2) as usize;
    for my in 0..mb_h {
        for mx in 0..mb_w {
            let lb = encode_luma_mb(
                &mut Plane {
                    src: &frame.y,
                    rec: &mut rec_y,
                    stride: y_stride,
                    nnz: &mut luma_nnz,
                    nnz_stride: mb_w * 4,
                },
                mx,
                my,
                qp_i,
            );
            luma_blocks.extend_from_slice(&lb);
            chroma_u.push(encode_chroma_mb(
                &mut Plane {
                    src: &frame.u,
                    rec: &mut rec_u,
                    stride: c_stride,
                    nnz: &mut u_nnz,
                    nnz_stride: mb_w * 2,
                },
                mx,
                my,
                qp_i,
            ));
            chroma_v.push(encode_chroma_mb(
                &mut Plane {
                    src: &frame.v,
                    rec: &mut rec_v,
                    stride: c_stride,
                    nnz: &mut v_nnz,
                    nnz_stride: mb_w * 2,
                },
                mx,
                my,
                qp_i,
            ));
        }
    }
    let mut out = Vec::new();
    out.extend(write_aud());
    out.extend(write_sps(width, height));
    out.extend(write_pps(qp_i));
    out.extend(write_slice(&Slice {
        width,
        height,
        luma: &luma_blocks,
        chroma_u: &chroma_u,
        chroma_v: &chroma_v,
        luma_nnz: &luma_nnz,
        u_nnz: &u_nnz,
        v_nnz: &v_nnz,
    }));
    let reconstructed = Yuv420 {
        width,
        height,
        y: rec_y,
        u: rec_u,
        v: rec_v,
    };
    Ok((out, reconstructed))
}

struct Inter<'a> {
    src: &'a [u8],
    reference: &'a [u8],
    rec: &'a mut [u8],
    stride: usize,
    nnz: &'a mut [u8],
    nnz_stride: usize,
}

fn encode_inter_luma(plane: &mut Inter<'_>, mb_x: usize, mb_y: usize, qp: i32) -> [LumaBlock; 16] {
    let mut blocks = [LumaBlock {
        scan: [0; 16],
        total: 0,
    }; 16];
    for i in 0..16 {
        let x = mb_x * 16 + BLK_X[i] * 4;
        let y = mb_y * 16 + BLK_Y[i] * 4;
        let mut residual = [[0i32; 4]; 4];
        for (r, row) in residual.iter_mut().enumerate() {
            for (c, sample) in row.iter_mut().enumerate() {
                let at = (y + r) * plane.stride + x + c;
                *sample = plane.src[at] as i32 - plane.reference[at] as i32;
            }
        }
        let z = quant4(&fwd4(&residual), qp);
        let scan = to_scan(&z);
        let idct = idct4(&dequant4(&z, qp));
        for (r, row) in idct.iter().enumerate() {
            for (c, coef) in row.iter().enumerate() {
                let at = (y + r) * plane.stride + x + c;
                plane.rec[at] = clip8(plane.reference[at] as i32 + coef);
            }
        }
        let bx = mb_x * 4 + BLK_X[i];
        let by = mb_y * 4 + BLK_Y[i];
        let total = nnz_of(&scan);
        plane.nnz[by * plane.nnz_stride + bx] = total;
        blocks[i] = LumaBlock { scan, total };
    }
    blocks
}

fn encode_inter_chroma(plane: &mut Inter<'_>, mb_x: usize, mb_y: usize, qp: i32) -> ChromaMb {
    let x0 = mb_x * 8;
    let y0 = mb_y * 8;
    let mut dcs = [0i32; 4];
    for (i, dc) in dcs.iter_mut().enumerate() {
        let bx = i & 1;
        let by = i >> 1;
        let mut residual = [[0i32; 4]; 4];
        for (r, row) in residual.iter_mut().enumerate() {
            for (c, sample) in row.iter_mut().enumerate() {
                let at = (y0 + by * 4 + r) * plane.stride + x0 + bx * 4 + c;
                *sample = plane.src[at] as i32 - plane.reference[at] as i32;
            }
        }
        *dc = fwd4(&residual)[0][0];
    }
    let dc_q = quant_chroma_dc(dcs, qp);
    let dc_r = dequant_chroma_dc(dc_q, qp);
    for (i, coef) in dc_r.iter().enumerate() {
        let mut dq = [[0i32; 4]; 4];
        dq[0][0] = *coef;
        let idct = idct4(&dq);
        let bx = i & 1;
        let by = i >> 1;
        for (r, row) in idct.iter().enumerate() {
            for (c, add) in row.iter().enumerate() {
                let at = (y0 + by * 4 + r) * plane.stride + x0 + bx * 4 + c;
                plane.rec[at] = clip8(plane.reference[at] as i32 + add);
            }
        }
        let cx = mb_x * 2 + bx;
        let cy = mb_y * 2 + by;
        plane.nnz[cy * plane.nnz_stride + cx] = 0;
    }
    ChromaMb {
        dc: dc_q,
        ac: [[0; 15]; 4],
        ac_total: [0; 4],
    }
}

fn write_p_slice(slice: &Slice<'_>) -> Vec<u8> {
    let mb_w = (slice.width / 16) as usize;
    let mb_h = (slice.height / 16) as usize;
    let mut b = BitWriter::new();
    b.write_ue(0);
    b.write_ue(0);
    b.write_ue(0);
    b.write_bits(1, 4);
    b.write_bit(0);
    b.write_bit(0);
    b.write_se(0);
    b.write_ue(1);
    for my in 0..mb_h {
        for mx in 0..mb_w {
            let mb = my * mb_w + mx;
            let blocks = &slice.luma[mb * 16..(mb + 1) * 16];
            b.write_ue(0);
            b.write_ue(0);
            b.write_se(0);
            b.write_se(0);
            let mut cbpl = 0u32;
            for i8 in 0..4 {
                if (0..4).any(|k| blocks[i8 * 4 + k].total != 0) {
                    cbpl |= 1 << i8;
                }
            }
            let cu = &slice.chroma_u[mb];
            let cv = &slice.chroma_v[mb];
            let dc = cu.dc.iter().any(|&c| c != 0) || cv.dc.iter().any(|&c| c != 0);
            let cbpc = if dc { 1 } else { 0 };
            let cbp = (cbpl as usize) + (cbpc << 4);
            b.write_ue(inter_cbp_code(cbp));
            if cbp != 0 {
                b.write_se(0);
            }
            if cbpl != 0 {
                for i8 in 0..4 {
                    if (cbpl & (1 << i8)) == 0 {
                        continue;
                    }
                    for k in 0..4 {
                        let i = i8 * 4 + k;
                        let bx = mx * 4 + BLK_X[i];
                        let by = my * 4 + BLK_Y[i];
                        let nc = nc_at(slice.luma_nnz, mb_w * 4, bx, by);
                        write_block(&mut b, &blocks[i].scan, nc);
                    }
                }
            }
            if cbpc > 0 {
                write_block(&mut b, &cu.dc, -1);
                write_block(&mut b, &cv.dc, -1);
            }
        }
    }
    b.rbsp_trailing();
    // nal_ref_idc 0: this P picture is not itself a reference.
    nal_unit(0, 1, &b.into_bytes())
}

/// One P slice after an already-reconstructed I-frame. Motion is zero.
pub fn encode_pframe(source: &Yuv420, reference: &Yuv420, qp: u8) -> Result<(Vec<u8>, Yuv420), Error> {
    if source.width != reference.width || source.height != reference.height {
        return Err(Error::Shape);
    }
    let width = source.width;
    let height = source.height;
    let qp_i = qp as i32;
    let mb_w = (width / 16) as usize;
    let mb_h = (height / 16) as usize;
    let mut rec_y = vec![0u8; source.y.len()];
    let mut rec_u = vec![0u8; source.u.len()];
    let mut rec_v = vec![0u8; source.v.len()];
    let mut luma_nnz = vec![0u8; mb_w * 4 * mb_h * 4];
    let mut u_nnz = vec![0u8; mb_w * 2 * mb_h * 2];
    let mut v_nnz = vec![0u8; mb_w * 2 * mb_h * 2];
    let mut luma_blocks = Vec::with_capacity(mb_w * mb_h * 16);
    let mut chroma_u = Vec::with_capacity(mb_w * mb_h);
    let mut chroma_v = Vec::with_capacity(mb_w * mb_h);
    let y_stride = width as usize;
    let c_stride = (width / 2) as usize;
    for my in 0..mb_h {
        for mx in 0..mb_w {
            let lb = encode_inter_luma(
                &mut Inter {
                    src: &source.y,
                    reference: &reference.y,
                    rec: &mut rec_y,
                    stride: y_stride,
                    nnz: &mut luma_nnz,
                    nnz_stride: mb_w * 4,
                },
                mx,
                my,
                qp_i,
            );
            luma_blocks.extend_from_slice(&lb);
            chroma_u.push(encode_inter_chroma(
                &mut Inter {
                    src: &source.u,
                    reference: &reference.u,
                    rec: &mut rec_u,
                    stride: c_stride,
                    nnz: &mut u_nnz,
                    nnz_stride: mb_w * 2,
                },
                mx,
                my,
                qp_i,
            ));
            chroma_v.push(encode_inter_chroma(
                &mut Inter {
                    src: &source.v,
                    reference: &reference.v,
                    rec: &mut rec_v,
                    stride: c_stride,
                    nnz: &mut v_nnz,
                    nnz_stride: mb_w * 2,
                },
                mx,
                my,
                qp_i,
            ));
        }
    }
    let mut out = nal_unit(0, 9, &[0x30]);
    out.extend(write_p_slice(&Slice {
        width,
        height,
        luma: &luma_blocks,
        chroma_u: &chroma_u,
        chroma_v: &chroma_v,
        luma_nnz: &luma_nnz,
        u_nnz: &u_nnz,
        v_nnz: &v_nnz,
    }));
    Ok((
        out,
        Yuv420 {
            width,
            height,
            y: rec_y,
            u: rec_u,
            v: rec_v,
        },
    ))
}
