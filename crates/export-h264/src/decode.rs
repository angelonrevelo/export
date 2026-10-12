//! Read one I-frame this encoder wrote. Other bitstreams are refused.

use crate::bitstream::BitReader;
use crate::cavlc::{inter_cbp_from_code, intra_cbp_from_code, read_block};
use crate::encode::{
    clip8, dequant4, dequant_chroma_dc, idct4, mvp_16x16, nc_at, pred_chroma_dc, pred_luma_dc,
    BLK_X, BLK_Y, ZIGZAG,
};
use crate::error::Error;
use crate::Yuv420;

/// Rebuild the reconstruction by reading one I-frame Annex B from this encoder.
///
/// The bytes are parsed. A stored picture is not consulted. A P-frame, a
/// B-frame, and any other bitstream are refused.
pub fn decode_iframe(annex_b: &[u8]) -> Result<Yuv420, Error> {
    let nals = split_nals(annex_b)?;
    if nals.len() != 4 {
        return Err(Error::Bytes);
    }
    let (aud_h, aud) = &nals[0];
    let (sps_h, sps) = &nals[1];
    let (pps_h, pps) = &nals[2];
    let (idr_h, idr) = &nals[3];
    if *aud_h != 0x09 || aud.as_slice() != [0x10] {
        return Err(Error::Bytes);
    }
    if *sps_h != 0x67 || *pps_h != 0x68 || *idr_h != 0x65 {
        return Err(Error::Bytes);
    }
    let (width, height, frame_bits) = read_sps(sps)?;
    let qp = read_pps(pps)?;
    read_slice(idr, width, height, frame_bits, qp)
}

/// Read the 415-byte I-frame plus zero-motion P-frame this encoder writes.
///
/// Both pictures are rebuilt from the Annex B bytes. A stored buffer is not
/// returned. Non-zero motion, a B-frame, and any other bitstream are refused.
pub fn decode_pair(annex_b: &[u8]) -> Result<(Yuv420, Yuv420), Error> {
    let nals = split_nals(annex_b)?;
    if nals.len() != 6 {
        return Err(Error::Bytes);
    }
    let (aud_h, aud) = &nals[0];
    let (sps_h, sps) = &nals[1];
    let (pps_h, pps) = &nals[2];
    let (idr_h, idr) = &nals[3];
    if *aud_h != 0x09 || aud.as_slice() != [0x10] {
        return Err(Error::Bytes);
    }
    if *sps_h != 0x67 || *pps_h != 0x68 || *idr_h != 0x65 {
        return Err(Error::Bytes);
    }
    let (width, height, frame_bits) = read_sps(sps)?;
    let qp = read_pps(pps)?;
    let intra = read_slice(idr, width, height, frame_bits, qp)?;
    let (p_aud_h, p_aud) = &nals[4];
    if *p_aud_h != 0x09 || p_aud.as_slice() != [0x30] {
        return Err(Error::Bytes);
    }
    let (p_h, p_rbsp) = &nals[5];
    if *p_h != 0x01 {
        return Err(Error::Bytes);
    }
    let predicted = read_zero_p_slice(p_rbsp, &intra, frame_bits, qp)?;
    Ok((intra, predicted))
}

/// Read the shifted 32×32 pair: the I-frame plus quarter-pel (16, 0).
///
/// Motion is the 16×16 vector this encoder already writes, on a whole chroma
/// sample. The Annex B bytes are parsed. A stored buffer is not returned.
/// A B-frame and any other partition are refused.
pub fn decode_motion(annex_b: &[u8]) -> Result<(Yuv420, Yuv420), Error> {
    let nals = split_nals(annex_b)?;
    if nals.len() != 6 {
        return Err(Error::Bytes);
    }
    let (aud_h, aud) = &nals[0];
    let (sps_h, sps) = &nals[1];
    let (pps_h, pps) = &nals[2];
    let (idr_h, idr) = &nals[3];
    if *aud_h != 0x09 || aud.as_slice() != [0x10] {
        return Err(Error::Bytes);
    }
    if *sps_h != 0x67 || *pps_h != 0x68 || *idr_h != 0x65 {
        return Err(Error::Bytes);
    }
    let (width, height, frame_bits) = read_sps(sps)?;
    let qp = read_pps(pps)?;
    let intra = read_slice(idr, width, height, frame_bits, qp)?;
    let (p_aud_h, p_aud) = &nals[4];
    if *p_aud_h != 0x09 || p_aud.as_slice() != [0x30] {
        return Err(Error::Bytes);
    }
    let (p_h, p_rbsp) = &nals[5];
    if *p_h != 0x01 {
        return Err(Error::Bytes);
    }
    let predicted = read_motion_p_slice(p_rbsp, &intra, frame_bits, qp)?;
    Ok((intra, predicted))
}

