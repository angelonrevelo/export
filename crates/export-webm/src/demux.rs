//! EBML sniff and demux. Block payloads are copied, never decoded.

use crate::ebml::{
    unsupported_codec, Cursor, ID_AUDIO, ID_BLOCK, ID_BLOCK_DURATION, ID_BLOCK_GROUP, ID_CHANNELS,
    ID_CLUSTER, ID_CODEC_ID, ID_CODEC_PRIVATE, ID_COLOUR_SPACE, ID_CRC32, ID_DOCTYPE,
    ID_DOCTYPE_READ_VERSION, ID_DOCTYPE_VERSION, ID_DURATION, ID_EBML, ID_EBML_MAX_ID_LENGTH,
    ID_EBML_MAX_SIZE_LENGTH, ID_EBML_READ_VERSION, ID_EBML_VERSION, ID_INFO, ID_MUXING_APP,
    ID_PIXEL_HEIGHT, ID_PIXEL_WIDTH, ID_REFERENCE_BLOCK, ID_SAMPLING_FREQUENCY, ID_SEGMENT,
    ID_SIMPLE_BLOCK, ID_TIMESTAMP, ID_TIMESTAMP_SCALE, ID_TRACKS, ID_TRACK_ENTRY, ID_TRACK_NUMBER,
    ID_TRACK_TYPE, ID_TRACK_UID, ID_VIDEO, ID_VOID, ID_WRITING_APP,
};
use crate::error::Error;

/// One copied SimpleBlock or Block. `payload` is the media bytes after the
/// track, timestamp, and flags header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaBlock {
    pub track: u64,
    pub cluster_timestamp: u64,
    pub rel_timestamp: i16,
    pub flags: u8,
    pub payload: Vec<u8>,
}

impl MediaBlock {
    /// Timestamp in `timestamp_scale` units.
    pub fn timestamp(&self) -> i64 {
        (self.cluster_timestamp as i64).saturating_add(i64::from(self.rel_timestamp))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub number: u64,
    pub uid: u64,
    pub track_type: u64,
    pub codec_id: String,
    pub codec_private: Option<Vec<u8>>,
    pub pixel_width: Option<u64>,
    pub pixel_height: Option<u64>,
    pub colour_space: Option<Vec<u8>>,
    pub sampling_frequency: Option<f64>,
    pub channels: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Movie {
    pub doctype: String,
    pub timestamp_scale: u64,
    pub tracks: Vec<Track>,
    pub blocks: Vec<MediaBlock>,
}

/// Sniff the EBML header and codec ids.
///
/// Vorbis returns [`Error::Unsupported`]. It is not decoded.
/// VP8, VP9, and Opus payloads are copied as block bytes and are not decoded.
pub fn sniff(input: &[u8]) -> Result<(), Error> {
    let movie = parse(input, true)?;
    let _ = movie;
    Ok(())
}

/// Demux container structure. Codec payloads are not decoded.
pub fn demux(input: &[u8]) -> Result<Movie, Error> {
    parse(input, false)
}

fn parse(input: &[u8], stop_after_tracks: bool) -> Result<Movie, Error> {
    if input.is_empty() {
        return Err(Error::Truncated);
    }
    let mut top = Cursor::new(input);
    let header = top.next_element()?.ok_or(Error::Truncated)?;
    if header.id != ID_EBML {
        return Err(Error::Invalid);
    }
    let doctype = parse_ebml_header(header.body)?;
    let mut movie = Movie {
        doctype,
        timestamp_scale: 1_000_000,
        tracks: Vec::new(),
        blocks: Vec::new(),
    };

    let mut saw_segment = false;
    while let Some(element) = top.next_element()? {
        if element.id == ID_VOID || element.id == ID_CRC32 {
            continue;
        }
        if element.id != ID_SEGMENT {
            return Err(Error::Invalid);
        }
        saw_segment = true;
        parse_segment(element.body, &mut movie, stop_after_tracks)?;
        if stop_after_tracks && !movie.tracks.is_empty() {
            break;
        }
    }
    if !saw_segment {
        return Err(Error::Truncated);
    }
    if movie.tracks.is_empty() {
        return Err(Error::Invalid);
    }
    Ok(movie)
}

fn parse_ebml_header(body: &[u8]) -> Result<String, Error> {
    let mut cursor = Cursor::slice(body, body.len());
    let mut doctype = None;
    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_EBML_VERSION
            | ID_EBML_READ_VERSION
            | ID_EBML_MAX_ID_LENGTH
            | ID_EBML_MAX_SIZE_LENGTH
            | ID_DOCTYPE_VERSION
            | ID_DOCTYPE_READ_VERSION => {
                let _ = read_uint_body(element.body)?;
            }
            ID_DOCTYPE => {
                doctype = Some(crate::ebml::read_string(element.body)?);
            }
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }
    match doctype.as_deref() {
        Some("webm") | Some("matroska") => Ok(doctype.unwrap()),
        Some(_) => Err(Error::Invalid),
        None => Err(Error::Invalid),
    }
}

fn parse_segment(body: &[u8], movie: &mut Movie, stop_after_tracks: bool) -> Result<(), Error> {
    let mut cursor = Cursor::slice(body, body.len());
    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_INFO => parse_info(element.body, movie)?,
            ID_TRACKS => parse_tracks(element.body, movie)?,
            ID_CLUSTER if !stop_after_tracks => parse_cluster(element.body, movie)?,
            ID_CLUSTER => {}
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }
    Ok(())
}

