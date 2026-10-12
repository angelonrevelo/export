//! Stream-copy WebM and Matroska.
//!
//! This is a stream copy, not a transcode, and not a video shrink.
//! The pipeline hashes each input once (SHA-256), dedupes by that hash,
//! sniffs EBML, then copies block payloads and timestamps.
//! VP8, VP9, Opus, and Vorbis block payloads are copied and are not decoded.

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

/// WebM file whose audio payload is Opus. The bytes are copied, not decoded.
pub fn opus_webm() -> &'static [u8] {
    include_bytes!("../fixtures/opus.webm")
}

/// WebM file whose audio payload is Vorbis. The bytes are copied, not decoded.
pub fn vorbis_webm() -> &'static [u8] {
    include_bytes!("../fixtures/vorbis.webm")
}

/// One WebM file carrying the Opus track from [`opus_webm`] and the Vorbis
/// track from [`vorbis_webm`]. Both payloads are copied and neither is decoded.
pub fn opus_vorbis_webm() -> Vec<u8> {
    let opus = demux(opus_webm()).expect("opus fixture");
    let vorbis = demux(vorbis_webm()).expect("vorbis fixture");
    let opus_track = opus.tracks.into_iter().next().expect("opus track");
    let mut vorbis_track = vorbis.tracks.into_iter().next().expect("vorbis track");
    let vorbis_src = vorbis_track.number;
    if vorbis_track.number == opus_track.number {
        vorbis_track.number = opus_track.number.saturating_add(1);
    }
    if vorbis_track.uid == opus_track.uid {
        vorbis_track.uid = opus_track.uid.saturating_add(1);
    }
    let mut blocks = opus.blocks;
    for mut block in vorbis.blocks {
        if block.track == vorbis_src {
            block.track = vorbis_track.number;
        }
        blocks.push(block);
    }
    let movie = Movie {
        doctype: "webm".to_owned(),
        timestamp_scale: opus.timestamp_scale,
        tracks: vec![opus_track, vorbis_track],
        blocks,
    };
    remux(&movie).expect("opus and vorbis webm")
}

/// One WebM file carrying the VP8 track from [`vp8_webm`] and the Opus track
/// from [`opus_webm`]. Both payloads are copied and neither is decoded.
pub fn vp8_opus_webm() -> Vec<u8> {
    let vp8 = demux(vp8_webm()).expect("vp8 fixture");
    let opus = demux(opus_webm()).expect("opus fixture");
    let vp8_track = vp8.tracks.into_iter().next().expect("vp8 track");
    let mut opus_track = opus.tracks.into_iter().next().expect("opus track");
    let opus_src = opus_track.number;
    if opus_track.number == vp8_track.number {
        opus_track.number = vp8_track.number.saturating_add(1);
    }
    if opus_track.uid == vp8_track.uid {
        opus_track.uid = vp8_track.uid.saturating_add(1);
    }
    let mut blocks = vp8.blocks;
    for mut block in opus.blocks {
        if block.track == opus_src {
            block.track = opus_track.number;
        }
        blocks.push(block);
    }
    let movie = Movie {
        doctype: "webm".to_owned(),
        timestamp_scale: vp8.timestamp_scale,
        tracks: vec![vp8_track, opus_track],
        blocks,
    };
    remux(&movie).expect("vp8 and opus webm")
}

/// One WebM file carrying the 460800-byte VP8 payload from [`vp8_460800_webm`]
/// and the Opus packets from [`opus_webm`]. Both payloads are copied and
/// neither is decoded.
pub fn vp8_460800_opus_webm() -> Vec<u8> {
    let vp8 = demux(&vp8_460800_webm()).expect("460800-byte vp8");
    let opus = demux(opus_webm()).expect("opus fixture");
    let vp8_track = vp8.tracks.into_iter().next().expect("vp8 track");
    let mut opus_track = opus.tracks.into_iter().next().expect("opus track");
    let opus_src = opus_track.number;
    if opus_track.number == vp8_track.number {
        opus_track.number = vp8_track.number.saturating_add(1);
    }
    if opus_track.uid == vp8_track.uid {
        opus_track.uid = vp8_track.uid.saturating_add(1);
    }
    let mut blocks = vp8.blocks;
    for mut block in opus.blocks {
        if block.track == opus_src {
            block.track = opus_track.number;
        }
        blocks.push(block);
    }
    let movie = Movie {
        doctype: "webm".to_owned(),
        timestamp_scale: vp8.timestamp_scale,
        tracks: vec![vp8_track, opus_track],
        blocks,
    };
    remux(&movie).expect("460800-byte vp8 and opus webm")
}