fn split_nals(data: &[u8]) -> Result<Vec<(u8, Vec<u8>)>, Error> {
    if data.len() < 5 || data[0..4] != [0, 0, 0, 1] {
        return Err(Error::Bytes);
    }
    let mut starts = vec![0usize];
    let mut i = 4;
    while i + 4 <= data.len() {
        if data[i..i + 4] == [0, 0, 0, 1] {
            starts.push(i);
            i += 4;
        } else {
            i += 1;
        }
    }
    let mut nals = Vec::with_capacity(starts.len());
    for (n, &start) in starts.iter().enumerate() {
        let end = if n + 1 < starts.len() {
            starts[n + 1]
        } else {
            data.len()
        };
        let body = &data[start + 4..end];
        if body.is_empty() {
            return Err(Error::Bytes);
        }
        let header = body[0];
        if header & 0x80 != 0 {
            return Err(Error::Bytes);
        }
        nals.push((header, unescape(&body[1..])?));
    }
    Ok(nals)
}

fn unescape(rbsp: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(rbsp.len());
    let mut zeros = 0u8;
    for &b in rbsp {
        if zeros == 2 && b == 3 {
            zeros = 0;
            continue;
        }
        out.push(b);
        if b == 0 {
            zeros = zeros.saturating_add(1);
        } else {
            zeros = 0;
        }
    }
    Ok(out)
}

fn read_sps(rbsp: &[u8]) -> Result<(u32, u32, u32), Error> {
    let mut b = BitReader::new(rbsp);
    if b.read_bits(8)? != 66 {
        return Err(Error::Bytes);
    }
    if b.read_bit()? != 1 || b.read_bit()? != 1 || b.read_bits(6)? != 0 {
        return Err(Error::Bytes);
    }
    let level = b.read_bits(8)?;
    if b.read_ue()? != 0 {
        return Err(Error::Bytes);
    }
    let log2_max_frame_num_minus4 = b.read_ue()?;
    if log2_max_frame_num_minus4 > 1 {
        return Err(Error::Bytes);
    }
    if b.read_ue()? != 2 || b.read_ue()? != 1 || b.read_bit()? != 0 {
        return Err(Error::Bytes);
    }
    let mb_w = b.read_ue()? + 1;
    let mb_h = b.read_ue()? + 1;
    if b.read_bit()? != 1 || b.read_bit()? != 1 || b.read_bit()? != 0 || b.read_bit()? != 0 {
        return Err(Error::Bytes);
    }
    b.finish()?;
    let width = mb_w.saturating_mul(16);
    let height = mb_h.saturating_mul(16);
    if width == 0 || height == 0 || width > 1280 || height > 720 {
        return Err(Error::TooLarge);
    }
    let macroblocks = mb_w * mb_h;
    let expect = if macroblocks <= 99 {
        10
    } else if macroblocks <= 396 {
        11
    } else if macroblocks <= 1620 {
        30
    } else {
        31
    };
    if level != expect {
        return Err(Error::Bytes);
    }
    Ok((width, height, log2_max_frame_num_minus4 + 4))
}