fn parse_info(body: &[u8], movie: &mut Movie) -> Result<(), Error> {
    let mut cursor = Cursor::slice(body, body.len());
    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_TIMESTAMP_SCALE => {
                movie.timestamp_scale = read_uint_body(element.body)?;
            }
            ID_DURATION => {
                let _ = crate::ebml::read_float(element.body)?;
            }
            ID_MUXING_APP | ID_WRITING_APP => {
                let _ = crate::ebml::read_string(element.body)?;
            }
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }
    Ok(())
}

fn parse_tracks(body: &[u8], movie: &mut Movie) -> Result<(), Error> {
    let mut cursor = Cursor::slice(body, body.len());
    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_TRACK_ENTRY => movie.tracks.push(parse_track_entry(element.body)?),
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }
    Ok(())
}

fn parse_track_entry(body: &[u8]) -> Result<Track, Error> {
    let mut cursor = Cursor::slice(body, body.len());
    let mut number = None;
    let mut uid = None;
    let mut track_type = None;
    let mut codec_id = None;
    let mut codec_private = None;
    let mut pixel_width = None;
    let mut pixel_height = None;
    let mut colour_space = None;
    let mut sampling_frequency = None;
    let mut channels = None;

    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_TRACK_NUMBER => number = Some(read_uint_body(element.body)?),
            ID_TRACK_UID => uid = Some(read_uint_body(element.body)?),
            ID_TRACK_TYPE => track_type = Some(read_uint_body(element.body)?),
            ID_CODEC_ID => {
                let text = crate::ebml::read_string(element.body)?;
                if unsupported_codec(&text) {
                    return Err(Error::Unsupported);
                }
                codec_id = Some(text);
            }
            ID_CODEC_PRIVATE => codec_private = Some(element.body.to_vec()),
            ID_VIDEO => {
                let video = parse_video(element.body)?;
                pixel_width = video.width;
                pixel_height = video.height;
                colour_space = video.colour_space;
            }
            ID_AUDIO => {
                let (freq, ch) = parse_audio(element.body)?;
                sampling_frequency = freq;
                channels = ch;
            }
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }

    Ok(Track {
        number: number.ok_or(Error::Invalid)?,
        uid: uid.unwrap_or(1),
        track_type: track_type.ok_or(Error::Invalid)?,
        codec_id: codec_id.ok_or(Error::Invalid)?,
        codec_private,
        pixel_width,
        pixel_height,
        colour_space,
        sampling_frequency,
        channels,
    })
}

