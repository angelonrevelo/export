use crate::bits::{rbsp, Bits};
use crate::error::Error;

/// One access unit: SPS, PPS, and the IDR slice. AUD is checked and dropped.
pub struct Access<'a> {
    pub sps: &'a [u8],
    pub pps: &'a [u8],
    pub idr: &'a [u8],
    pub sei: Vec<&'a [u8]>,
}

pub fn split(data: &[u8]) -> Result<Access<'_>, Error> {
    if data.len() < 4 {
        return Err(Error::Truncated);
    }
    let mut nal_at = Vec::new();
    let mut i = 0;
    while i + 3 <= data.len() {
        if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            nal_at.push(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    if nal_at.is_empty() {
        return Err(Error::Truncated);
    }

    let first = start_origin(data, nal_at[0]);
    if data[..first].iter().any(|&b| b != 0) {
        return Err(Error::Frame);
    }

    let mut sps = None;
    let mut pps = None;
    let mut idr = None;
    let mut sei = Vec::new();
    let mut saw_aud = false;
    for (n, &start) in nal_at.iter().enumerate() {
        let end = if n + 1 < nal_at.len() {
            start_origin(data, nal_at[n + 1])
        } else {
            data.len()
        };
        if start >= end {
            return Err(Error::Truncated);
        }
        let nal = &data[start..end];
        if nal[0] & 0x80 != 0 {
            return Err(Error::Frame);
        }
        match nal[0] & 0x1f {
            1..=4 => return Err(Error::Frame),
            5 => {
                if idr.is_some() || nal[0] & 0x60 == 0 {
                    return Err(Error::Frame);
                }
                idr = Some(nal);
            }
            6 => {
                if idr.is_some() || nal.len() < 2 {
                    return Err(if nal.len() < 2 {
                        Error::Truncated
                    } else {
                        Error::Frame
                    });
                }
                sei.push(nal);
            }
            7 => {
                if sps.is_some() {
                    return Err(Error::Frame);
                }
                sps = Some(nal);
            }
            8 => {
                if pps.is_some() {
                    return Err(Error::Frame);
                }
                pps = Some(nal);
            }
            9 => {
                if saw_aud {
                    return Err(Error::Frame);
                }
                saw_aud = true;
                check_aud(nal)?;
            }
            _ => return Err(Error::Frame),
        }
    }

    match (sps, pps, idr) {
        (Some(sps), Some(pps), Some(idr)) => Ok(Access { sps, pps, idr, sei }),
        _ => Err(Error::Truncated),
    }
}

fn start_origin(data: &[u8], nal_pos: usize) -> usize {
    let mut origin = nal_pos - 3;
    if origin > 0 && data[origin - 1] == 0 {
        origin -= 1;
    }
    origin
}

fn check_aud(nal: &[u8]) -> Result<(), Error> {
    if nal.len() < 2 {
        return Err(Error::Truncated);
    }
    let raw = rbsp(nal)?;
    let mut bits = Bits::new(&raw);
    let pic = bits.bits(3)?;
    if pic != 0 {
        return Err(Error::Frame);
    }
    bits.finish()
}