fn read_pps(rbsp: &[u8]) -> Result<i32, Error> {
    let mut b = BitReader::new(rbsp);
    if b.read_ue()? != 0 || b.read_ue()? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_bit()? != 0 || b.read_bit()? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_ue()? != 0 || b.read_ue()? != 0 || b.read_ue()? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_bit()? != 0 || b.read_bits(2)? != 0 {
        return Err(Error::Bytes);
    }
    let qp = b.read_se()? + 26;
    if !(0..=51).contains(&qp) {
        return Err(Error::Qp);
    }
    if b.read_se()? != 0 || b.read_se()? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_bit()? != 1 || b.read_bit()? != 0 || b.read_bit()? != 0 {
        return Err(Error::Bytes);
    }
    b.finish()?;
    Ok(qp)
}

fn read_slice(
    rbsp: &[u8],
    width: u32,
    height: u32,
    frame_bits: u32,
    qp: i32,
) -> Result<Yuv420, Error> {
    let mut b = BitReader::new(rbsp);
    if b.read_ue()? != 0 || b.read_ue()? != 7 || b.read_ue()? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_bits(frame_bits)? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_ue()? != 0 || b.read_bit()? != 0 || b.read_bit()? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_se()? != 0 || b.read_ue()? != 1 {
        return Err(Error::Bytes);
    }
    let mb_w = (width / 16) as usize;
    let mb_h = (height / 16) as usize;
    let y_stride = width as usize;
    let c_stride = (width / 2) as usize;
    let mut y = vec![0u8; y_stride * height as usize];
    let mut u = vec![0u8; c_stride * (height as usize / 2)];
    let mut v = vec![0u8; c_stride * (height as usize / 2)];
    let mut luma_nnz = vec![0u8; mb_w * 4 * mb_h * 4];
    for my in 0..mb_h {
        for mx in 0..mb_w {
            if b.read_ue()? != 0 {
                return Err(Error::Bytes);
            }
            for _ in 0..16 {
                if b.read_bit()? != 1 {
                    return Err(Error::Bytes);
                }
            }
            if b.read_ue()? != 0 {
                return Err(Error::Bytes);
            }
            let cbp = intra_cbp_from_code(b.read_ue()?).ok_or(Error::Bytes)?;
            let cbpl = cbp & 15;
            let cbpc = cbp >> 4;
            if cbpc > 1 {
                return Err(Error::Bytes);
            }
            if cbp != 0 && b.read_se()? != 0 {
                return Err(Error::Bytes);
            }
            for i8 in 0..4 {
                for k in 0..4 {
                    let i = i8 * 4 + k;
                    let bx = mx * 4 + BLK_X[i];
                    let by = my * 4 + BLK_Y[i];
                    let scan = if cbpl & (1 << i8) == 0 {
                        [0i32; 16]
                    } else {
                        let nc = nc_at(&luma_nnz, mb_w * 4, bx, by);
                        let scan = read_block(&mut b, nc, 16)?;
                        luma_nnz[by * mb_w * 4 + bx] =
                            scan.iter().filter(|c| **c != 0).count() as u8;
                        scan
                    };
                    recon_luma(
                        &mut y,
                        y_stride,
                        mx * 16 + BLK_X[i] * 4,
                        my * 16 + BLK_Y[i] * 4,
                        &scan,
                        qp,
                    );
                }
            }
            let mut dc_u = [0i32; 4];
            let mut dc_v = [0i32; 4];
            if cbpc > 0 {
                let u_scan = read_block(&mut b, -1, 4)?;
                let v_scan = read_block(&mut b, -1, 4)?;
                dc_u.copy_from_slice(&u_scan[..4]);
                dc_v.copy_from_slice(&v_scan[..4]);
            }
            recon_chroma(&mut u, c_stride, mx, my, dc_u, qp);
            recon_chroma(&mut v, c_stride, mx, my, dc_v, qp);
        }
    }
    b.finish()?;
    Ok(Yuv420 {
        width,
        height,
        y,
        u,
        v,
    })
}

fn recon_luma(rec: &mut [u8], stride: usize, x: usize, y: usize, scan: &[i32], qp: i32) {
    let pred = pred_luma_dc(rec, stride, x, y);
    let mut z = [[0i32; 4]; 4];
    for (si, &ri) in ZIGZAG.iter().enumerate() {
        z[ri / 4][ri % 4] = scan[si];
    }
    let spatial = idct4(&dequant4(&z, qp));
    for r in 0..4 {
        for c in 0..4 {
            rec[(y + r) * stride + x + c] = clip8(pred[r * 4 + c] as i32 + spatial[r][c]);
        }
    }
}

