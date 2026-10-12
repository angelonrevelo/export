//! PCM WAV. 8-bit is unsigned in the file and signed in [`Pcm`].

use crate::error::{Error, Result};
use crate::flac::Pcm;

pub fn decode(data: &[u8]) -> Result<Pcm> {
    if data.len() < 12 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
        return Err(Error::Corrupt("wav"));
    }
    let mut pos = 12usize;
    let mut fmt: Option<(u16, u16, u32, u16)> = None;
    let mut pcm = None;
    while pos + 8 <= data.len() {
        let id = &data[pos..pos + 4];
        let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().unwrap()) as usize;
        pos += 8;
        if pos + size > data.len() {
            return Err(Error::Truncated {
                at: data.len(),
                need: pos + size - data.len(),
            });
        }
        let body = &data[pos..pos + size];
        if id == b"fmt " {
            if body.len() < 16 {
                return Err(Error::Corrupt("wav fmt"));
            }
            let format = u16::from_le_bytes([body[0], body[1]]);
            let channels = u16::from_le_bytes([body[2], body[3]]);
            let rate = u32::from_le_bytes([body[4], body[5], body[6], body[7]]);
            let bits = u16::from_le_bytes([body[14], body[15]]);
            if format != 1 {
                return Err(Error::Unsupported("wav compression"));
            }
            fmt = Some((
                channels,
                bits,
                rate,
                u16::from_le_bytes([body[12], body[13]]),
            ));
        } else if id == b"data" {
            pcm = Some(body.to_vec());
        }
        pos += size + (size & 1);
    }
    let (channels, bits, rate, _align) = fmt.ok_or(Error::Corrupt("wav fmt"))?;
    let raw = pcm.ok_or(Error::Corrupt("wav data"))?;
    if channels == 0 || rate == 0 {
        return Err(Error::Corrupt("wav fmt"));
    }
    let bits_u8 = u8::try_from(bits).map_err(|_| Error::Unsupported("bit depth"))?;
    let width = match bits_u8 {
        8 => 1,
        16 => 2,
        24 => 3,
        _ => return Err(Error::Unsupported("bit depth")),
    };
    let frame = width * channels as usize;
    if frame == 0 || raw.len() % frame != 0 {
        return Err(Error::Corrupt("wav data"));
    }
    let mut samples = Vec::with_capacity(raw.len() / width);
    for frame_bytes in raw.chunks(frame) {
        for ch in 0..channels as usize {
            let s = &frame_bytes[ch * width..(ch + 1) * width];
            let sample = match bits_u8 {
                8 => i32::from(s[0]) - 128,
                16 => i32::from(i16::from_le_bytes([s[0], s[1]])),
                24 => {
                    let u = u32::from(s[0]) | (u32::from(s[1]) << 8) | (u32::from(s[2]) << 16);
                    let sign = if s[2] & 0x80 != 0 { 0xff00_0000 } else { 0 };
                    (u | sign) as i32
                }
                _ => return Err(Error::Unsupported("bit depth")),
            };
            samples.push(sample);
        }
    }
    Ok(Pcm {
        sample_rate: rate,
        channels,
        bits: bits_u8,
        samples,
    })
}

pub fn encode(pcm: &Pcm) -> Result<Vec<u8>> {
    encode_with_info(pcm, &[])
}

/// 16-bit PCM WAV. `fields` are INFO ids in the order FFmpeg writes them.
/// `ISFT` is always `Lavf60.16.100` and is not taken from `fields`.
pub fn encode_with_info(pcm: &Pcm, fields: &[([u8; 4], &[u8])]) -> Result<Vec<u8>> {
    let width = match pcm.bits {
        8 => 1usize,
        16 => 2,
        24 => 3,
        _ => return Err(Error::Unsupported("bit depth")),
    };
    let channels = pcm.channels as usize;
    if channels == 0 || pcm.samples.len() % channels != 0 {
        return Err(Error::Corrupt("wav channels"));
    }
    let frames = pcm.samples.len() / channels;
    let data_len = frames * channels * width;
    let list = if pcm.bits == 16 {
        info_list(fields)
    } else {
        Vec::new()
    };
    let mut out = Vec::with_capacity(44 + list.len() + data_len);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&((36 + list.len() + data_len) as u32).to_le_bytes());
    out.extend_from_slice(b"WAVE");
    out.extend_from_slice(b"fmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&(channels as u16).to_le_bytes());
    out.extend_from_slice(&pcm.sample_rate.to_le_bytes());
    let byte_rate = pcm.sample_rate * channels as u32 * width as u32;
    out.extend_from_slice(&byte_rate.to_le_bytes());
    out.extend_from_slice(&((channels * width) as u16).to_le_bytes());
    out.extend_from_slice(&u16::from(pcm.bits).to_le_bytes());
    out.extend_from_slice(&list);
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data_len as u32).to_le_bytes());
    for frame in 0..frames {
        for ch in 0..channels {
            let s = pcm.samples[frame * channels + ch];
            match pcm.bits {
                8 => {
                    let u = (s + 128).clamp(0, 255) as u8;
                    out.push(u);
                }
                16 => {
                    let v = s.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16;
                    out.extend_from_slice(&v.to_le_bytes());
                }
                24 => {
                    let v = s.clamp(-8_388_608, 8_388_607) as u32;
                    out.push((v & 0xff) as u8);
                    out.push(((v >> 8) & 0xff) as u8);
                    out.push(((v >> 16) & 0xff) as u8);
                }
                _ => return Err(Error::Unsupported("bit depth")),
            }
        }
    }
    Ok(out)
}

/// `LIST`/`INFO` placed before `data`. Each value is NUL-terminated.
/// An odd field is padded inside the LIST body. `ISFT` is `Lavf60.16.100`.
fn info_list(fields: &[([u8; 4], &[u8])]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(b"INFO");
    for (id, value) in fields {
        if value.is_empty() || *id == *b"ISFT" {
            continue;
        }
        let mut text = value.to_vec();
        text.push(0);
        push_info(&mut body, id, &text);
    }
    push_info(&mut body, b"ISFT", b"Lavf60.16.100\x00");
    let mut out = Vec::with_capacity(8 + body.len());
    out.extend_from_slice(b"LIST");
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.extend_from_slice(&body);
    out
}

fn push_info(body: &mut Vec<u8>, id: &[u8; 4], data: &[u8]) {
    body.extend_from_slice(id);
    body.extend_from_slice(&(data.len() as u32).to_le_bytes());
    body.extend_from_slice(data);
    if data.len() % 2 == 1 {
        body.push(0);
    }
}