/// One WebM file carrying the 460800-byte VP8 payload from [`vp8_460800_webm`],
/// the Opus packets from [`opus_webm`], and the Vorbis track from
/// [`vorbis_webm`]. The payloads are copied and none is decoded.
pub fn vp8_460800_opus_vorbis_webm() -> Vec<u8> {
    let vp8 = demux(&vp8_460800_webm()).expect("460800-byte vp8");
    let opus = demux(opus_webm()).expect("opus fixture");
    let vorbis = demux(vorbis_webm()).expect("vorbis fixture");
    let vp8_track = vp8.tracks.into_iter().next().expect("vp8 track");
    let mut opus_track = opus.tracks.into_iter().next().expect("opus track");
    let mut vorbis_track = vorbis.tracks.into_iter().next().expect("vorbis track");
    let opus_src = opus_track.number;
    let vorbis_src = vorbis_track.number;
    let mut numbers = vec![vp8_track.number];
    let mut uids = vec![vp8_track.uid];
    take_fresh(&mut opus_track.number, &mut numbers);
    take_fresh(&mut opus_track.uid, &mut uids);
    take_fresh(&mut vorbis_track.number, &mut numbers);
    take_fresh(&mut vorbis_track.uid, &mut uids);
    let video_blocks = vp8.blocks.len();
    let mut blocks = vp8.blocks;
    for mut block in opus.blocks {
        if block.track == opus_src {
            block.track = opus_track.number;
        }
        blocks.push(block);
    }
    for mut block in vorbis.blocks {
        if block.track == vorbis_src {
            block.track = vorbis_track.number;
        }
        blocks.push(block);
    }
    // Video stays in the first cluster. Audio opens a second cluster.
    // Absolute timestamps stay the same. Payloads are not changed.
    open_second_cluster(&mut blocks, video_blocks);
    let movie = Movie {
        doctype: "webm".to_owned(),
        timestamp_scale: vp8.timestamp_scale,
        tracks: vec![vp8_track, opus_track, vorbis_track],
        blocks,
    };
    remux(&movie).expect("460800-byte vp8, opus, and vorbis webm")
}

fn open_second_cluster(blocks: &mut [MediaBlock], video_blocks: usize) {
    if video_blocks == 0 || blocks.len() <= video_blocks {
        return;
    }
    let first = blocks[0].cluster_timestamp;
    if blocks[video_blocks].cluster_timestamp != first {
        return;
    }
    let second = first.saturating_add(1);
    let second_i64 = i64::try_from(second).expect("cluster timestamp");
    for block in &mut blocks[video_blocks..] {
        let rel = block.timestamp() - second_i64;
        block.cluster_timestamp = second;
        block.rel_timestamp = i16::try_from(rel).expect("audio relative timestamp");
    }
}

fn take_fresh(value: &mut u64, used: &mut Vec<u64>) {
    if used.contains(value) {
        let mut next = value.saturating_add(1);
        while used.contains(&next) {
            next = next.saturating_add(1);
        }
        *value = next;
    }
    used.push(*value);
}

/// WebM file whose video payload is VP9. The bytes are copied, not decoded.
pub fn vp9_webm() -> &'static [u8] {
    include_bytes!("../fixtures/vp9.webm")
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

