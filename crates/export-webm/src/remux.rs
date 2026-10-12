//! Rebuild a WebM/Matroska stream from demuxed blocks. Bytes are copied.

use crate::demux::{MediaBlock, Movie, Track};
use crate::ebml::{
    element, float64_element, master, string_element, uint_element, ID_AUDIO, ID_CHANNELS,
    ID_CLUSTER, ID_CODEC_ID, ID_CODEC_PRIVATE, ID_COLOUR_SPACE, ID_DOCTYPE,
    ID_DOCTYPE_READ_VERSION, ID_DOCTYPE_VERSION, ID_DURATION, ID_EBML, ID_EBML_MAX_ID_LENGTH,
    ID_EBML_MAX_SIZE_LENGTH, ID_EBML_READ_VERSION, ID_EBML_VERSION, ID_FLAG_DEFAULT,
    ID_FLAG_ENABLED, ID_FLAG_LACING, ID_INFO, ID_MUXING_APP, ID_PIXEL_HEIGHT, ID_PIXEL_WIDTH,
    ID_SAMPLING_FREQUENCY, ID_SEGMENT, ID_SIMPLE_BLOCK, ID_TIMESTAMP, ID_TIMESTAMP_SCALE,
    ID_TRACKS, ID_TRACK_ENTRY, ID_TRACK_NUMBER, ID_TRACK_TYPE, ID_TRACK_UID, ID_VIDEO,
    ID_WRITING_APP,
};
use crate::error::Error;

/// Write `movie` as a single Segment of SimpleBlocks.
///
/// Cluster timestamps and block-relative timestamps are preserved, so the
/// absolute timestamps stay the same. Payload bytes are copied as-is.
/// Declared element sizes come from bytes already in hand, so the writer does
/// not allocate from an unchecked length.
pub fn remux(movie: &Movie) -> Result<Vec<u8>, Error> {
    if movie.doctype != "webm" && movie.doctype != "matroska" {
        return Err(Error::Invalid);
    }
    if movie.tracks.is_empty() {
        return Err(Error::Invalid);
    }

    let mut segment_body = Vec::new();
    write_info(&mut segment_body, movie)?;
    write_tracks(&mut segment_body, &movie.tracks)?;
    write_clusters(&mut segment_body, &movie.blocks)?;

    let mut out = Vec::new();
    write_ebml_header(&mut out, &movie.doctype)?;
    element(&mut out, ID_SEGMENT, &segment_body)?;
    Ok(out)
}

fn write_ebml_header(out: &mut Vec<u8>, doctype: &str) -> Result<(), Error> {
    let mut body = Vec::new();
    uint_element(&mut body, ID_EBML_VERSION, 1)?;
    uint_element(&mut body, ID_EBML_READ_VERSION, 1)?;
    uint_element(&mut body, ID_EBML_MAX_ID_LENGTH, 4)?;
    uint_element(&mut body, ID_EBML_MAX_SIZE_LENGTH, 8)?;
    string_element(&mut body, ID_DOCTYPE, doctype)?;
    uint_element(&mut body, ID_DOCTYPE_VERSION, 4)?;
    uint_element(&mut body, ID_DOCTYPE_READ_VERSION, 2)?;
    out.extend(master(ID_EBML, &body)?);
    Ok(())
}

fn write_info(out: &mut Vec<u8>, movie: &Movie) -> Result<(), Error> {
    let mut body = Vec::new();
    uint_element(&mut body, ID_TIMESTAMP_SCALE, movie.timestamp_scale)?;
    string_element(&mut body, ID_MUXING_APP, "export-webm")?;
    string_element(&mut body, ID_WRITING_APP, "export-webm")?;
    if let Some(last) = movie.blocks.last() {
        let duration = last.timestamp().max(0) as f64;
        float64_element(&mut body, ID_DURATION, duration)?;
    }
    out.extend(master(ID_INFO, &body)?);
    Ok(())
}

fn write_tracks(out: &mut Vec<u8>, tracks: &[Track]) -> Result<(), Error> {
    let mut body = Vec::new();
    for track in tracks {
        body.extend(write_track(track)?);
    }
    out.extend(master(ID_TRACKS, &body)?);
    Ok(())
}

fn write_track(track: &Track) -> Result<Vec<u8>, Error> {
    if track.number == 0 || track.codec_id.is_empty() {
        return Err(Error::Invalid);
    }
    let mut body = Vec::new();
    uint_element(&mut body, ID_TRACK_NUMBER, track.number)?;
    uint_element(&mut body, ID_TRACK_UID, track.uid)?;
    uint_element(&mut body, ID_TRACK_TYPE, track.track_type)?;
    uint_element(&mut body, ID_FLAG_ENABLED, 1)?;
    uint_element(&mut body, ID_FLAG_DEFAULT, 1)?;
    uint_element(&mut body, ID_FLAG_LACING, 0)?;
    string_element(&mut body, ID_CODEC_ID, &track.codec_id)?;
    if let Some(private) = &track.codec_private {
        element(&mut body, ID_CODEC_PRIVATE, private)?;
    }
    if track.track_type == 1 {
        let width = track.pixel_width.unwrap_or(16);
        let height = track.pixel_height.unwrap_or(16);
        let mut video = Vec::new();
        uint_element(&mut video, ID_PIXEL_WIDTH, width)?;
        uint_element(&mut video, ID_PIXEL_HEIGHT, height)?;
        if let Some(space) = &track.colour_space {
            element(&mut video, ID_COLOUR_SPACE, space)?;
        }
        body.extend(master(ID_VIDEO, &video)?);
    } else if track.track_type == 2 {
        let mut audio = Vec::new();
        float64_element(
            &mut audio,
            ID_SAMPLING_FREQUENCY,
            track.sampling_frequency.unwrap_or(48_000.0),
        )?;
        uint_element(&mut audio, ID_CHANNELS, track.channels.unwrap_or(1))?;
        body.extend(master(ID_AUDIO, &audio)?);
    }
    master(ID_TRACK_ENTRY, &body)
}

