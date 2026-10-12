//! Read moof/mdat sample bytes. Codec bitstreams are not parsed.

use crate::atom::{atoms, Cur};
use crate::error::Error;
use crate::sniff::is_forbidden;

const TFHD_BASE_DATA_OFFSET: u32 = 0x000001;
const TFHD_SAMPLE_DESC: u32 = 0x000002;
const TFHD_DEFAULT_DURATION: u32 = 0x000008;
const TFHD_DEFAULT_SIZE: u32 = 0x000010;
const TFHD_DEFAULT_FLAGS: u32 = 0x000020;
const TFHD_DURATION_IS_EMPTY: u32 = 0x010000;
const TFHD_DEFAULT_BASE_IS_MOOF: u32 = 0x020000;

const TRUN_DATA_OFFSET: u32 = 0x000001;
const TRUN_FIRST_FLAGS: u32 = 0x000004;
const TRUN_DURATION: u32 = 0x000100;
const TRUN_SIZE: u32 = 0x000200;
const TRUN_FLAGS: u32 = 0x000400;
const TRUN_CTS: u32 = 0x000800;

/// One copied sample. `payload` is the mdat byte range named by the trun.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sample {
    pub duration: u32,
    pub flags: u32,
    pub cts: i64,
    pub payload: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub track_id: u32,
    pub base_media_decode_time: u64,
    pub samples: Vec<Sample>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fragment {
    pub sequence: u32,
    pub runs: Vec<Run>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Track {
    pub id: u32,
    pub handler: [u8; 4],
    pub timescale: u32,
    /// Width in pixels for a video track. Zero for audio.
    pub width: u32,
    /// Height in pixels for a video track. Zero for audio.
    pub height: u32,
    /// Entire sample-entry box, including its size and fourcc.
    pub sample_entry: Vec<u8>,
    pub default_duration: u32,
    pub default_size: u32,
    pub default_flags: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Movie {
    pub brand: [u8; 4],
    pub timescale: u32,
    pub tracks: Vec<Track>,
    pub fragments: Vec<Fragment>,
}

/// Parse init and fragment boxes, then copy each sample's bytes out of the file.
pub fn demux(input: &[u8]) -> Result<Movie, Error> {
    let top = atoms(input, 0, input.len())?;
    let mut brand = *b"isom";
    let mut saw_ftyp = false;
    let mut moov = None;
    let mut moofs = Vec::new();
    for atom in &top {
        match &atom.typ {
            b"ftyp" => {
                let payload = &input[atom.payload.clone()];
                if payload.len() < 8 {
                    return Err(Error::Invalid);
                }
                brand.copy_from_slice(&payload[..4]);
                saw_ftyp = true;
            }
            b"moov" => moov = Some(atom),
            b"moof" => moofs.push(atom),
            _ => {}
        }
    }
    if !saw_ftyp {
        return Err(Error::Invalid);
    }
    let moov = moov.ok_or(Error::Invalid)?;
    if moofs.is_empty() {
        return Err(Error::Invalid);
    }
    let mut movie = parse_moov(input, moov.payload.clone())?;
    movie.brand = brand;
    for moof in moofs {
        movie.fragments.push(parse_moof(
            input,
            moof.start,
            moof.payload.end,
            &movie.tracks,
        )?);
    }
    let samples: usize = movie
        .fragments
        .iter()
        .map(|fragment| {
            fragment
                .runs
                .iter()
                .map(|run| run.samples.len())
                .sum::<usize>()
        })
        .sum();
    if samples == 0 {
        return Err(Error::Invalid);
    }
    Ok(movie)
}

fn parse_moov(input: &[u8], range: std::ops::Range<usize>) -> Result<Movie, Error> {
    let children = atoms(input, range.start, range.end)?;
    let mut timescale = 1000u32;
    let mut tracks = Vec::new();
    let mut trex = Vec::new();
    for child in &children {
        match &child.typ {
            b"mvhd" => {
                timescale = parse_mvhd(&input[child.payload.clone()])?;
            }
            b"trak" => tracks.push(parse_trak(input, child.payload.clone())?),
            b"mvex" => {
                let boxes = atoms(input, child.payload.start, child.payload.end)?;
                for box_ in &boxes {
                    if box_.typ == *b"trex" {
                        trex.push(parse_trex(&input[box_.payload.clone()])?);
                    }
                }
            }
            _ => {}
        }
    }
    if tracks.is_empty() {
        return Err(Error::Invalid);
    }
    for (track_id, duration, size, flags) in trex {
        let track = tracks
            .iter_mut()
            .find(|track| track.id == track_id)
            .ok_or(Error::Invalid)?;
        track.default_duration = duration;
        track.default_size = size;
        track.default_flags = flags;
    }
    Ok(Movie {
        brand: *b"isom",
        timescale,
        tracks,
        fragments: Vec::new(),
    })
}

fn parse_mvhd(payload: &[u8]) -> Result<u32, Error> {
    let mut cur = Cur::new(payload);
    let version = cur.u32()? >> 24;
    if version == 0 {
        cur.skip(8)?;
        cur.u32()
    } else if version == 1 {
        cur.skip(16)?;
        cur.u32()
    } else {
        Err(Error::Invalid)
    }
}

fn parse_trak(input: &[u8], range: std::ops::Range<usize>) -> Result<Track, Error> {
    let children = atoms(input, range.start, range.end)?;
    let mut id = None;
    let mut handler = None;
    let mut timescale = None;
    let mut entry = None;
    let mut width = 0u32;
    let mut height = 0u32;
    for child in &children {
        match &child.typ {
            b"tkhd" => id = Some(parse_tkhd(&input[child.payload.clone()])?),
            b"mdia" => {
                let mdia = atoms(input, child.payload.start, child.payload.end)?;
                for box_ in &mdia {
                    match &box_.typ {
                        b"mdhd" => {
                            timescale = Some(parse_mdhd(&input[box_.payload.clone()])?);
                        }
                        b"hdlr" => {
                            handler = Some(parse_hdlr(&input[box_.payload.clone()])?);
                        }
                        b"minf" => {
                            let found = find_sample_entry(input, box_.payload.clone())?;
                            width = found.0;
                            height = found.1;
                            entry = Some(found.2);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    let sample_entry = entry.ok_or(Error::Invalid)?;
    let handler = handler.ok_or(Error::Invalid)?;
    if handler != *b"vide" {
        width = 0;
        height = 0;
    }
    Ok(Track {
        id: id.ok_or(Error::Invalid)?,
        handler,
        timescale: timescale
            .filter(|value| *value != 0)
            .ok_or(Error::Invalid)?,
        width,
        height,
        sample_entry,
        default_duration: 0,
        default_size: 0,
        default_flags: 0,
    })
}

fn parse_tkhd(payload: &[u8]) -> Result<u32, Error> {
    let mut cur = Cur::new(payload);
    let version = cur.u32()? >> 24;
    if version == 0 {
        cur.skip(8)?;
    } else if version == 1 {
        cur.skip(16)?;
    } else {
        return Err(Error::Invalid);
    }
    let id = cur.u32()?;
    if id == 0 {
        return Err(Error::Invalid);
    }
    Ok(id)
}

fn parse_mdhd(payload: &[u8]) -> Result<u32, Error> {
    let mut cur = Cur::new(payload);
    let version = cur.u32()? >> 24;
    if version == 0 {
        cur.skip(8)?;
        cur.u32()
    } else if version == 1 {
        cur.skip(16)?;
        cur.u32()
    } else {
        Err(Error::Invalid)
    }
}

fn parse_hdlr(payload: &[u8]) -> Result<[u8; 4], Error> {
    let mut cur = Cur::new(payload);
    cur.u32()?;
    cur.skip(4)?;
    let mut handler = [0u8; 4];
    handler.copy_from_slice(cur.bytes(4)?);
    Ok(handler)
}

fn find_sample_entry(
    input: &[u8],
    minf: std::ops::Range<usize>,
) -> Result<(u32, u32, Vec<u8>), Error> {
    let minf_boxes = atoms(input, minf.start, minf.end)?;
    for box_ in &minf_boxes {
        if box_.typ != *b"stbl" {
            continue;
        }
        let stbl = atoms(input, box_.payload.start, box_.payload.end)?;
        for table in &stbl {
            if table.typ != *b"stsd" {
                continue;
            }
            return first_entry(input, table.payload.clone());
        }
    }
    Err(Error::Invalid)
}

fn first_entry(input: &[u8], stsd: std::ops::Range<usize>) -> Result<(u32, u32, Vec<u8>), Error> {
    if stsd.end - stsd.start < 8 {
        return Err(Error::Invalid);
    }
    let entries = atoms(input, stsd.start + 8, stsd.end)?;
    let entry = entries.first().ok_or(Error::Invalid)?;
    if is_forbidden(entry.typ) {
        return Err(Error::Unsupported);
    }
    let bytes = input[entry.start..entry.payload.end].to_vec();
    let payload = &input[entry.payload.clone()];
    let (width, height) = visual_size(payload).unwrap_or((0, 0));
    Ok((width, height, bytes))
}

/// Visual sample entries are 78 bytes before any codec boxes. Audio entries are
/// shorter, so a failed read means "not visual" and the caller keeps 0×0.
fn visual_size(payload: &[u8]) -> Option<(u32, u32)> {
    if payload.len() < 78 {
        return None;
    }
    let width = u16::from_be_bytes([payload[24], payload[25]]);
    let height = u16::from_be_bytes([payload[26], payload[27]]);
    if width == 0 || height == 0 {
        return None;
    }
    Some((u32::from(width), u32::from(height)))
}

fn parse_trex(payload: &[u8]) -> Result<(u32, u32, u32, u32), Error> {
    let mut cur = Cur::new(payload);
    cur.u32()?;
    let track_id = cur.u32()?;
    let _index = cur.u32()?;
    let duration = cur.u32()?;
    let size = cur.u32()?;
    let flags = cur.u32()?;
    Ok((track_id, duration, size, flags))
}

fn parse_moof(
    input: &[u8],
    moof_start: usize,
    moof_end: usize,
    tracks: &[Track],
) -> Result<Fragment, Error> {
    let children = atoms(input, moof_start + 8, moof_end)?;
    let mut sequence = 1u32;
    let mut runs = Vec::new();
    for child in &children {
        if child.typ == *b"mfhd" {
            let mut cur = Cur::new(&input[child.payload.clone()]);
            cur.u32()?;
            sequence = cur.u32()?;
        } else if child.typ == *b"traf" {
            runs.push(parse_traf(
                input,
                moof_start,
                moof_end,
                child.payload.clone(),
                tracks,
            )?);
        }
    }
    if runs.is_empty() {
        return Err(Error::Invalid);
    }
    Ok(Fragment { sequence, runs })
}

fn parse_traf(
    input: &[u8],
    moof_start: usize,
    moof_end: usize,
    range: std::ops::Range<usize>,
    tracks: &[Track],
) -> Result<Run, Error> {
    let children = atoms(input, range.start, range.end)?;
    let mut track_id = 0u32;
    let mut saw_tfhd = false;
    let mut base_is_moof = false;
    let mut explicit_base: Option<u64> = None;
    let mut default_duration = 0u32;
    let mut default_size = 0u32;
    let mut default_flags = 0u32;
    let mut base_time = 0u64;
    let mut truns = Vec::new();
    for child in &children {
        let payload = &input[child.payload.clone()];
        match &child.typ {
            b"tfhd" => {
                let header = parse_tfhd(payload)?;
                track_id = header.track_id;
                saw_tfhd = true;
                base_is_moof = header.base_is_moof;
                explicit_base = header.explicit_base;
                let track = tracks
                    .iter()
                    .find(|track| track.id == track_id)
                    .ok_or(Error::Invalid)?;
                default_duration = header.duration.unwrap_or(track.default_duration);
                default_size = header.size.unwrap_or(track.default_size);
                default_flags = header.flags.unwrap_or(track.default_flags);
                if header.duration_is_empty {
                    default_duration = 0;
                }
            }
            b"tfdt" => base_time = parse_tfdt(payload)?,
            b"trun" => truns.push(payload),
            _ => {}
        }
    }
    if !saw_tfhd || track_id == 0 || truns.is_empty() {
        return Err(Error::Invalid);
    }
    let base = if base_is_moof {
        moof_start as u64
    } else if let Some(explicit) = explicit_base {
        explicit
    } else {
        moof_end as u64
    };
    let mut samples = Vec::new();
    let mut cursor = base;
    for trun in truns {
        let parsed = parse_trun(
            trun,
            default_duration,
            default_size,
            default_flags,
            input.len(),
        )?;
        let mut pos = if let Some(delta) = parsed.data_offset {
            add_offset(base, delta)?
        } else {
            cursor
        };
        for meta in parsed.samples {
            let start = usize::try_from(pos).map_err(|_| Error::Invalid)?;
            let end = start
                .checked_add(meta.size as usize)
                .ok_or(Error::Invalid)?;
            if end > input.len() {
                return Err(Error::Truncated);
            }
            samples.push(Sample {
                duration: meta.duration,
                flags: meta.flags,
                cts: meta.cts,
                payload: input[start..end].to_vec(),
            });
            pos = pos.saturating_add(u64::from(meta.size));
        }
        cursor = pos;
    }
    if samples.is_empty() {
        return Err(Error::Invalid);
    }
    Ok(Run {
        track_id,
        base_media_decode_time: base_time,
        samples,
    })
}

struct Tfhd {
    track_id: u32,
    base_is_moof: bool,
    explicit_base: Option<u64>,
    duration: Option<u32>,
    size: Option<u32>,
    flags: Option<u32>,
    duration_is_empty: bool,
}

fn parse_tfhd(payload: &[u8]) -> Result<Tfhd, Error> {
    let mut cur = Cur::new(payload);
    let flags = cur.u32()? & 0x00ff_ffff;
    let track_id = cur.u32()?;
    let mut explicit_base = None;
    if flags & TFHD_BASE_DATA_OFFSET != 0 {
        explicit_base = Some(cur.u64()?);
    }
    if flags & TFHD_SAMPLE_DESC != 0 {
        let index = cur.u32()?;
        if index != 1 {
            return Err(Error::Invalid);
        }
    }
    let duration = if flags & TFHD_DEFAULT_DURATION != 0 {
        Some(cur.u32()?)
    } else {
        None
    };
    let size = if flags & TFHD_DEFAULT_SIZE != 0 {
        Some(cur.u32()?)
    } else {
        None
    };
    let sample_flags = if flags & TFHD_DEFAULT_FLAGS != 0 {
        Some(cur.u32()?)
    } else {
        None
    };
    Ok(Tfhd {
        track_id,
        base_is_moof: flags & TFHD_DEFAULT_BASE_IS_MOOF != 0,
        explicit_base,
        duration,
        size,
        flags: sample_flags,
        duration_is_empty: flags & TFHD_DURATION_IS_EMPTY != 0,
    })
}

fn parse_tfdt(payload: &[u8]) -> Result<u64, Error> {
    let mut cur = Cur::new(payload);
    let version = cur.u32()? >> 24;
    if version == 0 {
        cur.u32().map(u64::from)
    } else if version == 1 {
        cur.u64()
    } else {
        Err(Error::Invalid)
    }
}

struct Trun {
    data_offset: Option<i32>,
    samples: Vec<SampleMeta>,
}

struct SampleMeta {
    duration: u32,
    size: u32,
    flags: u32,
    cts: i64,
}

fn parse_trun(
    payload: &[u8],
    default_duration: u32,
    default_size: u32,
    default_flags: u32,
    file_len: usize,
) -> Result<Trun, Error> {
    let mut cur = Cur::new(payload);
    let version_flags = cur.u32()?;
    let version = version_flags >> 24;
    let flags = version_flags & 0x00ff_ffff;
    let count_u = cur.u32()?;
    let count = count_u as usize;
    if count > file_len {
        return Err(Error::Invalid);
    }
    let data_offset = if flags & TRUN_DATA_OFFSET != 0 {
        Some(cur.i32()?)
    } else {
        None
    };
    let first_flags = if flags & TRUN_FIRST_FLAGS != 0 {
        Some(cur.u32()?)
    } else {
        None
    };
    let has_duration = flags & TRUN_DURATION != 0;
    let has_size = flags & TRUN_SIZE != 0;
    let has_flags = flags & TRUN_FLAGS != 0;
    let has_cts = flags & TRUN_CTS != 0;
    let mut per = 0usize;
    if has_duration {
        per += 4;
    }
    if has_size {
        per += 4;
    }
    if has_flags {
        per += 4;
    }
    if has_cts {
        per += 4;
    }
    let need = count.checked_mul(per).ok_or(Error::Invalid)?;
    if need > cur.remaining() {
        return Err(Error::Truncated);
    }
    let mut samples = Vec::with_capacity(count);
    let mut total = 0u64;
    for index in 0..count {
        let duration = if has_duration {
            cur.u32()?
        } else {
            default_duration
        };
        let size = if has_size { cur.u32()? } else { default_size };
        let sample_flags = if has_flags {
            cur.u32()?
        } else if index == 0 {
            first_flags.unwrap_or(default_flags)
        } else {
            default_flags
        };
        let cts = if has_cts {
            let raw = cur.u32()?;
            if version == 0 {
                i64::from(raw)
            } else {
                i64::from(raw as i32)
            }
        } else {
            0
        };
        total = total.checked_add(u64::from(size)).ok_or(Error::Invalid)?;
        if total > file_len as u64 {
            return Err(Error::Truncated);
        }
        samples.push(SampleMeta {
            duration,
            size,
            flags: sample_flags,
            cts,
        });
    }
    Ok(Trun {
        data_offset,
        samples,
    })
}

fn add_offset(base: u64, delta: i32) -> Result<u64, Error> {
    let base_i = i64::try_from(base).map_err(|_| Error::Invalid)?;
    let sum = base_i.checked_add(i64::from(delta)).ok_or(Error::Invalid)?;
    u64::try_from(sum).map_err(|_| Error::Invalid)
}