fn recon_chroma(rec: &mut [u8], stride: usize, mb_x: usize, mb_y: usize, dc: [i32; 4], qp: i32) {
    let x0 = mb_x * 8;
    let y0 = mb_y * 8;
    let pred = pred_chroma_dc(rec, stride, x0, y0);
    let dc_r = dequant_chroma_dc(dc, qp);
    for (i, coef) in dc_r.iter().enumerate() {
        let mut z = [[0i32; 4]; 4];
        z[0][0] = *coef;
        let spatial = idct4(&z);
        let bx = i & 1;
        let by = i >> 1;
        for r in 0..4 {
            for c in 0..4 {
                let py = by * 4 + r;
                let px = bx * 4 + c;
                rec[(y0 + py) * stride + x0 + px] = clip8(pred[py * 8 + px] as i32 + spatial[r][c]);
            }
        }
    }
}

fn read_zero_p_slice(
    rbsp: &[u8],
    reference: &Yuv420,
    frame_bits: u32,
    qp: i32,
) -> Result<Yuv420, Error> {
    let mut b = BitReader::new(rbsp);
    if b.read_ue()? != 0 || b.read_ue()? != 0 || b.read_ue()? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_bits(frame_bits)? != 1 {
        return Err(Error::Bytes);
    }
    if b.read_bit()? != 0 || b.read_bit()? != 0 || b.read_se()? != 0 || b.read_ue()? != 1 {
        return Err(Error::Bytes);
    }
    let width = reference.width;
    let height = reference.height;
    let mb_w = (width / 16) as usize;
    let mb_h = (height / 16) as usize;
    let y_stride = width as usize;
    let c_stride = (width / 2) as usize;
    let mut y = vec![0u8; reference.y.len()];
    let mut u = vec![0u8; reference.u.len()];
    let mut v = vec![0u8; reference.v.len()];
    let mut luma_nnz = vec![0u8; mb_w * 4 * mb_h * 4];
    for my in 0..mb_h {
        for mx in 0..mb_w {
            if b.read_ue()? != 0 || b.read_ue()? != 0 {
                return Err(Error::Bytes);
            }
            if b.read_se()? != 0 || b.read_se()? != 0 {
                return Err(Error::Bytes);
            }
            let cbp = inter_cbp_from_code(b.read_ue()?).ok_or(Error::Bytes)?;
            let cbpl = cbp & 15;
            let cbpc = cbp >> 4;
            if cbpc > 1 {
                return Err(Error::Bytes);
            }
            if cbp != 0 && b.read_se()? != 0 {
                return Err(Error::Bytes);
            }
            for i8 in 0..4 {
                for k in 0..4 {
                    let i = i8 * 4 + k;
                    let bx = mx * 4 + BLK_X[i];
                    let by = my * 4 + BLK_Y[i];
                    let x = mx * 16 + BLK_X[i] * 4;
                    let yy = my * 16 + BLK_Y[i] * 4;
                    let scan = if cbpl & (1 << i8) == 0 {
                        [0i32; 16]
                    } else {
                        let nc = nc_at(&luma_nnz, mb_w * 4, bx, by);
                        let scan = read_block(&mut b, nc, 16)?;
                        luma_nnz[by * mb_w * 4 + bx] =
                            scan.iter().filter(|c| **c != 0).count() as u8;
                        scan
                    };
                    add_zero_luma(&mut y, &reference.y, y_stride, x, yy, &scan, qp);
                }
            }
            let mut dc_u = [0i32; 4];
            let mut dc_v = [0i32; 4];
            if cbpc > 0 {
                let u_scan = read_block(&mut b, -1, 4)?;
                let v_scan = read_block(&mut b, -1, 4)?;
                dc_u.copy_from_slice(&u_scan[..4]);
                dc_v.copy_from_slice(&v_scan[..4]);
            }
            add_zero_chroma(&mut u, &reference.u, c_stride, mx, my, dc_u, qp);
            add_zero_chroma(&mut v, &reference.v, c_stride, mx, my, dc_v, qp);
        }
    }
    b.finish()?;
    Ok(Yuv420 {
        width,
        height,
        y,
        u,
        v,
    })
}

