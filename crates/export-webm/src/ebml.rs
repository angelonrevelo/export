//! Element ids, sizes, and integer readers. Payloads are not interpreted.

use crate::error::Error;

pub const ID_EBML: u64 = 0x1A45DFA3;
pub const ID_EBML_VERSION: u64 = 0x4286;
pub const ID_EBML_READ_VERSION: u64 = 0x42F7;
pub const ID_EBML_MAX_ID_LENGTH: u64 = 0x42F2;
pub const ID_EBML_MAX_SIZE_LENGTH: u64 = 0x42F3;
pub const ID_DOCTYPE: u64 = 0x4282;
pub const ID_DOCTYPE_VERSION: u64 = 0x4287;
pub const ID_DOCTYPE_READ_VERSION: u64 = 0x4285;
pub const ID_VOID: u64 = 0xEC;
pub const ID_CRC32: u64 = 0xBF;
pub const ID_SEGMENT: u64 = 0x18538067;
pub const ID_INFO: u64 = 0x1549A966;
pub const ID_TIMESTAMP_SCALE: u64 = 0x2AD7B1;
pub const ID_MUXING_APP: u64 = 0x4D80;
pub const ID_WRITING_APP: u64 = 0x5741;
pub const ID_DURATION: u64 = 0x4489;
pub const ID_TRACKS: u64 = 0x1654AE6B;
pub const ID_TRACK_ENTRY: u64 = 0xAE;
pub const ID_TRACK_NUMBER: u64 = 0xD7;
pub const ID_TRACK_UID: u64 = 0x73C5;
pub const ID_TRACK_TYPE: u64 = 0x83;
pub const ID_FLAG_ENABLED: u64 = 0xB9;
pub const ID_FLAG_DEFAULT: u64 = 0x88;
pub const ID_FLAG_LACING: u64 = 0x9C;
pub const ID_CODEC_ID: u64 = 0x86;
pub const ID_CODEC_PRIVATE: u64 = 0x63A2;
pub const ID_VIDEO: u64 = 0xE0;
pub const ID_AUDIO: u64 = 0xE1;
pub const ID_PIXEL_WIDTH: u64 = 0xB0;
pub const ID_PIXEL_HEIGHT: u64 = 0xBA;
pub const ID_COLOUR_SPACE: u64 = 0x2EB524;
pub const ID_SAMPLING_FREQUENCY: u64 = 0xB5;
pub const ID_CHANNELS: u64 = 0x9F;
pub const ID_CLUSTER: u64 = 0x1F43B675;
pub const ID_TIMESTAMP: u64 = 0xE7;
pub const ID_SIMPLE_BLOCK: u64 = 0xA3;
pub const ID_BLOCK_GROUP: u64 = 0xA0;
pub const ID_BLOCK: u64 = 0xA1;
pub const ID_BLOCK_DURATION: u64 = 0x9B;
pub const ID_REFERENCE_BLOCK: u64 = 0xFB;

pub const CODEC_VP8: &str = "V_VP8";
pub const CODEC_VP9: &str = "V_VP9";
pub const CODEC_OPUS: &str = "A_OPUS";
pub const CODEC_VORBIS: &str = "A_VORBIS";

/// A header that has been checked and whose body is still unread.
pub struct Element<'a> {
    pub id: u64,
    pub body: &'a [u8],
}

pub struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    end: usize,
}

