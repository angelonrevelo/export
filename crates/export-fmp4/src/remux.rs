//! Rebuild a fragmented MP4 from copied samples. Bytes are copied.

use crate::atom::{full_boxed, push_box, push_full};
use crate::demux::{Fragment, Movie, Run, Sample, Track};
use crate::error::Error;

const SAMPLE_RATE: u32 = 8_000;
const SAMPLE_BYTES: usize = 1 << 20;
const SAMPLES_PER_FRAGMENT: usize = 4;
const FRAGMENTS: usize = 2;
/// `sample_depends_on = 2` (does not depend on other samples).
const SYNC_FLAGS: u32 = 0x0200_0000;

const TFHD_DEFAULT_BASE_IS_MOOF: u32 = 0x020000;
const TRUN_DATA_OFFSET: u32 = 0x000001;
const TRUN_DURATION: u32 = 0x000100;
const TRUN_SIZE: u32 = 0x000200;
const TRUN_FLAGS: u32 = 0x000400;

/// Write `movie` as `ftyp` + `moov` + one `moof`/`mdat` pair per fragment.
///
/// Sample payloads are copied. Declared box sizes come from bytes already in
/// hand. A `trun` data offset that does not fit in `i32` is refused.
pub fn remux(movie: &Movie) -> Result<Vec<u8>, Error> {
    if movie.tracks.is_empty() || movie.fragments.is_empty() {
        return Err(Error::Invalid);
    }
    let mut out = Vec::new();
    push_box(&mut out, *b"ftyp", &ftyp_payload(movie.brand)?)?;
    write_moov(&mut out, movie)?;
    for fragment in &movie.fragments {
        write_fragment(&mut out, fragment)?;
    }
    Ok(out)
}

fn ftyp_payload(brand: [u8; 4]) -> Result<Vec<u8>, Error> {
    let major = if brand == *b"qt  " { *b"iso5" } else { brand };
    let mut payload = Vec::with_capacity(20);
    payload.extend_from_slice(&major);
    payload.extend_from_slice(&0x200u32.to_be_bytes());
    payload.extend_from_slice(b"iso5iso6mp41");
    if payload.len() < 8 {
        return Err(Error::Invalid);
    }
    Ok(payload)
}

fn write_moov(out: &mut Vec<u8>, movie: &Movie) -> Result<(), Error> {
    let mut body = Vec::new();
    let next_track = movie
        .tracks
        .iter()
        .map(|track| track.id)
        .max()
        .ok_or(Error::Invalid)?
        .saturating_add(1);
    let timescale = if movie.timescale == 0 {
        movie.tracks[0].timescale
    } else {
        movie.timescale
    };
    write_mvhd(&mut body, timescale, next_track)?;
    for track in &movie.tracks {
        write_trak(&mut body, track)?;
    }
    write_mvex(&mut body, movie)?;
    push_box(out, *b"moov", &body)
}

fn write_mvhd(out: &mut Vec<u8>, timescale: u32, next_track: u32) -> Result<(), Error> {
    let mut body = Vec::with_capacity(96);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&timescale.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    body.extend_from_slice(&0x0100u16.to_be_bytes());
    body.extend_from_slice(&[0u8; 10]);
    write_matrix(&mut body);
    body.extend_from_slice(&[0u8; 24]);
    body.extend_from_slice(&next_track.to_be_bytes());
    push_full(out, *b"mvhd", 0, 0, &body)
}

fn write_trak(out: &mut Vec<u8>, track: &Track) -> Result<(), Error> {
    let mut body = Vec::new();
    write_tkhd(&mut body, track)?;
    write_mdia(&mut body, track)?;
    push_box(out, *b"trak", &body)
}

