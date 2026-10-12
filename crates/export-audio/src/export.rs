//! Product entry. Hash once, dedup, sniff, then the codec.
//! A duplicate returns before sniff. A failed parse is not inserted.

use crate::dedup::{self, Dedup, Prior};
use crate::error::{Error, Result};
use crate::flac;
use crate::sha256::Sha256;
use crate::sniff::{self, Kind};
use crate::wav;
use std::io::Read;

/// Bytes the streaming reader will hold before it refuses the job.
pub const DEFAULT_MAX_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Job {
    /// WAV PCM or FLAC in, subset FLAC out.
    ToFlac,
    /// FLAC in, WAV PCM out.
    ToWav,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Output {
    Fresh {
        digest: [u8; 32],
        kind: Kind,
        data: Vec<u8>,
    },
    /// This digest was ingested already. The codec did not run.
    Duplicate {
        digest: [u8; 32],
    },
}

pub fn export(bytes: &[u8], job: Job, dedup: &mut Dedup) -> Result<Output> {
    export_hashed(bytes, dedup::digest_of(bytes), job, dedup)
}

/// Read at most `max` bytes. The hash is updated in that same pass.
/// Over the cap, returns [`Error::Limit`] before sniff or the codec.
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
        let n = reader.read(&mut chunk).map_err(|_| Error::Corrupt("read"))?;
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
        Job::ToFlac => match kind {
            Kind::Flac => {
                let pcm = flac::decode(bytes)?;
                flac::encode(&pcm)?
            }
            Kind::Wav => {
                let pcm = wav::decode(bytes)?;
                flac::encode(&pcm)?
            }
            Kind::Mp3 => return Err(Error::Unsupported("MP3")),
            Kind::Aac => return Err(Error::Unsupported("AAC")),
            Kind::Opus => return Err(Error::Unsupported("Opus")),
            other => {
                return Err(Error::Kind {
                    found: other,
                    job: "flac",
                })
            }
        },
        Job::ToWav => match kind {
            Kind::Flac => {
                let pcm = flac::decode(bytes)?;
                wav::encode(&pcm)?
            }
            Kind::Mp3 => return Err(Error::Unsupported("MP3")),
            Kind::Aac => return Err(Error::Unsupported("AAC")),
            Kind::Opus => return Err(Error::Unsupported("Opus")),
            other => {
                return Err(Error::Kind {
                    found: other,
                    job: "wav",
                })
            }
        },
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