impl<'a> Cursor<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            end: data.len(),
        }
    }

    pub fn slice(data: &'a [u8], end: usize) -> Self {
        Self {
            data,
            pos: 0,
            end: end.min(data.len()),
        }
    }

    pub fn next_element(&mut self) -> Result<Option<Element<'a>>, Error> {
        if self.pos >= self.end {
            return Ok(None);
        }
        if self.pos >= self.data.len() {
            return Err(Error::Truncated);
        }
        let id = read_element_id(self.data, &mut self.pos, self.end)?;
        if self.pos > self.end {
            return Err(Error::Truncated);
        }
        let (size, unknown) = read_size(self.data, &mut self.pos, self.end)?;
        if self.pos > self.end {
            return Err(Error::Truncated);
        }
        let body = if unknown {
            if self.pos > self.end {
                return Err(Error::Truncated);
            }
            &self.data[self.pos..self.end]
        } else {
            let size = size as usize;
            let body_end = self.pos.checked_add(size).ok_or(Error::Invalid)?;
            if body_end > self.end || body_end > self.data.len() {
                return Err(Error::Truncated);
            }
            let body = &self.data[self.pos..body_end];
            self.pos = body_end;
            body
        };
        if unknown {
            self.pos = self.end;
        }
        Ok(Some(Element { id, body }))
    }
}

/// Element id VINT. Illegal encodings are [`Error::BadElementId`].
pub fn read_element_id(data: &[u8], pos: &mut usize, end: usize) -> Result<u64, Error> {
    let end = end.min(data.len());
    if *pos >= end {
        return Err(Error::Truncated);
    }
    let first = data[*pos];
    if first == 0 {
        return Err(Error::BadElementId);
    }
    let width = (first.leading_zeros() as usize) + 1;
    if width > 4 {
        return Err(Error::BadElementId);
    }
    if pos.saturating_add(width) > end {
        return Err(Error::Truncated);
    }
    let mut raw = 0u64;
    for i in 0..width {
        raw = (raw << 8) | u64::from(data[*pos + i]);
    }
    *pos += width;

    let marker = 1u64 << (width * 7);
    let data_bits = raw & (marker - 1);
    if data_bits == 0 || data_bits == marker - 1 {
        return Err(Error::BadElementId);
    }
    if width > 1 {
        let shorter = 1u64 << ((width - 1) * 7);
        if data_bits < shorter {
            return Err(Error::BadElementId);
        }
    }
    Ok(raw)
}

/// Size VINT. `unknown` is the all-ones reserved length.
pub fn read_size(data: &[u8], pos: &mut usize, end: usize) -> Result<(u64, bool), Error> {
    let end = end.min(data.len());
    if *pos >= end {
        return Err(Error::Truncated);
    }
    let first = data[*pos];
    if first == 0 {
        return Err(Error::Invalid);
    }
    let width = (first.leading_zeros() as usize) + 1;
    if width > 8 {
        return Err(Error::Invalid);
    }
    if pos.saturating_add(width) > end {
        return Err(Error::Truncated);
    }
    let mut raw = 0u64;
    for i in 0..width {
        raw = (raw << 8) | u64::from(data[*pos + i]);
    }
    *pos += width;
    let marker = 1u64 << (width * 7);
    let data_bits = raw & (marker - 1);
    if data_bits == marker - 1 {
        return Ok((0, true));
    }
    Ok((data_bits, false))
}

pub fn read_uint(data: &[u8]) -> Result<u64, Error> {
    if data.is_empty() || data.len() > 8 {
        return Err(Error::Invalid);
    }
    let mut value = 0u64;
    for byte in data {
        value = (value << 8) | u64::from(*byte);
    }
    Ok(value)
}

pub fn read_float(data: &[u8]) -> Result<f64, Error> {
    match data.len() {
        4 => Ok(f64::from(f32::from_be_bytes(data.try_into().unwrap()))),
        8 => Ok(f64::from_be_bytes(data.try_into().unwrap())),
        _ => Err(Error::Invalid),
    }
}

pub fn read_string(data: &[u8]) -> Result<String, Error> {
    let end = data.iter().position(|b| *b == 0).unwrap_or(data.len());
    let bytes = &data[..end];
    std::str::from_utf8(bytes)
        .map(|s| s.to_owned())
        .map_err(|_| Error::Invalid)
}

pub fn write_id(out: &mut Vec<u8>, id: u64) {
    let bytes = id.to_be_bytes();
    let start = bytes.iter().position(|b| *b != 0).unwrap_or(7);
    out.extend_from_slice(&bytes[start..]);
}

