//! Zero-copy ISO BMFF (mp4 / m4a / mov) parse and stream-copy remux.
//!
//! Sample payloads stay as ranges into the input. The index (sizes, offsets,
//! deltas) is the only allocation the parser needs.

use crate::error::{Error, Result};

mod write;

pub use write::{build_movie, MovieSpec, TrackSpec};

#[derive(Clone, Debug)]
pub struct Movie<'a> {
    pub data: &'a [u8],
    pub brand: [u8; 4],
    pub ftyp: &'a [u8],
    pub timescale: u32,
    pub duration: u64,
    pub tracks: Vec<Track<'a>>,
    pub moov_pos: usize,
    pub mdat_pos: usize,
    pub fragmented: bool,
}

#[derive(Clone, Debug)]
pub struct Track<'a> {
    pub id: u32,
    pub handler: [u8; 4],
    pub codec: [u8; 4],
    pub timescale: u32,
    pub width: u32,
    pub height: u32,
    pub channels: u16,
    pub bits: u16,
    pub sample_rate: u32,
    /// Full `stsd` box, including its size and type.
    pub stsd: &'a [u8],
    pub stsd_off: usize,
    pub samples: SampleTable,
}

#[derive(Clone, Debug, Default)]
pub struct SampleTable {
    pub sizes: Vec<u32>,
    pub offsets: Vec<u64>,
    pub deltas: Vec<u32>,
    /// 1-based sync sample numbers. Empty means every sample is a sync sample.
    pub sync: Vec<u32>,
}

impl SampleTable {
    pub fn len(&self) -> usize {
        self.sizes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sizes.is_empty()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RemuxOpt {
    pub faststart: bool,
    pub audio_only: bool,
    pub video_only: bool,
}

pub fn parse(data: &[u8]) -> Result<Movie<'_>> {
    if data.len() < 8 {
        return Err(Error::Truncated {
            at: data.len(),
            need: 8 - data.len(),
        });
    }
    let boxes = iter_boxes(data, 0, data.len())?;
    let mut ftyp: &[u8] = &[];
    let mut brand = *b"isom";
    let mut moov: Option<(usize, std::ops::Range<usize>)> = None;
    let mut mdat_pos = usize::MAX;
    let mut fragmented = false;
    for b in &boxes {
        if b.typ == *b"ftyp" {
            ftyp = &data[b.start..b.end];
            if b.payload.len() >= 4 {
                brand.copy_from_slice(&data[b.payload.start..b.payload.start + 4]);
            }
        } else if b.typ == *b"moov" {
            moov = Some((b.start, b.payload.clone()));
        } else if b.typ == *b"mdat" && mdat_pos == usize::MAX {
            mdat_pos = b.start;
        } else if b.typ == *b"moof" {
            fragmented = true;
        }
    }
    let (moov_pos, moov_range) = moov.ok_or(Error::Corrupt("moov atom not found"))?;
    if fragmented {
        return Err(Error::Unsupported(
            "fragmented mp4 (moof) is refused, not flattened",
        ));
    }
    let children = iter_boxes(data, moov_range.start, moov_range.end)?;
    let mut timescale = 1000u32;
    let mut duration = 0u64;
    let mut tracks = Vec::new();
    for c in &children {
        if c.typ == *b"mvhd" {
            let (ts, dur) = parse_mvhd(&data[c.payload.start..c.payload.end])?;
            timescale = ts;
            duration = dur;
        } else if c.typ == *b"trak" {
            tracks.push(parse_trak(data, c.payload.clone())?);
        }
    }
    Ok(Movie {
        data,
        brand,
        ftyp,
        timescale,
        duration,
        tracks,
        moov_pos,
        mdat_pos,
        fragmented,
    })
}

pub fn remux(data: &[u8], opt: RemuxOpt) -> Result<Vec<u8>> {
    if opt.audio_only && opt.video_only {
        return Err(Error::Corrupt("audio_only and video_only"));
    }
    let movie = parse(data)?;
    let mut selected: Vec<&Track<'_>> = Vec::new();
    for t in &movie.tracks {
        let audio = t.handler == *b"soun";
        let video = t.handler == *b"vide";
        let keep = if opt.audio_only {
            audio
        } else if opt.video_only {
            video
        } else {
            true
        };
        if keep {
            if t.samples.is_empty() {
                return Err(Error::Corrupt("track has no samples"));
            }
            selected.push(t);
        }
    }
    if selected.is_empty() {
        return Err(Error::Corrupt("no tracks selected"));
    }
    write::remux_tracks(data, &movie, &selected, opt.faststart)
}

/// Concatenate sample payloads in decode order. Does not decode them.
pub fn elementary<'a>(track: &Track<'_>, data: &'a [u8]) -> Result<Vec<&'a [u8]>> {
    let mut out = Vec::with_capacity(track.samples.len());
    for i in 0..track.samples.len() {
        out.push(sample_slice(data, &track.samples, i)?);
    }
    Ok(out)
}

pub fn append_samples(data: &[u8], table: &SampleTable, out: &mut Vec<u8>) -> Result<()> {
    let n = table.sizes.len();
    if table.offsets.len() != n || table.deltas.len() != n {
        return Err(Error::Corrupt("sample table width"));
    }
    let mut total = 0usize;
    for size in &table.sizes {
        total = total
            .checked_add(*size as usize)
            .ok_or(Error::Corrupt("sample bytes"))?;
    }
    out.reserve(total);
    for i in 0..n {
        out.extend_from_slice(sample_slice(data, table, i)?);
    }
    Ok(())
}

fn sample_slice<'a>(data: &'a [u8], table: &SampleTable, i: usize) -> Result<&'a [u8]> {
    let start = table.offsets[i];
    let end = start
        .checked_add(u64::from(table.sizes[i]))
        .ok_or(Error::Corrupt("sample end"))?;
    let s = usize::try_from(start).map_err(|_| Error::Corrupt("offset"))?;
    let e = usize::try_from(end).map_err(|_| Error::Corrupt("offset"))?;
    if s > e || e > data.len() {
        return Err(Error::Corrupt("sample extends past end"));
    }
    Ok(&data[s..e])
}

