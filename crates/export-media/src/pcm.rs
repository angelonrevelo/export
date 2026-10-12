//! Integer linear resample and channel downmix. This is the PCM convert path
//! used when a WAV (or PCM-in-MP4) job asks for a different rate or mono.

use crate::error::{Error, Result};

pub fn downmix_s16(input: &[i16], channels: u16) -> Result<Vec<i16>> {
    let ch = channels as usize;
    if ch == 0 {
        return Err(Error::Corrupt("zero channels"));
    }
    if input.len() % ch != 0 {
        return Err(Error::Corrupt("pcm not aligned to channels"));
    }
    if ch == 1 {
        return Ok(input.to_vec());
    }
    let frames = input.len() / ch;
    let mut out = Vec::with_capacity(frames);
    for frame in 0..frames {
        let mut acc: i32 = 0;
        for c in 0..ch {
            acc += i32::from(input[frame * ch + c]);
        }
        out.push((acc / ch as i32).clamp(-32768, 32767) as i16);
    }
    Ok(out)
}

/// Linear interpolation in integer arithmetic. `input` is interleaved.
/// Output frame count is `in_frames * dst_rate / src_rate`.
pub fn resample_s16(
    input: &[i16],
    channels: u16,
    src_rate: u32,
    dst_rate: u32,
) -> Result<Vec<i16>> {
    let ch = channels as usize;
    if ch == 0 || src_rate == 0 || dst_rate == 0 {
        return Err(Error::Corrupt("bad resample args"));
    }
    if input.len() % ch != 0 {
        return Err(Error::Corrupt("pcm not aligned to channels"));
    }
    if src_rate == dst_rate {
        return Ok(input.to_vec());
    }
    let in_frames = input.len() / ch;
    if in_frames == 0 {
        return Ok(Vec::new());
    }
    let out_frames = ((in_frames as u64) * u64::from(dst_rate)) / u64::from(src_rate);
    if out_frames == 0 {
        return Ok(Vec::new());
    }
    let out_frames = out_frames as usize;
    let mut out = vec![0i16; out_frames * ch];
    let src = u64::from(src_rate);
    let dst = u64::from(dst_rate);
    for i in 0..out_frames {
        let pos = (i as u64) * src;
        let mut i0 = (pos / dst) as usize;
        if i0 >= in_frames {
            i0 = in_frames - 1;
        }
        let i1 = (i0 + 1).min(in_frames - 1);
        let frac = pos % dst;
        for c in 0..ch {
            let a = i64::from(input[i0 * ch + c]);
            let b = i64::from(input[i1 * ch + c]);
            let v = a + ((b - a) * frac as i64) / dst as i64;
            out[i * ch + c] = v.clamp(-32768, 32767) as i16;
        }
    }
    Ok(out)
}

pub fn s16_le(samples: &[i16]) -> Vec<u8> {
    let mut out = Vec::with_capacity(samples.len() * 2);
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

pub fn s16_from_le(bytes: &[u8]) -> Result<Vec<i16>> {
    if bytes.len() % 2 != 0 {
        return Err(Error::Corrupt("pcm byte length"));
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        out.push(i16::from_le_bytes([chunk[0], chunk[1]]));
    }
    Ok(out)
}

pub fn s16_from_be(bytes: &[u8]) -> Result<Vec<i16>> {
    if bytes.len() % 2 != 0 {
        return Err(Error::Corrupt("pcm byte length"));
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for chunk in bytes.chunks_exact(2) {
        out.push(i16::from_be_bytes([chunk[0], chunk[1]]));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_survives_resample_and_downmix() {
        let stereo: Vec<i16> = (0..1000).flat_map(|_| [1000, 1000]).collect();
        let mono = downmix_s16(&stereo, 2).unwrap();
        assert_eq!(mono.len(), 1000);
        assert!(mono.iter().all(|s| *s == 1000));
        let out = resample_s16(&mono, 1, 44100, 16000).unwrap();
        assert_eq!(out.len(), 1000 * 16000 / 44100);
        assert!(out.iter().all(|s| *s == 1000));
    }
}
