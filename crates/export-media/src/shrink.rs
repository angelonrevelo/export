//! One-pass WAV shrink. Mono downmix and one integer resample, then stop.
//! The returned byte count is the file that was written. Nothing is padded
//! up to the budget, and no second encode is tried.

use crate::error::{Error, Result};
use crate::pcm::{self, downmix_s16, resample_s16};
use crate::sniff::{self, MediaKind};
use crate::wav::{self, WavSpec};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WavShrink {
    pub data: Vec<u8>,
    pub bytes: u64,
}

/// Smallest PCM WAV this crate can honestly produce for `budget`.
/// A file already at or under the budget is copied and not rewritten.
pub fn shrink_wav(input: &[u8], budget: u64) -> Result<WavShrink> {
    let kind = sniff::sniff(input);
    if kind != MediaKind::Wav {
        return Err(Error::Kind {
            found: kind,
            job: "wav-shrink",
        });
    }
    if budget < 46 {
        return Err(Error::Unsupported("wav shrink budget is below a pcm frame"));
    }
    if input.len() as u64 <= budget {
        return Ok(WavShrink {
            data: input.to_vec(),
            bytes: input.len() as u64,
        });
    }
    let wav = wav::decode(input)?;
    if wav.spec.bits != 16 {
        return Err(Error::Unsupported("wav shrink needs 16-bit pcm"));
    }
    let samples = wav::to_s16(&wav)?;
    let channels = wav.spec.channels;
    let rate = wav.spec.sample_rate;
    let frames = samples.len() / channels as usize;
    if frames == 0 || channels == 0 {
        return Err(Error::Corrupt("wav has no frames"));
    }
    let header = 44u64;
    let (pcm, out_channels) = if channels > 1 {
        (downmix_s16(&samples, channels)?, 1u16)
    } else {
        (samples, channels)
    };
    let frame_bytes = u64::from(out_channels) * 2;
    let full_bytes = header + (frames as u64) * frame_bytes;
    if full_bytes <= budget {
        return encode(out_channels, rate, &pcm);
    }
    let room = budget - header;
    let max_frames = room / frame_bytes;
    if max_frames == 0 {
        return Err(Error::Unsupported("wav shrink budget is below a pcm frame"));
    }
    let dst = max_frames.saturating_mul(u64::from(rate)) / (frames as u64);
    if dst == 0 || dst > u64::from(u32::MAX) {
        return Err(Error::Unsupported("wav shrink budget is below a pcm frame"));
    }
    let shrunk = resample_s16(&pcm, out_channels, rate, dst as u32)?;
    encode(out_channels, dst as u32, &shrunk)
}

fn encode(channels: u16, sample_rate: u32, samples: &[i16]) -> Result<WavShrink> {
    let data = wav::encode(
        WavSpec {
            channels,
            sample_rate,
            bits: 16,
        },
        &pcm::s16_le(samples),
    )?;
    let bytes = data.len() as u64;
    Ok(WavShrink { data, bytes })
}