#[derive(Clone, Debug)]
struct BoxSpan {
    typ: [u8; 4],
    start: usize,
    end: usize,
    payload: std::ops::Range<usize>,
}

fn iter_boxes(data: &[u8], start: usize, end: usize) -> Result<Vec<BoxSpan>> {
    let mut out = Vec::new();
    let mut off = start;
    while off + 8 <= end {
        let size32 = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        let typ = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        let (header, size) = if size32 == 1 {
            if off + 16 > end {
                return Err(Error::Truncated {
                    at: end,
                    need: off + 16 - end,
                });
            }
            let size = u64::from_be_bytes([
                data[off + 8],
                data[off + 9],
                data[off + 10],
                data[off + 11],
                data[off + 12],
                data[off + 13],
                data[off + 14],
                data[off + 15],
            ]);
            (16usize, size)
        } else if size32 == 0 {
            (8usize, (end - off) as u64)
        } else {
            (8usize, u64::from(size32))
        };
        if size < header as u64 {
            return Err(Error::Corrupt("box smaller than header"));
        }
        let size_us = usize::try_from(size).map_err(|_| Error::Corrupt("box size"))?;
        let box_end = off.checked_add(size_us).ok_or(Error::Corrupt("box end"))?;
        if box_end > end {
            return Err(Error::Truncated {
                at: end,
                need: box_end - end,
            });
        }
        out.push(BoxSpan {
            typ,
            start: off,
            end: box_end,
            payload: off + header..box_end,
        });
        off = box_end;
    }
    if off != end && end - off >= 8 {
        return Err(Error::Corrupt("trailing boxes"));
    }
    Ok(out)
}

fn parse_mvhd(body: &[u8]) -> Result<(u32, u64)> {
    if body.len() < 4 {
        return Err(Error::Corrupt("mvhd"));
    }
    let version = body[0];
    if version == 0 {
        if body.len() < 20 {
            return Err(Error::Corrupt("mvhd"));
        }
        let timescale = u32::from_be_bytes([body[12], body[13], body[14], body[15]]);
        let duration = u32::from_be_bytes([body[16], body[17], body[18], body[19]]) as u64;
        Ok((timescale, duration))
    } else if version == 1 {
        if body.len() < 32 {
            return Err(Error::Corrupt("mvhd"));
        }
        let timescale = u32::from_be_bytes([body[20], body[21], body[22], body[23]]);
        let duration = u64::from_be_bytes([
            body[24], body[25], body[26], body[27], body[28], body[29], body[30], body[31],
        ]);
        Ok((timescale, duration))
    } else {
        Err(Error::Corrupt("mvhd version"))
    }
}

