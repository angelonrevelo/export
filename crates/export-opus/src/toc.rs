use crate::error::Error;

/// Parsed table-of-contents byte (RFC 6716 section 3.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Toc {
    pub config: u8,
    pub stereo: bool,
    pub code: u8,
    pub frames: usize,
}

impl Toc {
    pub(crate) fn from_byte(byte: u8) -> Self {
        Self {
            config: byte >> 3,
            stereo: (byte & 0x04) != 0,
            code: byte & 0x03,
            frames: 1,
        }
    }
}

/// Samples at 48 kHz represented by one frame of this configuration.
pub(crate) fn samples_per_frame(config: u8) -> Result<usize, Error> {
    const SILK: [usize; 4] = [480, 960, 1920, 2880];
    const CELT: [usize; 4] = [120, 240, 480, 960];
    match config {
        0..=11 => Ok(SILK[(config % 4) as usize]),
        12 | 14 => Ok(480),
        13 | 15 => Ok(960),
        16..=31 => Ok(CELT[((config - 16) % 4) as usize]),
        _ => Err(Error::BadToc),
    }
}

/// Walk the packet header far enough to know whether it is truncated or the
/// TOC structure is illegal. Does not interpret the frame payload.
pub(crate) fn parse(packet: &[u8]) -> Result<Toc, Error> {
    if packet.is_empty() {
        return Err(Error::Truncated);
    }
    let mut toc = Toc::from_byte(packet[0]);
    let samples = samples_per_frame(toc.config)?;
    match toc.code {
        0 => {
            if packet.len() < 2 {
                return Err(Error::Truncated);
            }
            toc.frames = 1;
        }
        1 => {
            let payload = packet.len().saturating_sub(1);
            if payload < 2 {
                return Err(Error::Truncated);
            }
            if payload % 2 != 0 {
                return Err(Error::BadToc);
            }
            toc.frames = 2;
        }
        2 => {
            let (len1, header) = code2_frame_length(packet)?;
            if packet.len() < header + len1 {
                return Err(Error::Truncated);
            }
            let len2 = packet.len() - header - len1;
            if len1 == 0 || len2 == 0 {
                return Err(Error::BadToc);
            }
            toc.frames = 2;
        }
        3 => {
            toc.frames = parse_code3(packet)?;
        }
        _ => return Err(Error::BadToc),
    }
    let total = samples.saturating_mul(toc.frames);
    // 120 ms at 48 kHz. Longer packets are illegal.
    if total == 0 || total > 5760 {
        return Err(Error::BadToc);
    }
    Ok(toc)
}

fn code2_frame_length(packet: &[u8]) -> Result<(usize, usize), Error> {
    if packet.len() < 2 {
        return Err(Error::Truncated);
    }
    let first = packet[1] as usize;
    if first < 252 {
        return Ok((first, 2));
    }
    if packet.len() < 3 {
        return Err(Error::Truncated);
    }
    let len = packet[2] as usize * 4 + first;
    Ok((len, 3))
}

/// Code 3: frame-count byte, optional padding length, optional VBR sizes,
/// then frame bytes, then padding. Returns the frame count.
fn parse_code3(packet: &[u8]) -> Result<usize, Error> {
    if packet.len() < 2 {
        return Err(Error::Truncated);
    }
    let count_byte = packet[1];
    let count = (count_byte & 0x3F) as usize;
    if count == 0 || count > 48 {
        return Err(Error::BadToc);
    }
    let vbr = (count_byte & 0x80) != 0;
    let has_padding = (count_byte & 0x40) != 0;
    let mut offset = 2usize;
    let padding = if has_padding {
        parse_padding(packet, &mut offset)?
    } else {
        0
    };
    let mut used = 0usize;
    if vbr {
        for _ in 0..count - 1 {
            let (size, nbytes) = read_frame_size(&packet[offset..])?;
            if size == 0 {
                return Err(Error::BadToc);
            }
            offset += nbytes;
            used = match used.checked_add(size) {
                Some(sum) => sum,
                None => return Err(Error::BadToc),
            };
        }
    }
    if offset > packet.len() || padding > packet.len() - offset {
        return Err(Error::Truncated);
    }
    let frame_bytes = packet.len() - offset - padding;
    if vbr {
        // The last frame is whatever remains. Nothing remaining means the
        // packet ended before that frame.
        if used >= frame_bytes {
            return Err(Error::Truncated);
        }
    } else if frame_bytes < count {
        return Err(Error::Truncated);
    } else if frame_bytes % count != 0 {
        return Err(Error::BadToc);
    }
    Ok(count)
}

/// Padding length: a byte below 255 is the count; 255 means 254 bytes and
/// another length byte follows.
fn parse_padding(packet: &[u8], offset: &mut usize) -> Result<usize, Error> {
    let mut padding = 0usize;
    loop {
        if *offset >= packet.len() {
            return Err(Error::Truncated);
        }
        let byte = packet[*offset] as usize;
        *offset += 1;
        let add = if byte == 255 { 254 } else { byte };
        padding = padding.saturating_add(add);
        if byte != 255 {
            return Ok(padding);
        }
    }
}

fn read_frame_size(data: &[u8]) -> Result<(usize, usize), Error> {
    if data.is_empty() {
        return Err(Error::Truncated);
    }
    let first = data[0] as usize;
    if first < 252 {
        return Ok((first, 1));
    }
    if data.len() < 2 {
        return Err(Error::Truncated);
    }
    Ok((4 * data[1] as usize + first, 2))
}