struct VideoInfo {
    width: Option<u64>,
    height: Option<u64>,
    colour_space: Option<Vec<u8>>,
}

fn parse_video(body: &[u8]) -> Result<VideoInfo, Error> {
    let mut cursor = Cursor::slice(body, body.len());
    let mut info = VideoInfo {
        width: None,
        height: None,
        colour_space: None,
    };
    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_PIXEL_WIDTH => info.width = Some(read_uint_body(element.body)?),
            ID_PIXEL_HEIGHT => info.height = Some(read_uint_body(element.body)?),
            ID_COLOUR_SPACE => info.colour_space = Some(element.body.to_vec()),
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }
    Ok(info)
}

fn parse_audio(body: &[u8]) -> Result<(Option<f64>, Option<u64>), Error> {
    let mut cursor = Cursor::slice(body, body.len());
    let mut freq = None;
    let mut channels = None;
    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_SAMPLING_FREQUENCY => freq = Some(crate::ebml::read_float(element.body)?),
            ID_CHANNELS => channels = Some(read_uint_body(element.body)?),
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }
    Ok((freq, channels))
}

fn parse_cluster(body: &[u8], movie: &mut Movie) -> Result<(), Error> {
    let mut cursor = Cursor::slice(body, body.len());
    let mut cluster_timestamp = 0u64;
    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_TIMESTAMP => cluster_timestamp = read_uint_body(element.body)?,
            ID_SIMPLE_BLOCK => {
                movie
                    .blocks
                    .push(parse_block(element.body, cluster_timestamp)?);
            }
            ID_BLOCK_GROUP => parse_block_group(element.body, cluster_timestamp, movie)?,
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }
    Ok(())
}

fn parse_block_group(body: &[u8], cluster_timestamp: u64, movie: &mut Movie) -> Result<(), Error> {
    let mut cursor = Cursor::slice(body, body.len());
    while let Some(element) = cursor.next_element()? {
        match element.id {
            ID_BLOCK => movie
                .blocks
                .push(parse_block(element.body, cluster_timestamp)?),
            ID_BLOCK_DURATION | ID_REFERENCE_BLOCK => {}
            ID_VOID | ID_CRC32 => {}
            _ => {}
        }
    }
    Ok(())
}

fn parse_block(body: &[u8], cluster_timestamp: u64) -> Result<MediaBlock, Error> {
    if body.is_empty() {
        return Err(Error::Truncated);
    }
    let mut pos = 0usize;
    let track = read_track_vint(body, &mut pos)?;
    if pos + 3 > body.len() {
        return Err(Error::Truncated);
    }
    let rel_timestamp = i16::from_be_bytes([body[pos], body[pos + 1]]);
    pos += 2;
    let flags = body[pos];
    pos += 1;
    Ok(MediaBlock {
        track,
        cluster_timestamp,
        rel_timestamp,
        flags,
        payload: body[pos..].to_vec(),
    })
}

fn read_track_vint(data: &[u8], pos: &mut usize) -> Result<u64, Error> {
    if *pos >= data.len() {
        return Err(Error::Truncated);
    }
    let first = data[*pos];
    if first == 0 {
        return Err(Error::Invalid);
    }
    let width = (first.leading_zeros() as usize) + 1;
    if width > 8 || *pos + width > data.len() {
        return Err(Error::Truncated);
    }
    let mut raw = 0u64;
    for i in 0..width {
        raw = (raw << 8) | u64::from(data[*pos + i]);
    }
    *pos += width;
    let marker = 1u64 << (width * 7);
    Ok(raw & (marker - 1))
}

fn read_uint_body(body: &[u8]) -> Result<u64, Error> {
    crate::ebml::read_uint(body)
}