/// Padding. One WebM block whose payload is the small VP9 frame from
/// [`vp9_webm`] followed by zero bytes out to 4096. This is not a larger
/// encoded picture. The payload is copied and is not decoded.
pub fn vp9_4096_webm() -> Vec<u8> {
    let parsed = demux(vp9_webm()).expect("one-block vp9 fixture");
    let frame = &parsed.blocks[0].payload;
    assert!(frame.len() < 4096);
    let mut payload = vec![0u8; 4096];
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
    remux(&movie).expect("padded vp9 webm")
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
    fn vp9_payload_is_copied() {
        let original = vp9_webm();
        let parsed = demux(original).unwrap();
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP9);
        assert!(!parsed.blocks.is_empty());
        let copied = stream_copy(original).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_VP9);
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    #[test]
    fn vp9_4096_payload_is_copied() {
        let original_frame = demux(vp9_webm()).unwrap().blocks[0].payload.clone();
        let file = vp9_4096_webm();
        let parsed = demux(&file).unwrap();
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP9);
        assert_eq!(parsed.blocks.len(), 1);
        assert_eq!(parsed.blocks[0].payload.len(), 4096);
        assert_eq!(
            &parsed.blocks[0].payload[..original_frame.len()],
            &original_frame[..]
        );
        let copied = stream_copy(&file).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_VP9);
        assert_eq!(again.blocks[0].payload, parsed.blocks[0].payload);
        assert_eq!(again.blocks[0].timestamp(), parsed.blocks[0].timestamp());
    }

    #[test]
    fn opus_payload_is_copied() {
        let original = opus_webm();
        let parsed = demux(original).unwrap();
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(parsed.tracks[0].track_type, 2);
        assert!(!parsed.blocks.is_empty());
        let copied = stream_copy(original).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(
            again.tracks[0].codec_private,
            parsed.tracks[0].codec_private
        );
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    #[test]
    fn vorbis_payload_is_copied() {
        let original = vorbis_webm();
        let parsed = demux(original).unwrap();
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VORBIS);
        assert_eq!(parsed.tracks[0].track_type, 2);
        assert!(!parsed.blocks.is_empty());
        let copied = stream_copy(original).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_VORBIS);
        assert_eq!(
            again.tracks[0].codec_private,
            parsed.tracks[0].codec_private
        );
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    #[test]
    fn opus_and_vorbis_are_copied() {
        let opus = demux(opus_webm()).unwrap();
        let vorbis = demux(vorbis_webm()).unwrap();
        let file = opus_vorbis_webm();
        let parsed = demux(&file).unwrap();
        assert_eq!(parsed.tracks.len(), 2);
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(parsed.tracks[1].codec_id, crate::ebml::CODEC_VORBIS);
        assert_eq!(parsed.tracks[0].codec_private, opus.tracks[0].codec_private);
        assert_eq!(
            parsed.tracks[1].codec_private,
            vorbis.tracks[0].codec_private
        );
        let opus_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|block| block.track == parsed.tracks[0].number)
            .collect();
        let vorbis_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|block| block.track == parsed.tracks[1].number)
            .collect();
        assert_eq!(opus_blocks.len(), opus.blocks.len());
        assert_eq!(vorbis_blocks.len(), vorbis.blocks.len());
        for (left, right) in opus.blocks.iter().zip(opus_blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
        for (left, right) in vorbis.blocks.iter().zip(vorbis_blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
        let copied = stream_copy(&file).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(again.tracks[1].codec_id, crate::ebml::CODEC_VORBIS);
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    #[test]
    fn vp8_and_opus_are_copied() {
        let vp8 = demux(vp8_webm()).unwrap();
        let opus = demux(opus_webm()).unwrap();
        let file = vp8_opus_webm();
        let parsed = demux(&file).unwrap();
        assert_eq!(parsed.tracks.len(), 2);
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(parsed.tracks[1].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(parsed.tracks[0].codec_private, vp8.tracks[0].codec_private);
        assert_eq!(parsed.tracks[1].codec_private, opus.tracks[0].codec_private);
        let vp8_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|block| block.track == parsed.tracks[0].number)
            .collect();
        let opus_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|block| block.track == parsed.tracks[1].number)
            .collect();
        assert_eq!(vp8_blocks.len(), vp8.blocks.len());
        assert_eq!(opus_blocks.len(), opus.blocks.len());
        for (left, right) in vp8.blocks.iter().zip(vp8_blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
        for (left, right) in opus.blocks.iter().zip(opus_blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
        let copied = stream_copy(&file).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(again.tracks[1].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    #[test]
    fn vp8_460800_and_opus_are_copied() {
        let vp8 = demux(&vp8_460800_webm()).unwrap();
        let opus = demux(opus_webm()).unwrap();
        let file = vp8_460800_opus_webm();
        let parsed = demux(&file).unwrap();
        assert_eq!(parsed.tracks.len(), 2);
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(parsed.tracks[1].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(
            parsed
                .blocks
                .iter()
                .filter(|b| b.track == parsed.tracks[0].number)
                .count(),
            1
        );
        assert_eq!(parsed.blocks[0].payload.len(), 460_800);
        assert_eq!(parsed.blocks[0].payload, vp8.blocks[0].payload);
        assert_eq!(parsed.blocks[0].timestamp(), vp8.blocks[0].timestamp());
        let opus_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|block| block.track == parsed.tracks[1].number)
            .collect();
        assert_eq!(opus_blocks.len(), opus.blocks.len());
        for (left, right) in opus.blocks.iter().zip(opus_blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
        let copied = stream_copy(&file).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(again.tracks[1].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
    }

    fn has_cues(bytes: &[u8]) -> bool {
        bytes
            .windows(4)
            .any(|window| window == [0x1C, 0x53, 0xBB, 0x6B])
    }

    fn clusters_and_cues(bytes: &[u8]) -> (Vec<u64>, Vec<(u64, u64)>) {
        let top = children(bytes);
        let (_, seg_start, seg_end) = top
            .iter()
            .find(|element| element.0 == 0x1853_8067)
            .copied()
            .expect("segment");
        let segment = &bytes[seg_start..seg_end];
        let mut clusters = Vec::new();
        let mut cues = Vec::new();
        let mut cursor = 0;
        while cursor < segment.len() {
            let element_start = cursor as u64;
            let (id, after_id) = read_id(segment, cursor);
            let (len, body_start) = read_size(segment, after_id);
            let body = &segment[body_start..body_start + len];
            if id == crate::ebml::ID_CLUSTER {
                clusters.push(element_start);
            }
            if id == crate::ebml::ID_CUES {
                for (point_id, point_start, point_end) in children(body) {
                    if point_id != crate::ebml::ID_CUE_POINT {
                        continue;
                    }
                    let point = &body[point_start..point_end];
                    let mut time = None;
                    let mut position = None;
                    for (child_id, child_start, child_end) in children(point) {
                        if child_id == crate::ebml::ID_CUE_TIME {
                            time = Some(read_uint(&point[child_start..child_end]));
                        }
                        if child_id == crate::ebml::ID_CUE_TRACK_POSITIONS {
                            let positions = &point[child_start..child_end];
                            for (slot_id, slot_start, slot_end) in children(positions) {
                                if slot_id == crate::ebml::ID_CUE_CLUSTER_POSITION {
                                    position = Some(read_uint(&positions[slot_start..slot_end]));
                                }
                            }
                        }
                    }
                    cues.push((time.expect("cue time"), position.expect("cue position")));
                }
            }
            cursor = body_start + len;
        }
        (clusters, cues)
    }

    fn children(body: &[u8]) -> Vec<(u64, usize, usize)> {
        let mut out = Vec::new();
        let mut cursor = 0;
        while cursor < body.len() {
            let (id, after_id) = read_id(body, cursor);
            let (len, start) = read_size(body, after_id);
            let end = start + len;
            out.push((id, start, end));
            cursor = end;
        }
        out
    }

    fn read_id(bytes: &[u8], index: usize) -> (u64, usize) {
        let first = bytes[index];
        let mut width = 1usize;
        let mut mask = 0x80u8;
        while width < 4 && first & mask == 0 {
            width += 1;
            mask >>= 1;
        }
        let mut raw = 0u64;
        for byte in &bytes[index..index + width] {
            raw = (raw << 8) | u64::from(*byte);
        }
        (raw, index + width)
    }

    fn read_size(bytes: &[u8], index: usize) -> (usize, usize) {
        let first = bytes[index];
        let mut width = 1usize;
        let mut mask = 0x80u8;
        while width < 8 && first & mask == 0 {
            width += 1;
            mask >>= 1;
        }
        let mut raw = 0u64;
        for byte in &bytes[index..index + width] {
            raw = (raw << 8) | u64::from(*byte);
        }
        let value = raw & ((1u64 << (7 * width)) - 1);
        (value as usize, index + width)
    }

    fn read_uint(body: &[u8]) -> u64 {
        let mut value = 0u64;
        for byte in body {
            value = (value << 8) | u64::from(*byte);
        }
        value
    }

    fn cluster_count(blocks: &[crate::MediaBlock]) -> usize {
        let mut count = 0;
        let mut previous = None;
        for block in blocks {
            if previous != Some(block.cluster_timestamp) {
                count += 1;
                previous = Some(block.cluster_timestamp);
            }
        }
        count
    }

    #[test]
    fn vp8_460800_opus_and_vorbis_are_copied() {
        let vp8 = demux(&vp8_460800_webm()).unwrap();
        let opus = demux(opus_webm()).unwrap();
        let vorbis = demux(vorbis_webm()).unwrap();
        let file = vp8_460800_opus_vorbis_webm();
        let parsed = demux(&file).unwrap();
        assert_eq!(parsed.tracks.len(), 3);
        assert_eq!(parsed.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(parsed.tracks[1].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(parsed.tracks[2].codec_id, crate::ebml::CODEC_VORBIS);
        let (clusters, cues) = clusters_and_cues(&file);
        assert_eq!(clusters.len(), 2);
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].1, clusters[0]);
        assert_eq!(cues[1].1, clusters[1]);
        assert_ne!(clusters[0], clusters[1]);
        assert_eq!(cluster_count(&parsed.blocks), 2);
        assert!(!has_cues(&vp8_460800_opus_webm()));
        let vp8_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|block| block.track == parsed.tracks[0].number)
            .collect();
        let opus_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|block| block.track == parsed.tracks[1].number)
            .collect();
        let vorbis_blocks: Vec<_> = parsed
            .blocks
            .iter()
            .filter(|block| block.track == parsed.tracks[2].number)
            .collect();
        assert_eq!(vp8_blocks.len(), 1);
        assert_eq!(vp8_blocks[0].payload.len(), 460_800);
        assert_eq!(vp8_blocks[0].payload, vp8.blocks[0].payload);
        assert_eq!(vp8_blocks[0].timestamp(), vp8.blocks[0].timestamp());
        assert_eq!(opus_blocks.len(), opus.blocks.len());
        assert_eq!(vorbis_blocks.len(), vorbis.blocks.len());
        for (left, right) in opus.blocks.iter().zip(opus_blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
        for (left, right) in vorbis.blocks.iter().zip(vorbis_blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
        }
        let copied = stream_copy(&file).unwrap();
        let again = demux(&copied.bytes).unwrap();
        assert_eq!(again.tracks[0].codec_id, crate::ebml::CODEC_VP8);
        assert_eq!(again.tracks[1].codec_id, crate::ebml::CODEC_OPUS);
        assert_eq!(again.tracks[2].codec_id, crate::ebml::CODEC_VORBIS);
        let (copied_clusters, copied_cues) = clusters_and_cues(&copied.bytes);
        assert_eq!(copied_clusters.len(), 2);
        assert_eq!(copied_cues.len(), 2);
        assert_eq!(copied_cues[0].1, copied_clusters[0]);
        assert_eq!(copied_cues[1].1, copied_clusters[1]);
        assert_eq!(cluster_count(&again.blocks), 2);
        assert_eq!(again.blocks.len(), parsed.blocks.len());
        for (left, right) in parsed.blocks.iter().zip(again.blocks.iter()) {
            assert_eq!(left.payload, right.payload);
            assert_eq!(left.timestamp(), right.timestamp());
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