fn parse_trak(data: &[u8], range: std::ops::Range<usize>) -> Result<Track<'_>> {
    let children = iter_boxes(data, range.start, range.end)?;
    let mut id = 0u32;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut mdia: Option<std::ops::Range<usize>> = None;
    for c in &children {
        if c.typ == *b"tkhd" {
            let (tid, w, h) = parse_tkhd(&data[c.payload.start..c.payload.end])?;
            id = tid;
            width = w;
            height = h;
        } else if c.typ == *b"mdia" {
            mdia = Some(c.payload.clone());
        }
    }
    let mdia = mdia.ok_or(Error::Corrupt("trak missing mdia"))?;
    let mut timescale = 0u32;
    let mut handler = *b"unkn";
    let mut stbl: Option<std::ops::Range<usize>> = None;
    for c in iter_boxes(data, mdia.start, mdia.end)? {
        if c.typ == *b"mdhd" {
            timescale = parse_mdhd_timescale(&data[c.payload.start..c.payload.end])?;
        } else if c.typ == *b"hdlr" {
            handler = parse_hdlr(&data[c.payload.start..c.payload.end])?;
        } else if c.typ == *b"minf" {
            for m in iter_boxes(data, c.payload.start, c.payload.end)? {
                if m.typ == *b"stbl" {
                    stbl = Some(m.payload.clone());
                }
            }
        }
    }
    let stbl = stbl.ok_or(Error::Corrupt("trak missing stbl"))?;
    let mut stsd: &[u8] = &[];
    let mut stsd_off = 0usize;
    let mut samples = SampleTable::default();
    let mut codec = *b"none";
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut sample_rate = 0u32;
    for c in iter_boxes(data, stbl.start, stbl.end)? {
        let body = &data[c.payload.start..c.payload.end];
        if c.typ == *b"stsd" {
            stsd = &data[c.start..c.end];
            stsd_off = c.start;
            let parsed = parse_stsd(body, handler)?;
            codec = parsed.0;
            channels = parsed.1;
            bits = parsed.2;
            sample_rate = parsed.3;
        } else if c.typ == *b"stts" {
            samples.deltas = parse_stts(body)?;
        } else if c.typ == *b"stsz" {
            samples.sizes = parse_stsz(body)?;
        } else if c.typ == *b"stz2" {
            return Err(Error::Unsupported("stz2 compact sample sizes"));
        } else if c.typ == *b"stco" {
            samples.offsets = parse_chunk_offsets(body, false)?;
        } else if c.typ == *b"co64" {
            samples.offsets = parse_chunk_offsets(body, true)?;
        } else if c.typ == *b"stss" {
            samples.sync = parse_stss(body)?;
        }
    }
    // Re-read stsc now that chunk offsets exist. The loop above only stored stco.
    let mut stsc_entries: Vec<Stsc> = Vec::new();
    for c in iter_boxes(data, stbl.start, stbl.end)? {
        if c.typ == *b"stsc" {
            stsc_entries = parse_stsc(&data[c.payload.start..c.payload.end])?;
        }
    }
    expand_chunks(&mut samples, &stsc_entries)?;
    if samples.deltas.len() != samples.sizes.len() {
        return Err(Error::Corrupt("stts count"));
    }
    if sample_rate == 0 && handler == *b"soun" {
        sample_rate = timescale;
    }
    Ok(Track {
        id,
        handler,
        codec,
        timescale,
        width,
        height,
        channels,
        bits,
        sample_rate,
        stsd,
        stsd_off,
        samples,
    })
}

fn parse_tkhd(body: &[u8]) -> Result<(u32, u32, u32)> {
    if body.is_empty() {
        return Err(Error::Corrupt("tkhd"));
    }
    let version = body[0];
    let (id_at, w_at) = if version == 0 {
        (12usize, body.len().saturating_sub(8))
    } else if version == 1 {
        (20usize, body.len().saturating_sub(8))
    } else {
        return Err(Error::Corrupt("tkhd version"));
    };
    if body.len() < id_at + 4 || body.len() < w_at + 8 {
        return Err(Error::Corrupt("tkhd"));
    }
    let id = u32::from_be_bytes([
        body[id_at],
        body[id_at + 1],
        body[id_at + 2],
        body[id_at + 3],
    ]);
    let width = u32::from_be_bytes([body[w_at], body[w_at + 1], body[w_at + 2], body[w_at + 3]]);
    let height = u32::from_be_bytes([
        body[w_at + 4],
        body[w_at + 5],
        body[w_at + 6],
        body[w_at + 7],
    ]);
    Ok((id, width, height))
}

