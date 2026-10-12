//! Stream-copy WebM and Matroska.
//!
//! This is a stream copy, not a transcode, and not a video shrink.
//! The pipeline hashes each input once (SHA-256), dedupes by that hash,
//! sniffs EBML, then copies block payloads and timestamps.
//! VP9, Opus, and Vorbis are sniffed and return [`Error::Unsupported`].
//! They are not decoded. VP8 block payloads are copied and are not decoded.

#![forbid(unsafe_code)]

mod demux;
mod ebml;
mod error;
mod remux;
mod sha256;

pub use demux::{demux, sniff, MediaBlock, Movie, Track};
pub use error::Error;
pub use remux::{large_sample_file, large_sample_movie, remux, sample_file, sample_movie};
pub use sha256::sha256;

/// WebM file whose block payload is VP8. The bytes are copied, not decoded.
pub fn vp8_webm() -> &'static [u8] {
    include_bytes!("../fixtures/vp8.webm")
}

/// WebM file with two VP8 blocks. Both payloads are copied and neither is decoded.
pub fn vp8_two_webm() -> &'static [u8] {
    include_bytes!("../fixtures/vp8-two.webm")
}

/// One WebM block whose payload is 4096 bytes and begins with the VP8 frame
/// from [`vp8_webm`]. The payload is copied and is not decoded.
pub fn vp8_4096_webm() -> Vec<u8> {
    vp8_padded_webm(4096)
}

/// One WebM block whose VP8 payload is 460800 bytes, the same payload size as
/// the 320×240 rawvideo sample. The bytes begin with the VP8 frame from
/// [`vp8_webm`]. The payload is copied and is not decoded.
pub fn vp8_460800_webm() -> Vec<u8> {
    vp8_padded_webm(460_800)
}

fn vp8_padded_webm(payload_len: usize) -> Vec<u8> {
    let parsed = demux(vp8_webm()).expect("one-block vp8 fixture");
    let frame = &parsed.blocks[0].payload;
    assert!(frame.len() < payload_len);
    let mut payload = vec![0u8; payload_len];
    payload[..frame.len()].copy_from_slice(frame);
    let track = &parsed.tracks[0];
    let movie = Movie {
        doctype: "webm".to_owned(),
        timestamp_scale: parsed.timestamp_scale,
        tracks: vec![Track {
            number: track.number,
            uid: track.uid,
            track_type: track.track_type,
            codec_id: track.codec_id.clone(),
            codec_private: track.codec_private.clone(),
            pixel_width: track.pixel_width,
            pixel_height: track.pixel_height,
            colour_space: None,
            sampling_frequency: None,
            channels: None,
        }],
        blocks: vec![MediaBlock {
            track: track.number,
            cluster_timestamp: parsed.blocks[0].cluster_timestamp,
            rel_timestamp: parsed.blocks[0].rel_timestamp,
            flags: parsed.blocks[0].flags,
            payload,
        }],
    };
    remux(&movie).expect("padded vp8 webm")
}

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
    /// Otherwise sniff EBML, then stream-copy.
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

/// Sniff the EBML header and codec ids, then copy payloads and timestamps.
///
/// Element bodies are slices of `input`. A size that runs past the buffer is
/// [`Error::Truncated`] and is not allocated.
fn sniff_and_copy(input: &[u8]) -> Result<Vec<u8>, Error> {
    sniff(input)?;
    let movie = demux(input)?;
    remux(&movie)
}

