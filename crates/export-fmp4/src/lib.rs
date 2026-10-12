//! Stream-copy fragmented MP4.
//!
//! This is a stream copy, not a transcode, and not a video shrink.
//! The pipeline hashes each input once (SHA-256), sniffs the boxes, then
//! copies `moof`/`mdat` sample bytes.
//! AAC, Opus, and VP9 are sniffed and return [`Error::Unsupported`].
//! H.264 `avc1` sample bytes are copied and are not decoded.
//! This crate does not call ffmpeg.

#![forbid(unsafe_code)]

mod atom;
mod demux;
mod error;
mod h264;
mod remux;
mod sha256;
mod sniff;

pub use demux::{demux, Fragment, Movie, Run, Sample, Track};
pub use error::Error;
pub use h264::h264_file;
pub use remux::{remux, sample_file, sample_movie};
pub use sha256::sha256;
pub use sniff::{sniff, Sniff};

use std::collections::BTreeMap;

/// Remuxed bytes plus the single SHA-256 of the input that produced them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Copied {
    pub sha256: [u8; 32],
    pub bytes: Vec<u8>,
}

/// Cache of copied streams, keyed by the one hash of each input.
///
/// A repeated hash returns the earlier result and does not sniff again.
#[derive(Debug, Default)]
pub struct Dedupe {
    by_hash: BTreeMap<[u8; 32], Result<Vec<u8>, Error>>,
}

impl Dedupe {
    pub fn new() -> Self {
        Self {
            by_hash: BTreeMap::new(),
        }
    }

    pub fn len(&self) -> usize {
        self.by_hash.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_hash.is_empty()
    }

    /// Hash `input` once. On a repeated hash, return the earlier result.
    /// Otherwise sniff, then copy `moof`/`mdat` samples.
    pub fn copy(&mut self, input: &[u8]) -> Result<Copied, Error> {
        let digest = sha256(input);
        if let Some(prev) = self.by_hash.get(&digest) {
            return match prev {
                Ok(bytes) => Ok(Copied {
                    sha256: digest,
                    bytes: bytes.clone(),
                }),
                Err(err) => Err(err.clone()),
            };
        }
        let result = sniff_and_copy(input);
        self.by_hash.insert(digest, result.clone());
        match result {
            Ok(bytes) => Ok(Copied {
                sha256: digest,
                bytes,
            }),
            Err(err) => Err(err),
        }
    }
}

/// Sniff brands and sample-entry fourccs, then copy sample bytes.
fn sniff_and_copy(input: &[u8]) -> Result<Vec<u8>, Error> {
    sniff(input)?;
    let movie = demux(input)?;
    remux(&movie)
}