fn parse_mdhd_timescale(body: &[u8]) -> Result<u32> {
    if body.is_empty() {
        return Err(Error::Corrupt("mdhd"));
    }
    let at = if body[0] == 0 {
        12
    } else if body[0] == 1 {
        20
    } else {
        return Err(Error::Corrupt("mdhd version"));
    };
    if body.len() < at + 4 {
        return Err(Error::Corrupt("mdhd"));
    }
    Ok(u32::from_be_bytes([
        body[at],
        body[at + 1],
        body[at + 2],
        body[at + 3],
    ]))
}

fn parse_hdlr(body: &[u8]) -> Result<[u8; 4]> {
    // full box already stripped: version/flags (4) + pre_defined (4) + handler (4)
    if body.len() < 12 {
        return Err(Error::Corrupt("hdlr"));
    }
    Ok([body[8], body[9], body[10], body[11]])
}

fn parse_stsd(body: &[u8], handler: [u8; 4]) -> Result<([u8; 4], u16, u16, u32)> {
    if body.len() < 8 {
        return Err(Error::Corrupt("stsd"));
    }
    let count = u32::from_be_bytes([body[4], body[5], body[6], body[7]]);
    if count == 0 {
        return Err(Error::Corrupt("stsd empty"));
    }
    if body.len() < 16 {
        return Err(Error::Corrupt("stsd entry"));
    }
    let entry_size = u32::from_be_bytes([body[8], body[9], body[10], body[11]]) as usize;
    let codec = [body[12], body[13], body[14], body[15]];
    if entry_size < 16 || 8 + entry_size > body.len() {
        return Err(Error::Corrupt("stsd entry size"));
    }
    let entry = &body[8..8 + entry_size];
    let mut channels = 0u16;
    let mut bits = 0u16;
    let mut rate = 0u32;
    if handler == *b"soun" && entry.len() >= 36 {
        channels = u16::from_be_bytes([entry[24], entry[25]]);
        bits = u16::from_be_bytes([entry[26], entry[27]]);
        let raw = u32::from_be_bytes([entry[32], entry[33], entry[34], entry[35]]);
        rate = raw >> 16;
    }
    Ok((codec, channels, bits, rate))
}

fn fullbox_rest(body: &[u8]) -> Result<&[u8]> {
    if body.len() < 4 {
        return Err(Error::Corrupt("full box"));
    }
    Ok(&body[4..])
}

fn parse_stts(body: &[u8]) -> Result<Vec<u32>> {
    let rest = fullbox_rest(body)?;
    if rest.len() < 4 {
        return Err(Error::Corrupt("stts"));
    }
    let count = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if count > (1 << 26) || rest.len() < 4 + count * 8 {
        return Err(Error::Corrupt("stts"));
    }
    let mut deltas = Vec::new();
    for i in 0..count {
        let o = 4 + i * 8;
        let n = u32::from_be_bytes([rest[o], rest[o + 1], rest[o + 2], rest[o + 3]]) as usize;
        let delta = u32::from_be_bytes([rest[o + 4], rest[o + 5], rest[o + 6], rest[o + 7]]);
        if n > (1 << 26) || deltas.len().saturating_add(n) > (1 << 26) {
            return Err(Error::Corrupt("stts expansion"));
        }
        deltas.resize(deltas.len() + n, delta);
    }
    Ok(deltas)
}

fn parse_stsz(body: &[u8]) -> Result<Vec<u32>> {
    let rest = fullbox_rest(body)?;
    if rest.len() < 8 {
        return Err(Error::Corrupt("stsz"));
    }
    let constant = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]);
    let count = u32::from_be_bytes([rest[4], rest[5], rest[6], rest[7]]) as usize;
    if count > (1 << 26) {
        return Err(Error::Corrupt("stsz count"));
    }
    if constant != 0 {
        return Ok(vec![constant; count]);
    }
    if rest.len() < 8 + count * 4 {
        return Err(Error::Corrupt("stsz table"));
    }
    let mut sizes = Vec::with_capacity(count);
    for i in 0..count {
        let o = 8 + i * 4;
        sizes.push(u32::from_be_bytes([
            rest[o],
            rest[o + 1],
            rest[o + 2],
            rest[o + 3],
        ]));
    }
    Ok(sizes)
}

