use crate::bits::{rbsp, Bits};
use crate::error::Error;

/// Sequence parameter set fields the slice walk and the `avc1` box both need.
pub struct Sps {
    pub sps_id: u32,
    pub log2_max_frame_num: u32,
    pub poc_type: u32,
    pub log2_max_poc_lsb: u32,
    pub delta_pic_order_always_zero: bool,
    pub mb_w: u32,
    pub mb_h: u32,
    pub width: u32,
    pub height: u32,
}

/// Picture parameter set fields the slice header reads.
pub struct Pps {
    pub pps_id: u32,
    pub sps_id: u32,
    pub entropy_cabac: bool,
    pub bottom_field_pic_order: bool,
    pub deblocking_control: bool,
    pub redundant_pic_cnt: bool,
}

pub fn sps(nal: &[u8]) -> Result<Sps, Error> {
    let raw = rbsp(nal)?;
    let mut bits = Bits::new(&raw);
    let profile = bits.bits(8)?;
    let _compat = bits.bits(8)?;
    let _level = bits.bits(8)?;
    let sps_id = bits.ue()?;
    let mut chroma = 1u32;
    if high_profile(profile) {
        chroma = bits.ue()?;
        if chroma == 3 {
            let _separate = bits.bit()?;
        }
        let _bit_depth_luma = bits.ue()?;
        let _bit_depth_chroma = bits.ue()?;
        let _bypass = bits.bit()?;
        if bits.bit()? != 0 {
            return Err(Error::Frame);
        }
    }
    let log2_max_frame_num = bits.ue()?.saturating_add(4);
    if log2_max_frame_num > 16 {
        return Err(Error::Frame);
    }
    let poc_type = bits.ue()?;
    let mut log2_max_poc_lsb = 0u32;
    let mut delta_pic_order_always_zero = false;
    if poc_type == 0 {
        log2_max_poc_lsb = bits.ue()?.saturating_add(4);
        if log2_max_poc_lsb > 16 {
            return Err(Error::Frame);
        }
    } else if poc_type == 1 {
        delta_pic_order_always_zero = bits.bit()? == 1;
        let _ = bits.se()?;
        let _ = bits.se()?;
        let n = bits.ue()?;
        if n > 255 {
            return Err(Error::Frame);
        }
        for _ in 0..n {
            let _ = bits.se()?;
        }
    } else if poc_type > 2 {
        return Err(Error::Frame);
    }
    let _max_ref = bits.ue()?;
    let _gaps = bits.bit()?;
    let mb_w = bits.ue()?.saturating_add(1);
    let mb_h = bits.ue()?.saturating_add(1);
    if mb_w == 0 || mb_h == 0 || mb_w > 256 || mb_h > 256 {
        return Err(Error::Size);
    }
    let frame_mbs_only = bits.bit()? == 1;
    if !frame_mbs_only {
        let _mbaff = bits.bit()?;
    }
    let _direct = bits.bit()?;
    let crop = bits.bit()? == 1;
    let mut width = mb_w.saturating_mul(16);
    let mut height = mb_h.saturating_mul(16);
    if crop {
        let left = bits.ue()?;
        let right = bits.ue()?;
        let top = bits.ue()?;
        let bottom = bits.ue()?;
        let sub_w = left.saturating_add(right).saturating_mul(2);
        let sub_h = top.saturating_add(bottom).saturating_mul(2);
        if sub_w >= width || sub_h >= height {
            return Err(Error::Size);
        }
        width -= sub_w;
        height -= sub_h;
    }
    if bits.bit()? != 0 {
        return Err(Error::Frame);
    }
    bits.finish()?;
    if !frame_mbs_only || chroma != 1 || width > u16::MAX as u32 || height > u16::MAX as u32 {
        return Err(Error::Frame);
    }
    Ok(Sps {
        sps_id,
        log2_max_frame_num,
        poc_type,
        log2_max_poc_lsb,
        delta_pic_order_always_zero,
        mb_w,
        mb_h,
        width,
        height,
    })
}

pub fn pps(nal: &[u8]) -> Result<Pps, Error> {
    let raw = rbsp(nal)?;
    let mut bits = Bits::new(&raw);
    let pps_id = bits.ue()?;
    let sps_id = bits.ue()?;
    let entropy_cabac = bits.bit()? == 1;
    let bottom_field_pic_order = bits.bit()? == 1;
    let slice_groups = bits.ue()?;
    if slice_groups != 0 {
        return Err(Error::Frame);
    }
    let _l0 = bits.ue()?;
    let _l1 = bits.ue()?;
    let _weighted_pred = bits.bit()?;
    let _weighted_bipred = bits.bits(2)?;
    let _init_qp = bits.se()?;
    let _init_qs = bits.se()?;
    let _chroma_qp = bits.se()?;
    let deblocking_control = bits.bit()? == 1;
    let _constrained = bits.bit()?;
    let redundant_pic_cnt = bits.bit()? == 1;
    bits.finish()?;
    Ok(Pps {
        pps_id,
        sps_id,
        entropy_cabac,
        bottom_field_pic_order,
        deblocking_control,
        redundant_pic_cnt,
    })
}

fn high_profile(profile: u32) -> bool {
    matches!(
        profile,
        100 | 110 | 122 | 244 | 44 | 83 | 86 | 118 | 128 | 138 | 139 | 134 | 135
    )
}