/// Hash once, sniff, then stream-copy. No cross-call cache.
pub fn stream_copy(input: &[u8]) -> Result<Copied, Error> {
    let digest = sha256(input);
    Ok(Copied {
        sha256: digest,
        bytes: sniff_and_copy(input)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payloads(movie: &Movie) -> Vec<Vec<u8>> {
        movie
            .fragments
            .iter()
            .flat_map(|fragment| fragment.runs.iter())
            .flat_map(|run| run.samples.iter().map(|sample| sample.payload.clone()))
            .collect()
    }

    #[test]
    fn copies_moof_mdat_samples() {
        let original = sample_file();
        let parsed = demux(&original).unwrap();
        assert_eq!(parsed.fragments.len(), 2);
        let total: usize = payloads(&parsed).iter().map(Vec::len).sum();
        assert_eq!(total, 1 << 20);
        let copied = stream_copy(&original).unwrap();
        assert_eq!(copied.sha256, sha256(&original));
        assert!(copied.bytes.windows(4).any(|w| w == *b"moof"));
        assert!(copied.bytes.windows(4).any(|w| w == *b"mdat"));
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(payloads(&parsed), payloads(&again));
        for (left, right) in parsed.fragments.iter().zip(again.fragments.iter()) {
            assert_eq!(left.sequence, right.sequence);
            assert_eq!(left.runs.len(), right.runs.len());
            for (left_run, right_run) in left.runs.iter().zip(right.runs.iter()) {
                assert_eq!(
                    left_run.base_media_decode_time,
                    right_run.base_media_decode_time
                );
                for (left_sample, right_sample) in
                    left_run.samples.iter().zip(right_run.samples.iter())
                {
                    assert_eq!(left_sample.duration, right_sample.duration);
                    assert_eq!(left_sample.flags, right_sample.flags);
                    assert_eq!(left_sample.payload, right_sample.payload);
                }
            }
        }
        assert_eq!(sniff(&copied.bytes).unwrap().brand, *b"iso5");
    }

    #[test]
    fn truncated_input_is_an_error() {
        let bytes = sample_file();
        assert!(bytes.len() > 8);
        let mut half = bytes.clone();
        half.truncate(bytes.len() / 2);
        assert_eq!(stream_copy(&half).unwrap_err(), Error::Truncated);
        assert_eq!(stream_copy(&[]).unwrap_err(), Error::Truncated);
        assert_eq!(stream_copy(&[0, 0, 0, 1]).unwrap_err(), Error::Truncated);
        let declared = u32::from_be_bytes(bytes[0..4].try_into().unwrap()) as usize;
        assert!(declared > 1 && declared <= bytes.len());
        assert_eq!(
            stream_copy(&bytes[..declared - 1]).unwrap_err(),
            Error::Truncated
        );
    }

    #[test]
    fn aac_opus_vp9_are_unsupported() {
        for codec in [*b"mp4a", *b"Opus", *b"vp09"] {
            let file = remux::file_with_codec(codec).unwrap();
            assert_eq!(stream_copy(&file).unwrap_err(), Error::Unsupported);
            assert_eq!(demux(&file).unwrap_err(), Error::Unsupported);
        }
    }

    #[test]
    fn avc1_payload_is_copied_not_decoded() {
        let file = remux::file_with_codec(*b"avc1").unwrap();
        let copied = stream_copy(&file).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.fragments[0].runs[0].samples[0].payload, b"opaque");
    }

    #[test]
    fn h264_fragment_samples_round_trip() {
        let original = h264_file();
        let parsed = demux(&original).unwrap();
        assert_eq!(parsed.tracks[0].handler, *b"vide");
        assert_eq!(parsed.tracks[0].width, 16);
        assert_eq!(parsed.tracks[0].height, 16);
        assert!(parsed.tracks[0]
            .sample_entry
            .windows(4)
            .any(|w| w == b"avc1"));
        let copied = stream_copy(&original).unwrap();
        assert_eq!(copied.sha256, sha256(&original));
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(payloads(&parsed), payloads(&again));
        assert_eq!(payloads(&again), vec![h264::h264_payload().to_vec()]);
    }

    #[test]
    fn a_file_without_moof_is_invalid() {
        let mut bytes = sample_file();
        let moof = bytes.windows(4).position(|w| w == *b"moof").unwrap();
        bytes.truncate(moof - 4);
        assert_eq!(stream_copy(&bytes).unwrap_err(), Error::Invalid);
    }

    #[test]
    fn dedupe_hashes_once_per_distinct_input() {
        let file = sample_file();
        let mut dedupe = Dedupe::new();
        let first = dedupe.copy(&file).unwrap();
        let second = dedupe.copy(&file).unwrap();
        assert_eq!(first, second);
        assert_eq!(dedupe.len(), 1);
        assert_eq!(first.sha256, sha256(&file));

        let bad = [0xffu8, 0x00, 0x00, 0x08, b'b', b'a', b'd', b'!'];
        assert!(dedupe.copy(&bad).is_err());
        assert!(dedupe.copy(&bad).is_err());
        assert_eq!(dedupe.len(), 2);
    }
}
