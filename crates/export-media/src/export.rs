//! Product entry. Hash once, dedup, sniff, then a codec. A duplicate never parses.

use crate::bmff::{self, RemuxOpt};
use crate::dedup::{self, Dedup, Prior};
use crate::error::{Error, Result};
use crate::pcm::{self, downmix_s16, resample_s16};
use crate::sha256::Sha256;
use crate::sniff::{self, MediaKind};
use crate::wav::{self, WavSpec};
use std::io::Read;

/// Bytes the streaming reader will hold before it refuses the job.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// Stream-copy an ISO BMFF file. `faststart` writes `moov` before `mdat`.
    Remux {
        faststart: bool,
        audio_only: bool,
        video_only: bool,
    },
    /// PCM WAV. Sample-rate and channel changes are the resample / downmix path.
    Wav {
        sample_rate: Option<u32>,
        channels: Option<u16>,
    },
    /// Concatenate one elementary stream. No decode.
    Elementary { audio: bool },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Fresh {
        digest: [u8; 32],
        kind: MediaKind,
        data: Vec<u8>,
    },
    /// This digest was ingested already. The codec did not run.
    Duplicate { digest: [u8; 32] },
}

pub fn export(bytes: &[u8], job: Job, dedup: &mut Dedup) -> Result<Output> {
    export_hashed(bytes, dedup::digest_of(bytes), job, dedup)
}

/// Read at most `max` bytes. The hash is updated in that same pass.
/// Over the cap, returns [`Error::Limit`] before sniff or parse.
pub fn export_reader<R: Read>(
    reader: &mut R,
    max: usize,
    job: Job,
    dedup: &mut Dedup,
) -> Result<Output> {
    let mut hasher = Sha256::new();
    let mut buf = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let n = reader
            .read(&mut chunk)
            .map_err(|_| Error::Corrupt("read"))?;
        if n == 0 {
            break;
        }
        let got = (buf.len() + n) as u64;
        if buf.len() + n > max {
            return Err(Error::Limit {
                cap: max as u64,
                got,
            });
        }
        hasher.update(&chunk[..n]);
        buf.extend_from_slice(&chunk[..n]);
    }
    export_hashed(&buf, hasher.finish(), job, dedup)
}

fn export_hashed(bytes: &[u8], digest: [u8; 32], job: Job, dedup: &mut Dedup) -> Result<Output> {
    if dedup.get(&digest).is_some() {
        return Ok(Output::Duplicate { digest });
    }
    let kind = sniff::sniff(bytes);
    let data = match job {
        Job::Remux {
            faststart,
            audio_only,
            video_only,
        } => {
            if !kind.is_bmff() {
                return Err(Error::Kind {
                    found: kind,
                    job: "remux",
                });
            }
            bmff::remux(
                bytes,
                RemuxOpt {
                    faststart,
                    audio_only,
                    video_only,
                },
            )?
        }
        Job::Wav {
            sample_rate,
            channels,
        } => wav_job(bytes, kind, sample_rate, channels)?,
        Job::Elementary { audio } => elementary_job(bytes, kind, audio)?,
    };
    dedup.insert(
        digest,
        Prior {
            kind,
            len: bytes.len() as u64,
        },
    );
    Ok(Output::Fresh { digest, kind, data })
}

fn wav_job(
    bytes: &[u8],
    kind: MediaKind,
    sample_rate: Option<u32>,
    channels: Option<u16>,
) -> Result<Vec<u8>> {
    let (mut samples, mut ch, src_rate) = match kind {
        MediaKind::Wav => {
            let wav = wav::decode(bytes)?;
            let samples = wav::to_s16(&wav)?;
            (samples, wav.spec.channels, wav.spec.sample_rate)
        }
        other if other.is_bmff() => pcm_from_bmff(bytes)?,
        other => {
            return Err(Error::Kind {
                found: other,
                job: "wav",
            })
        }
    };
    if let Some(want) = channels {
        if want == 1 && ch > 1 {
            samples = downmix_s16(&samples, ch)?;
            ch = 1;
        } else if want != ch {
            return Err(Error::Unsupported("channel count"));
        }
    }
    let dst_rate = sample_rate.unwrap_or(src_rate);
    if dst_rate != src_rate {
        samples = resample_s16(&samples, ch, src_rate, dst_rate)?;
    }
    wav::encode(
        WavSpec {
            channels: ch,
            sample_rate: dst_rate,
            bits: 16,
        },
        &pcm::s16_le(&samples),
    )
}

fn pcm_from_bmff(bytes: &[u8]) -> Result<(Vec<i16>, u16, u32)> {
    let movie = bmff::parse(bytes)?;
    let track = movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"soun")
        .ok_or(Error::Corrupt("no audio track"))?;
    let mut raw = Vec::new();
    bmff::append_samples(bytes, &track.samples, &mut raw)?;
    let samples = match &track.codec {
        b"sowt" => pcm::s16_from_le(&raw)?,
        b"twos" => pcm::s16_from_be(&raw)?,
        _ => {
            return Err(Error::Unsupported(
                "audio codec is not PCM; remux copies the elementary stream",
            ))
        }
    };
    let channels = if track.channels == 0 {
        1
    } else {
        track.channels
    };
    let rate = if track.sample_rate == 0 {
        track.timescale
    } else {
        track.sample_rate
    };
    if samples.len() % channels as usize != 0 {
        return Err(Error::Corrupt("pcm channel alignment"));
    }
    Ok((samples, channels, rate))
}

fn elementary_job(bytes: &[u8], kind: MediaKind, audio: bool) -> Result<Vec<u8>> {
    if !kind.is_bmff() {
        return Err(Error::Kind {
            found: kind,
            job: "elementary",
        });
    }
    let movie = bmff::parse(bytes)?;
    let track = movie
        .tracks
        .iter()
        .find(|t| {
            if audio {
                t.handler == *b"soun"
            } else {
                t.handler == *b"vide"
            }
        })
        .ok_or(Error::Corrupt("track not found"))?;
    let mut out = Vec::new();
    bmff::append_samples(bytes, &track.samples, &mut out)?;
    Ok(out)
}