fn write_clusters(out: &mut Vec<u8>, blocks: &[MediaBlock]) -> Result<(), Error> {
    let mut index = 0;
    while index < blocks.len() {
        let cluster_timestamp = blocks[index].cluster_timestamp;
        let mut cluster = Vec::new();
        uint_element(&mut cluster, ID_TIMESTAMP, cluster_timestamp)?;
        while index < blocks.len() && blocks[index].cluster_timestamp == cluster_timestamp {
            write_simple_block(&mut cluster, &blocks[index])?;
            index += 1;
        }
        out.extend(master(ID_CLUSTER, &cluster)?);
    }
    Ok(())
}

fn write_simple_block(out: &mut Vec<u8>, block: &MediaBlock) -> Result<(), Error> {
    let mut body = Vec::with_capacity(4 + block.payload.len());
    write_track_vint(&mut body, block.track)?;
    body.extend_from_slice(&block.rel_timestamp.to_be_bytes());
    body.push(block.flags);
    body.extend_from_slice(&block.payload);
    out.extend(master(ID_SIMPLE_BLOCK, &body)?);
    Ok(())
}

fn write_track_vint(out: &mut Vec<u8>, track: u64) -> Result<(), Error> {
    if track == 0 {
        return Err(Error::Invalid);
    }
    crate::ebml::write_size(out, track)
}

/// One video track and three SimpleBlocks, using an uncompressed codec id.
///
/// The id is not VP8, VP9, Opus, or Vorbis, so the stream can be copied.
pub fn sample_movie() -> Movie {
    Movie {
        doctype: "matroska".to_owned(),
        timestamp_scale: 1_000_000,
        tracks: vec![Track {
            number: 1,
            uid: 1,
            track_type: 1,
            codec_id: "V_UNCOMPRESSED".to_owned(),
            codec_private: None,
            pixel_width: Some(2),
            pixel_height: Some(2),
            colour_space: Some(b"I420".to_vec()),
            sampling_frequency: None,
            channels: None,
        }],
        blocks: vec![
            MediaBlock {
                track: 1,
                cluster_timestamp: 0,
                rel_timestamp: 0,
                flags: 0x80,
                payload: b"frm-00".to_vec(),
            },
            MediaBlock {
                track: 1,
                cluster_timestamp: 0,
                rel_timestamp: 33,
                flags: 0x00,
                payload: b"frm-01".to_vec(),
            },
            MediaBlock {
                track: 1,
                cluster_timestamp: 1_000,
                rel_timestamp: 5,
                flags: 0x00,
                payload: vec![0x10, 0x20, 0x30, 0xff, 0x01, 0x02],
            },
        ],
    }
}

pub fn sample_file() -> Vec<u8> {
    remux(&sample_movie()).expect("sample movie is valid")
}

/// One uncompressed track and four SimpleBlocks.
///
/// The payload is 320×240 I420 frames, 460800 bytes in total. The codec id
/// is not VP8, VP9, Opus, or Vorbis.
pub fn large_sample_movie() -> Movie {
    const WIDTH: usize = 320;
    const HEIGHT: usize = 240;
    const FRAMES: usize = 4;
    let frame_bytes = WIDTH * HEIGHT * 3 / 2;
    let mut blocks = Vec::with_capacity(FRAMES);
    for index in 0..FRAMES {
        let mut payload = vec![0u8; frame_bytes];
        for (i, byte) in payload.iter_mut().enumerate() {
            *byte = (i as u8).wrapping_add(index as u8);
        }
        blocks.push(MediaBlock {
            track: 1,
            cluster_timestamp: 0,
            rel_timestamp: (index as i16) * 40,
            flags: if index == 0 { 0x80 } else { 0x00 },
            payload,
        });
    }
    Movie {
        doctype: "matroska".to_owned(),
        timestamp_scale: 1_000_000,
        tracks: vec![Track {
            number: 1,
            uid: 1,
            track_type: 1,
            codec_id: "V_UNCOMPRESSED".to_owned(),
            codec_private: None,
            pixel_width: Some(WIDTH as u64),
            pixel_height: Some(HEIGHT as u64),
            colour_space: Some(b"I420".to_vec()),
            sampling_frequency: None,
            channels: None,
        }],
        blocks,
    }
}

pub fn large_sample_file() -> Vec<u8> {
    remux(&large_sample_movie()).expect("large sample movie is valid")
}
