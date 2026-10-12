//! WAV/PCM read and write. PCM bytes are borrowed from the input on read.

use crate::error::{Error, Result};
use crate::pcm;
use std::io::Write;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WavSpec {
    pub channels: u16,
    pub sample_rate: u32,
    pub bits: u16,
}

#[derive(Clone, Copy, Debug)]
pub struct Wav<'a> {
    pub spec: WavSpec,
    pub pcm: &'a [u8],
}

pub fn decode(data: &[u8]) -> Result<Wav<'_>> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(Error::Corrupt("not a wav"));
    }
    let mut off = 12usize;
    let mut spec: Option<WavSpec> = None;
    let mut pcm: Option<&[u8]> = None;
    while off + 8 <= data.len() {
        let id = [data[off], data[off + 1], data[off + 2], data[off + 3]];
        let size = u32::from_le_bytes([data[off + 4], data[off + 5], data[off + 6], data[off + 7]])
            as usize;
        let start = off + 8;
        let end = start.checked_add(size).ok_or(Error::Corrupt("wav chunk"))?;
        if end > data.len() {
            return Err(Error::Truncated {
                at: data.len(),
                need: end - data.len(),
            });
        }
        if &id == b"fmt " {
            spec = Some(parse_fmt(&data[start..end])?);
        } else if &id == b"data" {
            pcm = Some(&data[start..end]);
        }
        off = end + (size & 1);
    }
    let spec = spec.ok_or(Error::Corrupt("wav missing fmt"))?;
    let pcm = pcm.ok_or(Error::Corrupt("wav missing data"))?;
    Ok(Wav { spec, pcm })
}

fn parse_fmt(fmt: &[u8]) -> Result<WavSpec> {
    if fmt.len() < 16 {
        return Err(Error::Corrupt("short fmt"));
    }
    let format = u16::from_le_bytes([fmt[0], fmt[1]]);
    let channels = u16::from_le_bytes([fmt[2], fmt[3]]);
    let sample_rate = u32::from_le_bytes([fmt[4], fmt[5], fmt[6], fmt[7]]);
    let bits = u16::from_le_bytes([fmt[14], fmt[15]]);
    if format != 1 {
        return Err(Error::Unsupported("wav is not pcm"));
    }
    if channels == 0 || sample_rate == 0 || (bits != 16 && bits != 8) {
        return Err(Error::Unsupported("wav pcm layout"));
    }
    Ok(WavSpec {
        channels,
        sample_rate,
        bits,
    })
}

pub fn encode(spec: WavSpec, pcm: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(44 + pcm.len());
    write_to(&mut out, spec, pcm)?;
    Ok(out)
}

pub fn write_to<W: Write>(w: &mut W, spec: WavSpec, pcm: &[u8]) -> Result<()> {
    if spec.bits != 16 && spec.bits != 8 {
        return Err(Error::Unsupported("wav writer bits"));
    }
    let bps = spec.bits / 8;
    if spec.channels == 0 || pcm.len() % (bps as usize * spec.channels as usize) != 0 {
        return Err(Error::Corrupt("wav pcm alignment"));
    }
    let mut header = [0u8; 44];
    header[0..4].copy_from_slice(b"RIFF");
    let riff_size = (36 + pcm.len()) as u32;
    header[4..8].copy_from_slice(&riff_size.to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(b"fmt ");
    header[16..20].copy_from_slice(&16u32.to_le_bytes());
    header[20..22].copy_from_slice(&1u16.to_le_bytes());
    header[22..24].copy_from_slice(&spec.channels.to_le_bytes());
    header[24..28].copy_from_slice(&spec.sample_rate.to_le_bytes());
    let byte_rate = spec.sample_rate * u32::from(spec.channels) * u32::from(bps);
    header[28..32].copy_from_slice(&byte_rate.to_le_bytes());
    let block = spec.channels * bps;
    header[32..34].copy_from_slice(&block.to_le_bytes());
    header[34..36].copy_from_slice(&spec.bits.to_le_bytes());
    header[36..40].copy_from_slice(b"data");
    header[40..44].copy_from_slice(&(pcm.len() as u32).to_le_bytes());
    w.write_all(&header)
        .map_err(|_| Error::Corrupt("wav write"))?;
    w.write_all(pcm).map_err(|_| Error::Corrupt("wav write"))?;
    Ok(())
}

/// Decode to interleaved s16. 8-bit WAV is unsigned and expanded.
pub fn to_s16(wav: &Wav<'_>) -> Result<Vec<i16>> {
    match wav.spec.bits {
        16 => pcm::s16_from_le(wav.pcm),
        8 => Ok(wav.pcm.iter().map(|b| (i16::from(*b) - 128) << 8).collect()),
        _ => Err(Error::Unsupported("wav bits")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_skips_a_junk_chunk() {
        let spec = WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits: 16,
        };
        let pcm = pcm::s16_le(&[0, -1, 32767, -32768]);
        let mut bytes = encode(spec, &pcm).unwrap();
        // Insert a JUNK chunk before data by rebuilding: fmt is already first.
        // Decoder must ignore unknown chunks, so append is not the test; splice.
        let mut with_junk = bytes[..36].to_vec();
        with_junk.extend_from_slice(b"JUNK");
        with_junk.extend_from_slice(&4u32.to_le_bytes());
        with_junk.extend_from_slice(b"xxxx");
        with_junk.extend_from_slice(&bytes[36..]);
        // RIFF size is stale; the walker follows chunk sizes, not the RIFF size.
        bytes = with_junk;
        let decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.spec, spec);
        assert_eq!(decoded.pcm, pcm.as_slice());
    }
}
