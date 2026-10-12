use crate::bits::{rbsp, Bits};
use crate::error::Error;

/// One I-frame, optionally followed by P-slices. AUD is checked and dropped.
pub struct Access<'a> {
    pub sps: &'a [u8],
    pub pps: &'a [u8],
    pub idr: &'a [u8],
    pub sei: Vec<&'a [u8]>,
    pub predicted: Vec<&'a [u8]>,
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
    let mut predicted = Vec::new();
    let mut saw_leading_aud = false;
    let mut aud_waiting = false;
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
            1 => {
                if idr.is_none() {
                    return Err(Error::Frame);
                }
                aud_waiting = false;
                predicted.push(nal);
            }
            2..=4 => return Err(Error::Frame),
            5 => {
                if idr.is_some() || nal[0] & 0x60 == 0 {
                    return Err(Error::Frame);
                }
                aud_waiting = false;
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
                if sps.is_some() || idr.is_some() {
                    return Err(Error::Frame);
                }
                sps = Some(nal);
            }
            8 => {
                if pps.is_some() || idr.is_some() {
                    return Err(Error::Frame);
                }
                pps = Some(nal);
            }
            9 => {
                if idr.is_none() {
                    if saw_leading_aud {
                        return Err(Error::Frame);
                    }
                    saw_leading_aud = true;
                    check_aud(nal, 0)?;
                } else {
                    if aud_waiting {
                        return Err(Error::Frame);
                    }
                    check_aud(nal, 1)?;
                    aud_waiting = true;
                }
            }
            _ => return Err(Error::Frame),
        }
    }
    if aud_waiting {
        return Err(Error::Truncated);
    }

    match (sps, pps, idr) {
        (Some(sps), Some(pps), Some(idr)) => Ok(Access {
            sps,
            pps,
            idr,
            sei,
            predicted,
        }),
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

fn check_aud(nal: &[u8], pic_type: u32) -> Result<(), Error> {
    if nal.len() < 2 {
        return Err(Error::Truncated);
    }
    let raw = rbsp(nal)?;
    let mut bits = Bits::new(&raw);
    let pic = bits.bits(3)?;
    if pic != pic_type {
        return Err(Error::Frame);
    }
    bits.finish()
}