fn write_tkhd(out: &mut Vec<u8>, track: &Track) -> Result<(), Error> {
    let audio = track.handler == *b"soun";
    let mut body = Vec::with_capacity(80);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&track.id.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&[0u8; 8]);
    body.extend_from_slice(&0u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    let volume: u16 = if audio { 0x0100 } else { 0 };
    body.extend_from_slice(&volume.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    write_matrix(&mut body);
    let width = track.width.saturating_mul(0x10000);
    let height = track.height.saturating_mul(0x10000);
    body.extend_from_slice(&width.to_be_bytes());
    body.extend_from_slice(&height.to_be_bytes());
    push_full(out, *b"tkhd", 0, 3, &body)
}

fn write_mdia(out: &mut Vec<u8>, track: &Track) -> Result<(), Error> {
    let mut body = Vec::new();
    write_mdhd(&mut body, track.timescale)?;
    write_hdlr(&mut body, track.handler)?;
    write_minf(&mut body, track)?;
    push_box(out, *b"mdia", &body)
}

fn write_mdhd(out: &mut Vec<u8>, timescale: u32) -> Result<(), Error> {
    let mut body = Vec::with_capacity(20);
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&timescale.to_be_bytes());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&0x55c4u16.to_be_bytes());
    body.extend_from_slice(&0u16.to_be_bytes());
    push_full(out, *b"mdhd", 0, 0, &body)
}

fn write_hdlr(out: &mut Vec<u8>, handler: [u8; 4]) -> Result<(), Error> {
    let name: &[u8] = if handler == *b"vide" {
        b"VideoHandler\0"
    } else {
        b"SoundHandler\0"
    };
    let mut body = Vec::with_capacity(20 + name.len());
    body.extend_from_slice(&0u32.to_be_bytes());
    body.extend_from_slice(&handler);
    body.extend_from_slice(&[0u8; 12]);
    body.extend_from_slice(name);
    push_full(out, *b"hdlr", 0, 0, &body)
}

fn write_minf(out: &mut Vec<u8>, track: &Track) -> Result<(), Error> {
    let mut body = Vec::new();
    if track.handler == *b"vide" {
        push_full(&mut body, *b"vmhd", 0, 1, &[0u8; 8])?;
    } else {
        push_full(&mut body, *b"smhd", 0, 0, &[0u8; 4])?;
    }
    write_dinf(&mut body)?;
    write_stbl(&mut body, track)?;
    push_box(out, *b"minf", &body)
}

fn write_dinf(out: &mut Vec<u8>) -> Result<(), Error> {
    let url = full_boxed(*b"url ", 0, 1, &[])?;
    let mut dref_body = Vec::new();
    dref_body.extend_from_slice(&1u32.to_be_bytes());
    dref_body.extend_from_slice(&url);
    let dref = full_boxed(*b"dref", 0, 0, &dref_body)?;
    push_box(out, *b"dinf", &dref)
}

fn write_stbl(out: &mut Vec<u8>, track: &Track) -> Result<(), Error> {
    let mut body = Vec::new();
    let mut stsd = Vec::new();
    stsd.extend_from_slice(&1u32.to_be_bytes());
    stsd.extend_from_slice(&track.sample_entry);
    push_full(&mut body, *b"stsd", 0, 0, &stsd)?;
    push_full(&mut body, *b"stts", 0, 0, &0u32.to_be_bytes())?;
    push_full(&mut body, *b"stsc", 0, 0, &0u32.to_be_bytes())?;
    let mut stsz = Vec::with_capacity(8);
    stsz.extend_from_slice(&0u32.to_be_bytes());
    stsz.extend_from_slice(&0u32.to_be_bytes());
    push_full(&mut body, *b"stsz", 0, 0, &stsz)?;
    push_full(&mut body, *b"stco", 0, 0, &0u32.to_be_bytes())?;
    push_box(out, *b"stbl", &body)
}

fn write_mvex(out: &mut Vec<u8>, movie: &Movie) -> Result<(), Error> {
    let mut body = Vec::new();
    for track in &movie.tracks {
        let mut trex = Vec::with_capacity(20);
        trex.extend_from_slice(&track.id.to_be_bytes());
        trex.extend_from_slice(&1u32.to_be_bytes());
        trex.extend_from_slice(&track.default_duration.to_be_bytes());
        trex.extend_from_slice(&track.default_size.to_be_bytes());
        trex.extend_from_slice(&track.default_flags.to_be_bytes());
        push_full(&mut body, *b"trex", 0, 0, &trex)?;
    }
    push_box(out, *b"mvex", &body)
}