fn read_motion_p_slice(
    rbsp: &[u8],
    reference: &Yuv420,
    frame_bits: u32,
    qp: i32,
) -> Result<Yuv420, Error> {
    let mut b = BitReader::new(rbsp);
    if b.read_ue()? != 0 || b.read_ue()? != 0 || b.read_ue()? != 0 {
        return Err(Error::Bytes);
    }
    if b.read_bits(frame_bits)? != 1 {
        return Err(Error::Bytes);
    }
    if b.read_bit()? != 0 || b.read_bit()? != 0 || b.read_se()? != 0 || b.read_ue()? != 1 {
        return Err(Error::Bytes);
    }
    let width = reference.width;
    let height = reference.height;
    let mb_w = (width / 16) as usize;
    let mb_h = (height / 16) as usize;
    let y_stride = width as usize;
    let c_stride = (width / 2) as usize;
    let mut y = vec![0u8; reference.y.len()];
    let mut u = vec![0u8; reference.u.len()];
    let mut v = vec![0u8; reference.v.len()];
    let mut luma_nnz = vec![0u8; mb_w * 4 * mb_h * 4];
    let mut mvs = vec![(0i32, 0i32); mb_w * mb_h];
    for my in 0..mb_h {
        for mx in 0..mb_w {
            if b.read_ue()? != 0 || b.read_ue()? != 0 {
                return Err(Error::Bytes);
            }
            let (px, py) = mvp_16x16(&mvs, mb_w, mx, my);
            let mvx = px + b.read_se()?;
            let mvy = py + b.read_se()?;
            if mvx % 8 != 0
                || mvy % 8 != 0
                || !(-512..=512).contains(&mvx)
                || !(-512..=512).contains(&mvy)
            {
                return Err(Error::Bytes);
            }
            mvs[my * mb_w + mx] = (mvx, mvy);
            let luma_dx = mvx / 4;
            let luma_dy = mvy / 4;
            let chroma_dx = mvx >> 3;
            let chroma_dy = mvy >> 3;
            let cbp = inter_cbp_from_code(b.read_ue()?).ok_or(Error::Bytes)?;
            let cbpl = cbp & 15;
            let cbpc = cbp >> 4;
            if cbpc > 1 {
                return Err(Error::Bytes);
            }
            if cbp != 0 && b.read_se()? != 0 {
                return Err(Error::Bytes);
            }
            for i8 in 0..4 {
                for k in 0..4 {
                    let i = i8 * 4 + k;
                    let bx = mx * 4 + BLK_X[i];
                    let by = my * 4 + BLK_Y[i];
                    let x = mx * 16 + BLK_X[i] * 4;
                    let yy = my * 16 + BLK_Y[i] * 4;
                    let scan = if cbpl & (1 << i8) == 0 {
                        [0i32; 16]
                    } else {
                        let nc = nc_at(&luma_nnz, mb_w * 4, bx, by);
                        let scan = read_block(&mut b, nc, 16)?;
                        luma_nnz[by * mb_w * 4 + bx] =
                            scan.iter().filter(|c| **c != 0).count() as u8;
                        scan
                    };
                    add_motion_luma(
                        &mut y,
                        &reference.y,
                        y_stride,
                        x,
                        yy,
                        luma_dx,
                        luma_dy,
                        width as i32,
                        height as i32,
                        &scan,
                        qp,
                    );
                }
            }
            let mut dc_u = [0i32; 4];
            let mut dc_v = [0i32; 4];
            if cbpc > 0 {
                let u_scan = read_block(&mut b, -1, 4)?;
                let v_scan = read_block(&mut b, -1, 4)?;
                dc_u.copy_from_slice(&u_scan[..4]);
                dc_v.copy_from_slice(&v_scan[..4]);
            }
            add_motion_chroma(
                &mut u,
                &reference.u,
                c_stride,
                mx,
                my,
                chroma_dx,
                chroma_dy,
                (width / 2) as i32,
                (height / 2) as i32,
                dc_u,
                qp,
            );
            add_motion_chroma(
                &mut v,
                &reference.v,
                c_stride,
                mx,
                my,
                chroma_dx,
                chroma_dy,
                (width / 2) as i32,
                (height / 2) as i32,
                dc_v,
                qp,
            );
        }
    }
    b.finish()?;
    Ok(Yuv420 {
        width,
        height,
        y,
        u,
        v,
    })
}