/// Largest known-size value that fits in a VINT of `width` bytes.
/// The all-ones pattern is reserved for unknown length, so it is excluded.
fn max_known_vint(width: usize) -> u64 {
    let bits = 7 * width;
    if bits >= 64 {
        u64::MAX
    } else {
        (1u64 << bits) - 2
    }
}

pub fn write_size(out: &mut Vec<u8>, size: u64) -> Result<(), Error> {
    let mut width = 1usize;
    while width < 8 && size > max_known_vint(width) {
        width += 1;
    }
    if size > max_known_vint(width) {
        return Err(Error::Invalid);
    }
    let marker = 1u64 << (width * 7);
    let raw = marker | size;
    let shift = (8 - width) * 8;
    let bytes = (raw << shift).to_be_bytes();
    out.extend_from_slice(&bytes[..width]);
    Ok(())
}

pub fn element(out: &mut Vec<u8>, id: u64, payload: &[u8]) -> Result<(), Error> {
    write_id(out, id);
    write_size(out, payload.len() as u64)?;
    out.extend_from_slice(payload);
    Ok(())
}

pub fn uint_element(out: &mut Vec<u8>, id: u64, value: u64) -> Result<(), Error> {
    if value == 0 {
        return element(out, id, &[0]);
    }
    let bytes = value.to_be_bytes();
    let start = bytes.iter().position(|b| *b != 0).unwrap_or(7);
    element(out, id, &bytes[start..])
}

pub fn string_element(out: &mut Vec<u8>, id: u64, value: &str) -> Result<(), Error> {
    element(out, id, value.as_bytes())
}

pub fn float64_element(out: &mut Vec<u8>, id: u64, value: f64) -> Result<(), Error> {
    element(out, id, &value.to_be_bytes())
}

pub fn master(id: u64, payload: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::with_capacity(payload.len().saturating_add(16));
    element(&mut out, id, payload)?;
    Ok(out)
}

/// Whether a codec id is refused. No decoder is called.
///
/// [`CODEC_VP8`], [`CODEC_VP9`], [`CODEC_OPUS`], and [`CODEC_VORBIS`] are
/// copied as block bytes and are not decoded, so this returns false for them
/// and for every other id.
pub fn unsupported_codec(codec_id: &str) -> bool {
    let copied = [
        CODEC_VP8,
        CODEC_VP9,
        CODEC_OPUS,
        CODEC_VORBIS,
    ]
    .iter()
    .any(|id| codec_id == *id);
    let _ = copied;
    false
}

#[cfg(test)]
mod tests {
    use super::{read_element_id, read_size, write_size};
    use crate::error::Error;

    #[test]
    fn size_127_is_known() {
        let mut buf = Vec::new();
        write_size(&mut buf, 127).unwrap();
        assert_ne!(buf, vec![0xff]);
        let mut pos = 0;
        let (size, unknown) = read_size(&buf, &mut pos, buf.len()).unwrap();
        assert!(!unknown);
        assert_eq!(size, 127);
        assert_eq!(pos, buf.len());
    }

    #[test]
    fn overlong_and_reserved_ids_are_bad() {
        let mut pos = 0;
        assert_eq!(
            read_element_id(&[0x40, 0x01], &mut pos, 2).unwrap_err(),
            Error::BadElementId
        );
        pos = 0;
        assert_eq!(
            read_element_id(&[0xff], &mut pos, 1).unwrap_err(),
            Error::BadElementId
        );
        pos = 0;
        assert_eq!(
            read_element_id(&[0x80], &mut pos, 1).unwrap_err(),
            Error::BadElementId
        );
    }

    #[test]
    fn id_may_not_cross_the_parent_end() {
        let mut pos = 0;
        let data = [0x1A, 0x45, 0xDF, 0xA3];
        assert_eq!(
            read_element_id(&data, &mut pos, 2).unwrap_err(),
            Error::Truncated
        );
    }
}