fn parse_chunk_offsets(body: &[u8], wide: bool) -> Result<Vec<u64>> {
    let rest = fullbox_rest(body)?;
    if rest.len() < 4 {
        return Err(Error::Corrupt("stco"));
    }
    let count = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    let width = if wide { 8 } else { 4 };
    if count > (1 << 26) || rest.len() < 4 + count * width {
        return Err(Error::Corrupt("stco"));
    }
    let mut offsets = Vec::with_capacity(count);
    for i in 0..count {
        let o = 4 + i * width;
        if wide {
            offsets.push(u64::from_be_bytes([
                rest[o],
                rest[o + 1],
                rest[o + 2],
                rest[o + 3],
                rest[o + 4],
                rest[o + 5],
                rest[o + 6],
                rest[o + 7],
            ]));
        } else {
            offsets
                .push(u32::from_be_bytes([rest[o], rest[o + 1], rest[o + 2], rest[o + 3]]) as u64);
        }
    }
    Ok(offsets)
}

fn parse_stss(body: &[u8]) -> Result<Vec<u32>> {
    let rest = fullbox_rest(body)?;
    if rest.len() < 4 {
        return Err(Error::Corrupt("stss"));
    }
    let count = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if rest.len() < 4 + count * 4 || count > (1 << 26) {
        return Err(Error::Corrupt("stss"));
    }
    let mut sync = Vec::with_capacity(count);
    for i in 0..count {
        let o = 4 + i * 4;
        sync.push(u32::from_be_bytes([
            rest[o],
            rest[o + 1],
            rest[o + 2],
            rest[o + 3],
        ]));
    }
    Ok(sync)
}

struct Stsc {
    first_chunk: u32,
    samples_per_chunk: u32,
    desc: u32,
}

fn parse_stsc(body: &[u8]) -> Result<Vec<Stsc>> {
    let rest = fullbox_rest(body)?;
    if rest.len() < 4 {
        return Err(Error::Corrupt("stsc"));
    }
    let count = u32::from_be_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
    if rest.len() < 4 + count * 12 || count > (1 << 20) {
        return Err(Error::Corrupt("stsc"));
    }
    let mut entries = Vec::with_capacity(count);
    for i in 0..count {
        let o = 4 + i * 12;
        entries.push(Stsc {
            first_chunk: u32::from_be_bytes([rest[o], rest[o + 1], rest[o + 2], rest[o + 3]]),
            samples_per_chunk: u32::from_be_bytes([
                rest[o + 4],
                rest[o + 5],
                rest[o + 6],
                rest[o + 7],
            ]),
            desc: u32::from_be_bytes([rest[o + 8], rest[o + 9], rest[o + 10], rest[o + 11]]),
        });
    }
    Ok(entries)
}

fn expand_chunks(samples: &mut SampleTable, stsc: &[Stsc]) -> Result<()> {
    let nchunks = samples.offsets.len();
    if nchunks == 0 {
        if samples.sizes.is_empty() {
            return Ok(());
        }
        return Err(Error::Corrupt("samples without chunk offsets"));
    }
    if stsc.is_empty() {
        return Err(Error::Corrupt("stsc"));
    }
    let mut per_chunk = vec![0u32; nchunks];
    for (ei, e) in stsc.iter().enumerate() {
        if e.first_chunk == 0 || e.desc == 0 {
            return Err(Error::Corrupt("stsc entry"));
        }
        let from = (e.first_chunk as usize) - 1;
        let to = if ei + 1 < stsc.len() {
            let next = stsc[ei + 1].first_chunk as usize;
            if next == 0 {
                return Err(Error::Corrupt("stsc entry"));
            }
            next - 1
        } else {
            nchunks
        };
        if from > to || to > nchunks {
            return Err(Error::Corrupt("stsc range"));
        }
        for slot in &mut per_chunk[from..to] {
            *slot = e.samples_per_chunk;
        }
    }
    let total: usize = per_chunk.iter().map(|n| *n as usize).sum();
    if total != samples.sizes.len() {
        return Err(Error::Corrupt("stsc sample count"));
    }
    let mut offsets = Vec::with_capacity(total);
    let mut sample_i = 0usize;
    for (ci, &ns) in per_chunk.iter().enumerate() {
        let mut off = samples.offsets[ci];
        for _ in 0..ns {
            let sz = samples.sizes[sample_i];
            offsets.push(off);
            off = off
                .checked_add(u64::from(sz))
                .ok_or(Error::Corrupt("chunk walk"))?;
            sample_i += 1;
        }
    }
    samples.offsets = offsets;
    Ok(())
}