fn add_zero_luma(
    rec: &mut [u8],
    reference: &[u8],
    stride: usize,
    x: usize,
    y: usize,
    scan: &[i32],
    qp: i32,
) {
    let mut z = [[0i32; 4]; 4];
    for (si, &ri) in ZIGZAG.iter().enumerate() {
        z[ri / 4][ri % 4] = scan[si];
    }
    let spatial = idct4(&dequant4(&z, qp));
    for r in 0..4 {
        for c in 0..4 {
            let at = (y + r) * stride + x + c;
            rec[at] = clip8(reference[at] as i32 + spatial[r][c]);
        }
    }
}

fn add_zero_chroma(
    rec: &mut [u8],
    reference: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    dc: [i32; 4],
    qp: i32,
) {
    let x0 = mb_x * 8;
    let y0 = mb_y * 8;
    let dc_r = dequant_chroma_dc(dc, qp);
    for (i, coef) in dc_r.iter().enumerate() {
        let mut z = [[0i32; 4]; 4];
        z[0][0] = *coef;
        let spatial = idct4(&z);
        let bx = i & 1;
        let by = i >> 1;
        for r in 0..4 {
            for c in 0..4 {
                let px = x0 + bx * 4 + c;
                let py = y0 + by * 4 + r;
                let at = py * stride + px;
                rec[at] = clip8(reference[at] as i32 + spatial[r][c]);
            }
        }
    }
}

fn moved_sample(
    reference: &[u8],
    stride: usize,
    x: usize,
    y: usize,
    dx: i32,
    dy: i32,
    limit_x: i32,
    limit_y: i32,
) -> i32 {
    let sx = (x as i32 + dx).clamp(0, limit_x - 1) as usize;
    let sy = (y as i32 + dy).clamp(0, limit_y - 1) as usize;
    reference[sy * stride + sx] as i32
}

fn add_motion_luma(
    rec: &mut [u8],
    reference: &[u8],
    stride: usize,
    x: usize,
    y: usize,
    dx: i32,
    dy: i32,
    limit_x: i32,
    limit_y: i32,
    scan: &[i32],
    qp: i32,
) {
    let mut z = [[0i32; 4]; 4];
    for (si, &ri) in ZIGZAG.iter().enumerate() {
        z[ri / 4][ri % 4] = scan[si];
    }
    let spatial = idct4(&dequant4(&z, qp));
    for r in 0..4 {
        for c in 0..4 {
            let pred = moved_sample(reference, stride, x + c, y + r, dx, dy, limit_x, limit_y);
            rec[(y + r) * stride + x + c] = clip8(pred + spatial[r][c]);
        }
    }
}

fn add_motion_chroma(
    rec: &mut [u8],
    reference: &[u8],
    stride: usize,
    mb_x: usize,
    mb_y: usize,
    dx: i32,
    dy: i32,
    limit_x: i32,
    limit_y: i32,
    dc: [i32; 4],
    qp: i32,
) {
    let x0 = mb_x * 8;
    let y0 = mb_y * 8;
    let dc_r = dequant_chroma_dc(dc, qp);
    for (i, coef) in dc_r.iter().enumerate() {
        let mut z = [[0i32; 4]; 4];
        z[0][0] = *coef;
        let spatial = idct4(&z);
        let bx = i & 1;
        let by = i >> 1;
        for r in 0..4 {
            for c in 0..4 {
                let px = x0 + bx * 4 + c;
                let py = y0 + by * 4 + r;
                let pred = moved_sample(reference, stride, px, py, dx, dy, limit_x, limit_y);
                rec[py * stride + px] = clip8(pred + spatial[r][c]);
            }
        }
    }
}
