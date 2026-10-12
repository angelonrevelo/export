//! One gray waveform frame muxed with the PCM audio track.
//!
//! The shipped file is a single uncompressed video sample whose duration
//! equals the audio. A 30 fps repeat exists only so a bench can time that
//! cost. There is no H.264 or AV1 encoder.

use crate::bmff::{build_movie, MovieSpec, TrackSpec};
use crate::error::{Error, Result};
use crate::sniff::{self, MediaKind};
use crate::wav;

/// Width of the gray frame, in pixels.
pub const WIDTH: u16 = 640;
/// Height of the gray frame, in pixels.
pub const HEIGHT: u16 = 120;

const BG: u8 = 32;
const FG: u8 = 220;

/// Uncompressed audiogram. `video_samples` is 1 for [`audiogram`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Audiogram {
    pub data: Vec<u8>,
    pub bytes: u64,
    pub video_samples: u32,
}

/// Draw the waveform once and mux that frame with the PCM track.
///
/// The video sample entry is `raw ` (24-bit RGB, R = G = B). Its delta
/// covers the whole WAV. This does not emit 30 frames.
pub fn audiogram(wav: &[u8]) -> Result<Audiogram> {
    let prepared = prepare(wav)?;
    let deltas = vec![prepared.audio_frames];
    mux(&prepared, deltas, prepared.rate)
}

/// Repeat the same gray frame `frames` times at `fps`.
///
/// [`audiogram`] is the file this crate ships. This function is the
/// comparison: 30 frames at 30 fps is one second of duplicated pixels.
pub fn audiogram_repeat(wav: &[u8], frames: u32, fps: u32) -> Result<Audiogram> {
    if frames == 0 || fps == 0 {
        return Err(Error::Unsupported("audiogram repeat needs frames and fps"));
    }
    let prepared = prepare(wav)?;
    let deltas = vec![1u32; frames as usize];
    mux(&prepared, deltas, fps)
}

struct Prepared {
    frame: Vec<u8>,
    pcm: Vec<u8>,
    channels: u16,
    rate: u32,
    audio_frames: u32,
}

fn prepare(input: &[u8]) -> Result<Prepared> {
    let kind = sniff::sniff(input);
    if kind != MediaKind::Wav {
        return Err(Error::Kind {
            found: kind,
            job: "audiogram",
        });
    }
    let wav = wav::decode(input)?;
    if wav.spec.bits != 16 && wav.spec.bits != 8 {
        return Err(Error::Unsupported("audiogram needs pcm wav"));
    }
    let channels = wav.spec.channels;
    let rate = wav.spec.sample_rate;
    let bps = usize::from(wav.spec.bits / 8);
    let stride = bps * usize::from(channels);
    if stride == 0 || wav.pcm.len() % stride != 0 {
        return Err(Error::Corrupt("wav pcm alignment"));
    }
    let audio_frames = wav.pcm.len() / stride;
    if audio_frames == 0 {
        return Err(Error::Unsupported("audiogram wav has no samples"));
    }
    let audio_frames = u32::try_from(audio_frames)
        .map_err(|_| Error::Unsupported("audiogram longer than 2^32 samples"))?;
    let frame = draw_gray(wav.pcm, wav.spec.bits, channels);
    let pcm = pcm_s16le(wav.pcm, wav.spec.bits);
    Ok(Prepared {
        frame,
        pcm,
        channels,
        rate,
        audio_frames,
    })
}

fn mux(prepared: &Prepared, deltas: Vec<u32>, video_timescale: u32) -> Result<Audiogram> {
    let copies = deltas.len();
    let video_bytes = prepared
        .frame
        .len()
        .checked_mul(copies)
        .ok_or(Error::Unsupported("audiogram frame repeat"))?;
    let total = video_bytes
        .checked_add(prepared.pcm.len())
        .ok_or(Error::Unsupported("audiogram larger than 4 GiB"))?;
    if total > (u32::MAX as usize).saturating_sub(8) {
        return Err(Error::Unsupported("mdat larger than 4 GiB"));
    }
    let mut payloads = Vec::with_capacity(copies);
    for _ in 0..copies {
        payloads.push(prepared.frame.clone());
    }
    let video_samples =
        u32::try_from(copies).map_err(|_| Error::Unsupported("audiogram frame repeat"))?;
    let data = build_movie(&MovieSpec {
        brand: *b"isom",
        timescale: prepared.rate,
        faststart: true,
        interleave: false,
        tracks: vec![
            TrackSpec {
                handler: *b"vide",
                codec: *b"raw ",
                timescale: video_timescale,
                width: WIDTH,
                height: HEIGHT,
                channels: 0,
                bits: 0,
                sample_rate: 0,
                deltas,
                payloads,
            },
            TrackSpec {
                handler: *b"soun",
                codec: *b"sowt",
                timescale: prepared.rate,
                width: 0,
                height: 0,
                channels: prepared.channels,
                bits: 16,
                sample_rate: prepared.rate,
                deltas: vec![prepared.audio_frames],
                payloads: vec![prepared.pcm.clone()],
            },
        ],
    })?;
    let bytes = data.len() as u64;
    Ok(Audiogram {
        data,
        bytes,
        video_samples,
    })
}

fn pcm_s16le(pcm: &[u8], bits: u16) -> Vec<u8> {
    if bits == 16 {
        return pcm.to_vec();
    }
    let mut out = Vec::with_capacity(pcm.len() * 2);
    for &b in pcm {
        let s = (i16::from(b) - 128) << 8;
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

fn draw_gray(pcm: &[u8], bits: u16, channels: u16) -> Vec<u8> {
    let w = usize::from(WIDTH);
    let h = usize::from(HEIGHT);
    let ch = usize::from(channels);
    let bps = usize::from(bits / 8);
    let frames = pcm.len() / (bps * ch);
    let mut rgb = vec![BG; w * h * 3];
    let mid = h / 2;
    for x in 0..w {
        let peak = column_peak(pcm, bits, ch, frames, x, w);
        let amp = (usize::try_from(peak).unwrap_or(0) * mid) / 32768;
        let y0 = mid.saturating_sub(amp);
        let y1 = (mid + amp).min(h - 1);
        for y in y0..=y1 {
            let o = (y * w + x) * 3;
            rgb[o] = FG;
            rgb[o + 1] = FG;
            rgb[o + 2] = FG;
        }
    }
    rgb
}

fn column_peak(pcm: &[u8], bits: u16, channels: usize, frames: usize, x: usize, w: usize) -> u32 {
    if frames == 0 || w == 0 {
        return 0;
    }
    let start = x * frames / w;
    let mut end = (x + 1) * frames / w;
    if end <= start {
        end = (start + 1).min(frames);
    }
    let mut peak = 0u32;
    for frame in start..end {
        for c in 0..channels {
            let amp = sample_abs(pcm, bits, channels, frame, c);
            if amp > peak {
                peak = amp;
            }
        }
    }
    peak
}

fn sample_abs(pcm: &[u8], bits: u16, channels: usize, frame: usize, channel: usize) -> u32 {
    let i = frame * channels + channel;
    if bits == 16 {
        let o = i * 2;
        let s = i16::from_le_bytes([pcm[o], pcm[o + 1]]);
        i32::from(s).unsigned_abs()
    } else {
        let s = (i16::from(pcm[i]) - 128) << 8;
        i32::from(s).unsigned_abs()
    }
}