/// Hash once, sniff EBML, then stream-copy. No cross-call cache.
pub fn stream_copy(input: &[u8]) -> Result<Copied, Error> {
    Dedupe::new().copy(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_track_simple_blocks_round_trip_payload_and_timestamps() {
        let original = sample_file();
        let parsed = demux(&original).unwrap();
        let copied = stream_copy(&original).unwrap();
        let again = demux(&copied.bytes).unwrap();

        assert_eq!(parsed.blocks.len(), 3);
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
            assert_eq!(left.track, right.track);
        }
        assert_eq!(copied.sha256, sha256(&original));
    }

    #[test]
    fn large_simple_block_payload_round_trips() {
        let original = large_sample_file();
        let parsed = demux(&original).unwrap();
        let payload_bytes: usize = parsed.blocks.iter().map(|block| block.payload.len()).sum();
        assert!(payload_bytes >= 300 * 1024);
        let copied = stream_copy(&original).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    #[test]
    fn truncated_file_is_an_error() {
        let mut bytes = sample_file();
        assert!(bytes.len() > 8);
        bytes.truncate(bytes.len() / 2);
        assert_eq!(stream_copy(&bytes).unwrap_err(), Error::Truncated);
        bytes.clear();
        assert_eq!(stream_copy(&bytes).unwrap_err(), Error::Truncated);
    }

    #[test]
    fn bad_element_id_is_an_error() {
        assert_eq!(
            stream_copy(&[0x00, 0x01, 0x02, 0x03]).unwrap_err(),
            Error::BadElementId
        );
        assert_eq!(
            stream_copy(&[0xff, 0x81, 0x01]).unwrap_err(),
            Error::BadElementId
        );
        let mut overlong = sample_file();
        let patch_at = overlong
            .windows(4)
            .position(|w| w == [0x1F, 0x43, 0xB6, 0x75])
            .expect("cluster id");
        overlong[patch_at] = 0x00;
        assert_eq!(stream_copy(&overlong).unwrap_err(), Error::BadElementId);
    }

    #[test]
    fn vp8_block_payload_is_copied() {
        let original = vp8_webm();
        let parsed = demux(original).unwrap();
        assert_eq!(parsed.tracks.len(), 1);
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert!(!parsed.blocks.is_empty());
        let copied = stream_copy(original).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    #[test]
    fn vp8_two_blocks_are_copied() {
        let original = vp8_two_webm();
        let parsed = demux(original).unwrap();
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(parsed.blocks.len(), 2);
        assert_ne!(parsed.blocks[0].payload, parsed.blocks[1].payload);
        let copied = stream_copy(original).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(again.blocks.len(), 2);
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    #[test]
    fn vp8_4096_payload_is_copied() {
        let original_frame = demux(vp8_webm()).unwrap().blocks[0].payload.clone();
        let file = vp8_4096_webm();
        let parsed = demux(&file).unwrap();
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].payload.len(), 4096);
        assert_eq!(
            &parsed.blocks[0].payload[..original_frame.len()],
            &original_frame[..]
        );
        let copied = stream_copy(&file).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.blocks[0].payload, parsed.blocks[0].payload);
        assert_eq!(again.blocks[0].timestamp(), parsed.blocks[0].timestamp());
    }

    #[test]
    fn vp8_460800_payload_is_copied() {
        let original_frame = demux(vp8_webm()).unwrap().blocks[0].payload.clone();
        let file = vp8_460800_webm();
        let parsed = demux(&file).unwrap();
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].payload.len(), 460_800);
        assert_eq!(
            &parsed.blocks[0].payload[..original_frame.len()],
            &original_frame[..]
        );
        let copied = stream_copy(&file).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.blocks[0].payload, parsed.blocks[0].payload);
        assert_eq!(again.blocks[0].timestamp(), parsed.blocks[0].timestamp());
    }

    #[test]
    fn vp9_opus_vorbis_are_unsupported() {
        for codec in ["V_VP9", "A_OPUS", "A_VORBIS"] {
            let mut movie = sample_movie();
            movie.tracks[0].codec_id = codec.to_owned();
            if codec.starts_with('A') {
                movie.tracks[0].track_type = 2;
                movie.tracks[0].pixel_width = None;
                movie.tracks[0].pixel_height = None;
                movie.tracks[0].channels = Some(1);
                movie.tracks[0].sampling_frequency = Some(48_000.0);
            }
            let bytes = remux(&movie).unwrap();
            assert_eq!(stream_copy(&bytes).unwrap_err(), Error::Unsupported);
            assert_eq!(demux(&bytes).unwrap_err(), Error::Unsupported);
        }
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

        let bad = [0xffu8, 0x81, 0x01];
        assert_eq!(dedupe.copy(&bad).unwrap_err(), Error::BadElementId);
        assert_eq!(dedupe.copy(&bad).unwrap_err(), Error::BadElementId);
        assert_eq!(dedupe.len(), 2);
    }
}