fn write_fragment(out: &mut Vec<u8>, fragment: &Fragment) -> Result<(), Error> {
    if fragment.runs.is_empty() {
        return Err(Error::Invalid);
    }
    let mut sample_bytes = 0usize;
    for run in &fragment.runs {
        for sample in &run.samples {
            sample_bytes = sample_bytes
                .checked_add(sample.payload.len())
                .ok_or(Error::Invalid)?;
        }
    }
    let moof = build_moof(fragment)?;
    out.extend_from_slice(&moof);
    let mut mdat = Vec::with_capacity(sample_bytes);
    for run in &fragment.runs {
        for sample in &run.samples {
            mdat.extend_from_slice(&sample.payload);
        }
    }
    push_box(out, *b"mdat", &mdat)
}

fn build_moof(fragment: &Fragment) -> Result<Vec<u8>, Error> {
    let mut drafted = Vec::with_capacity(fragment.runs.len());
    let mut sizes = Vec::with_capacity(fragment.runs.len());
    for run in &fragment.runs {
        sizes.push(run_bytes(run)?);
        drafted.push(write_traf(run, 0)?);
    }
    let mfhd = full_boxed(*b"mfhd", 0, 0, &fragment.sequence.to_be_bytes())?;
    let traf_len: usize = drafted.iter().map(Vec::len).sum();
    let moof_len = 8 + mfhd.len() + traf_len;
    let mut body = mfhd;
    let mut cursor = 0usize;
    for (index, run) in fragment.runs.iter().enumerate() {
        let data_offset = i32::try_from(moof_len + 8 + cursor).map_err(|_| Error::Invalid)?;
        let traf = write_traf(run, data_offset)?;
        if traf.len() != drafted[index].len() {
            return Err(Error::Invalid);
        }
        body.extend_from_slice(&traf);
        cursor = cursor.checked_add(sizes[index]).ok_or(Error::Invalid)?;
    }
    let mut moof = Vec::new();
    push_box(&mut moof, *b"moof", &body)?;
    if moof.len() != moof_len {
        return Err(Error::Invalid);
    }
    Ok(moof)
}

fn run_bytes(run: &Run) -> Result<usize, Error> {
    let mut total = 0usize;
    for sample in &run.samples {
        total = total
            .checked_add(sample.payload.len())
            .ok_or(Error::Invalid)?;
    }
    Ok(total)
}

fn write_traf(run: &Run, data_offset: i32) -> Result<Vec<u8>, Error> {
    let mut body = Vec::new();
    push_full(
        &mut body,
        *b"tfhd",
        0,
        TFHD_DEFAULT_BASE_IS_MOOF,
        &run.track_id.to_be_bytes(),
    )?;
    push_full(
        &mut body,
        *b"tfdt",
        1,
        0,
        &run.base_media_decode_time.to_be_bytes(),
    )?;
    write_trun(&mut body, run, data_offset)?;
    let mut traf = Vec::new();
    push_box(&mut traf, *b"traf", &body)?;
    Ok(traf)
}

fn write_trun(out: &mut Vec<u8>, run: &Run, data_offset: i32) -> Result<(), Error> {
    if run.samples.is_empty() {
        return Err(Error::Invalid);
    }
    let count = u32::try_from(run.samples.len()).map_err(|_| Error::Invalid)?;
    let mut body = Vec::new();
    body.extend_from_slice(&count.to_be_bytes());
    body.extend_from_slice(&data_offset.to_be_bytes());
    for sample in &run.samples {
        let size = u32::try_from(sample.payload.len()).map_err(|_| Error::Invalid)?;
        body.extend_from_slice(&sample.duration.to_be_bytes());
        body.extend_from_slice(&size.to_be_bytes());
        body.extend_from_slice(&sample.flags.to_be_bytes());
    }
    push_full(
        out,
        *b"trun",
        0,
        TRUN_DATA_OFFSET | TRUN_DURATION | TRUN_SIZE | TRUN_FLAGS,
        &body,
    )
}

