use crate::{decode_keyframe, Error, Frame};

/// Decode an IVF file that holds one VP9 keyframe.
pub fn decode_ivf(bytes: &[u8]) -> Result<Frame, Error> {
    if bytes.len() < 32 {
        return Err(Error::Truncated);
    }
    if &bytes[0..4] != b"DKIF" {
        return Err(Error::BadHeader);
    }
    let header_len = u16::from_le_bytes([bytes[6], bytes[7]]) as usize;
    if header_len < 32 || bytes.len() < header_len {
        return Err(Error::Truncated);
    }
    match &bytes[8..12] {
        b"VP80" => return Err(Error::Unsupported),
        b"VP90" => {}
        _ => return Err(Error::BadHeader),
    }
    let declared = u32::from_le_bytes([bytes[24], bytes[25], bytes[26], bytes[27]]);
    if declared > 1 {
        return Err(Error::Unsupported);
    }
    let mut offset = header_len;
    let mut payload: Option<&[u8]> = None;
    while offset < bytes.len() {
        if bytes.len() - offset < 12 {
            return Err(Error::Truncated);
        }
        let size = u32::from_le_bytes([
            bytes[offset],
            bytes[offset + 1],
            bytes[offset + 2],
            bytes[offset + 3],
        ]) as usize;
        let start = offset + 12;
        let end = start.checked_add(size).ok_or(Error::BadHeader)?;
        if end > bytes.len() {
            return Err(Error::Truncated);
        }
        if payload.is_some() {
            return Err(Error::Unsupported);
        }
        if size == 0 {
            return Err(Error::Truncated);
        }
        payload = Some(&bytes[start..end]);
        offset = end;
    }
    if offset != bytes.len() {
        return Err(Error::BadHeader);
    }
    let frame = payload.ok_or(Error::Truncated)?;
    let ivf_w = u16::from_le_bytes([bytes[12], bytes[13]]) as usize;
    let ivf_h = u16::from_le_bytes([bytes[14], bytes[15]]) as usize;
    let decoded = decode_keyframe(frame)?;
    if decoded.width != ivf_w || decoded.height != ivf_h {
        return Err(Error::BadHeader);
    }
    Ok(decoded)
}