fn write_matrix(out: &mut Vec<u8>) {
    for value in [0x0001_0000i32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// One PCM track and two fragments. The sample entry is `ipcm`, not H.264,
/// AAC, Opus, or VP9. Payload bytes are a counter, not a codec bitstream.
pub fn sample_movie() -> Movie {
    let per = SAMPLE_BYTES / (FRAGMENTS * SAMPLES_PER_FRAGMENT);
    let mut fragments = Vec::with_capacity(FRAGMENTS);
    let mut media_time = 0u64;
    for index in 0..FRAGMENTS {
        let mut samples = Vec::with_capacity(SAMPLES_PER_FRAGMENT);
        for sample_index in 0..SAMPLES_PER_FRAGMENT {
            let mut payload = vec![0u8; per];
            let salt = (index * SAMPLES_PER_FRAGMENT + sample_index) as u8;
            for (byte_index, byte) in payload.iter_mut().enumerate() {
                *byte = (byte_index as u8).wrapping_add(salt);
            }
            let duration = u32::try_from(per / 2).expect("duration fits");
            samples.push(Sample {
                duration,
                flags: SYNC_FLAGS,
                cts: 0,
                payload,
            });
            media_time += u64::from(duration);
        }
        let base = media_time
            - samples
                .iter()
                .map(|sample| u64::from(sample.duration))
                .sum::<u64>();
        fragments.push(Fragment {
            sequence: (index as u32) + 1,
            runs: vec![Run {
                track_id: 1,
                base_media_decode_time: base,
                samples,
            }],
        });
    }
    Movie {
        brand: *b"iso5",
        timescale: SAMPLE_RATE,
        tracks: vec![Track {
            id: 1,
            handler: *b"soun",
            timescale: SAMPLE_RATE,
            width: 0,
            height: 0,
            sample_entry: ipcm_entry(),
            default_duration: 0,
            default_size: 0,
            default_flags: 0,
        }],
        fragments,
    }
}

pub fn sample_file() -> Vec<u8> {
    remux(&sample_movie()).expect("sample movie is valid")
}

/// A one-sample file whose sample-entry fourcc is `codec`.
#[cfg(test)]
pub(crate) fn file_with_codec(codec: [u8; 4]) -> Result<Vec<u8>, Error> {
    let mut entry = ipcm_entry();
    entry[4..8].copy_from_slice(&codec);
    let movie = Movie {
        brand: *b"iso5",
        timescale: SAMPLE_RATE,
        tracks: vec![Track {
            id: 1,
            handler: *b"soun",
            timescale: SAMPLE_RATE,
            width: 0,
            height: 0,
            sample_entry: entry,
            default_duration: 0,
            default_size: 0,
            default_flags: 0,
        }],
        fragments: vec![Fragment {
            sequence: 1,
            runs: vec![Run {
                track_id: 1,
                base_media_decode_time: 0,
                samples: vec![Sample {
                    duration: 3,
                    flags: SYNC_FLAGS,
                    cts: 0,
                    payload: b"opaque".to_vec(),
                }],
            }],
        }],
    };
    remux(&movie)
}

fn ipcm_entry() -> Vec<u8> {
    let mut fixed = Vec::with_capacity(28 + 14);
    fixed.extend_from_slice(&[0u8; 6]);
    fixed.extend_from_slice(&1u16.to_be_bytes());
    fixed.extend_from_slice(&[0u8; 8]);
    fixed.extend_from_slice(&1u16.to_be_bytes());
    fixed.extend_from_slice(&16u16.to_be_bytes());
    fixed.extend_from_slice(&0u16.to_be_bytes());
    fixed.extend_from_slice(&0u16.to_be_bytes());
    fixed.extend_from_slice(&(SAMPLE_RATE << 16).to_be_bytes());
    let pcmc = full_boxed(*b"pcmC", 0, 0, &[0x01, 0x10]).expect("pcmC");
    fixed.extend_from_slice(&pcmc);
    let mut entry = Vec::new();
    push_box(&mut entry, *b"ipcm", &fixed).expect("ipcm");
    entry
}
