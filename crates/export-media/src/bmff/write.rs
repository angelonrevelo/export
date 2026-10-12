use super::{append_samples, Movie, Track};
use crate::error::{Error, Result};

#[derive(Clone, Debug)]
pub struct MovieSpec {
    pub brand: [u8; 4],
    pub timescale: u32,
    pub faststart: bool,
    /// Round-robin sample payloads so a track's samples are not one contiguous run.
    pub interleave: bool,
    pub tracks: Vec<TrackSpec>,
}

#[derive(Clone, Debug)]
pub struct TrackSpec {
    pub handler: [u8; 4],
    pub codec: [u8; 4],
    pub timescale: u32,
    pub width: u16,
    pub height: u16,
    pub channels: u16,
    pub bits: u16,
    pub sample_rate: u32,
    pub deltas: Vec<u32>,
    pub payloads: Vec<Vec<u8>>,
}

pub fn build_movie(spec: &MovieSpec) -> Result<Vec<u8>> {
    if spec.tracks.is_empty() {
        return Err(Error::Corrupt("no tracks"));
    }
    for t in &spec.tracks {
        if t.payloads.len() != t.deltas.len() || t.payloads.is_empty() {
            return Err(Error::Corrupt("track samples"));
        }
    }
    let ftyp = ftyp_bytes(spec.brand);
    let layout = layout_mdat(spec);
    let mut moov_probe = Vec::new();
    write_moov(&mut moov_probe, spec, &layout.rel, 0)?;
    let moov_len = moov_probe.len();
    let base = if spec.faststart {
        (ftyp.len() + moov_len + 8) as u64
    } else {
        (ftyp.len() + 8) as u64
    };
    let mut moov = Vec::with_capacity(moov_len);
    write_moov(&mut moov, spec, &layout.rel, base)?;
    if moov.len() != moov_len {
        return Err(Error::Corrupt("moov size changed"));
    }
    let mut mdat = Vec::with_capacity(8 + layout.bytes.len());
    push_u32(&mut mdat, (8 + layout.bytes.len()) as u32);
    mdat.extend_from_slice(b"mdat");
    mdat.extend_from_slice(&layout.bytes);

    let mut out = Vec::with_capacity(ftyp.len() + moov.len() + mdat.len());
    out.extend_from_slice(&ftyp);
    if spec.faststart {
        out.extend_from_slice(&moov);
        out.extend_from_slice(&mdat);
    } else {
        out.extend_from_slice(&mdat);
        out.extend_from_slice(&moov);
    }
    Ok(out)
}

struct Layout {
    rel: Vec<Vec<u64>>,
    bytes: Vec<u8>,
}

fn layout_mdat(spec: &MovieSpec) -> Layout {
    let mut rel = vec![Vec::new(); spec.tracks.len()];
    let mut bytes = Vec::new();
    if spec.interleave {
        let n = spec
            .tracks
            .iter()
            .map(|t| t.payloads.len())
            .max()
            .unwrap_or(0);
        for i in 0..n {
            for (ti, t) in spec.tracks.iter().enumerate() {
                if i < t.payloads.len() {
                    rel[ti].push(bytes.len() as u64);
                    bytes.extend_from_slice(&t.payloads[i]);
                }
            }
        }
    } else {
        for (ti, t) in spec.tracks.iter().enumerate() {
            for payload in &t.payloads {
                rel[ti].push(bytes.len() as u64);
                bytes.extend_from_slice(payload);
            }
        }
    }
    Layout { rel, bytes }
}

pub fn remux_tracks(
    data: &[u8],
    movie: &Movie<'_>,
    tracks: &[&Track<'_>],
    faststart: bool,
) -> Result<Vec<u8>> {
    let ftyp = if movie.ftyp.is_empty() {
        ftyp_bytes(movie.brand)
    } else {
        movie.ftyp.to_vec()
    };
    let mut runs = Vec::with_capacity(tracks.len());
    for t in tracks {
        let mut buf = Vec::new();
        append_samples(data, &t.samples, &mut buf)?;
        runs.push(buf);
    }
    let mut rel = Vec::with_capacity(runs.len());
    let mut cursor = 0u64;
    for run in &runs {
        rel.push(cursor);
        cursor += run.len() as u64;
    }
    let mut moov = Vec::new();
    write_moov_runs(&mut moov, movie, tracks, &rel, 0)?;
    let moov_len = moov.len();
    let base = if faststart {
        (ftyp.len() + moov_len + 8) as u64
    } else {
        (ftyp.len() + 8) as u64
    };
    moov.clear();
    write_moov_runs(&mut moov, movie, tracks, &rel, base)?;
    if moov.len() != moov_len {
        return Err(Error::Corrupt("moov size changed"));
    }
    let mut mdat = Vec::with_capacity(8 + cursor as usize);
    push_u32(&mut mdat, (8 + cursor) as u32);
    mdat.extend_from_slice(b"mdat");
    for run in &runs {
        mdat.extend_from_slice(run);
    }
    let mut out = Vec::with_capacity(ftyp.len() + moov.len() + mdat.len());
    out.extend_from_slice(&ftyp);
    if faststart {
        out.extend_from_slice(&moov);
        out.extend_from_slice(&mdat);
    } else {
        out.extend_from_slice(&mdat);
        out.extend_from_slice(&moov);
    }
    Ok(out)
}

fn write_moov_runs(
    out: &mut Vec<u8>,
    movie: &Movie<'_>,
    tracks: &[&Track<'_>],
    rel: &[u64],
    base: u64,
) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"moov");
    write_mvhd_runs(out, movie, tracks)?;
    for (i, t) in tracks.iter().enumerate() {
        write_trak_run(out, t, i as u32 + 1, movie.timescale.max(1), rel[i], base)?;
    }
    finish_box(out, start);
    Ok(())
}

fn write_mvhd_runs(out: &mut Vec<u8>, movie: &Movie<'_>, tracks: &[&Track<'_>]) -> Result<()> {
    let movie_ts = movie.timescale.max(1);
    let mut duration = 0u64;
    for t in tracks {
        let ticks: u64 = t.samples.deltas.iter().map(|d| u64::from(*d)).sum();
        let scaled = ticks.saturating_mul(u64::from(movie_ts)) / u64::from(t.timescale.max(1));
        duration = duration.max(scaled);
    }
    if duration > u64::from(u32::MAX) {
        return Err(Error::Unsupported("movie duration needs mvhd version 1"));
    }
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"mvhd");
    push_u32(out, 0);
    push_u32(out, 0);
    push_u32(out, 0);
    push_u32(out, movie_ts);
    push_u32(out, duration as u32);
    push_u32(out, 0x0001_0000);
    push_u16(out, 0x0100);
    push_u16(out, 0);
    push_u32(out, 0);
    push_u32(out, 0);
    write_matrix(out);
    for _ in 0..6 {
        push_u32(out, 0);
    }
    push_u32(out, tracks.len() as u32 + 1);
    finish_box(out, start);
    Ok(())
}

fn write_trak_run(
    out: &mut Vec<u8>,
    t: &Track<'_>,
    id: u32,
    movie_timescale: u32,
    rel: u64,
    base: u64,
) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"trak");
    let spec = track_spec_meta(t);
    write_tkhd(out, &spec, id, movie_timescale)?;
    write_mdia_run(out, t, &spec, rel, base)?;
    finish_box(out, start);
    Ok(())
}

fn track_spec_meta(t: &Track<'_>) -> TrackSpec {
    TrackSpec {
        handler: t.handler,
        codec: t.codec,
        timescale: t.timescale.max(1),
        width: (t.width >> 16) as u16,
        height: (t.height >> 16) as u16,
        channels: t.channels,
        bits: t.bits,
        sample_rate: t.sample_rate,
        deltas: t.samples.deltas.clone(),
        payloads: Vec::new(),
    }
}

fn write_mdia_run(
    out: &mut Vec<u8>,
    t: &Track<'_>,
    spec: &TrackSpec,
    rel: u64,
    base: u64,
) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"mdia");
    write_mdhd(out, spec)?;
    write_hdlr(out, spec.handler);
    let minf = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"minf");
    if spec.handler == *b"soun" {
        write_smhd(out);
    } else {
        write_vmhd(out);
    }
    write_dinf(out);
    write_stbl_run(out, t, rel, base)?;
    finish_box(out, minf);
    finish_box(out, start);
    Ok(())
}

fn write_stbl_run(out: &mut Vec<u8>, t: &Track<'_>, rel: u64, base: u64) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stbl");
    if t.stsd.is_empty() {
        return Err(Error::Corrupt("missing stsd"));
    }
    out.extend_from_slice(t.stsd);
    write_stts(out, &t.samples.deltas);
    // Samples were gathered into one contiguous run, so one chunk holds them all.
    write_stsc(out, t.samples.len() as u32);
    write_stsz_sizes(out, &t.samples.sizes);
    write_stco(out, &[rel], base)?;
    if t.handler == *b"vide" {
        if t.samples.sync.is_empty() {
            write_stss_all(out, t.samples.len() as u32);
        } else {
            write_stss_list(out, &t.samples.sync);
        }
    }
    finish_box(out, start);
    Ok(())
}

fn write_stsz_sizes(out: &mut Vec<u8>, sizes: &[u32]) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stsz");
    push_u32(out, 0);
    let constant = !sizes.is_empty() && sizes.iter().all(|s| *s == sizes[0]);
    if constant {
        push_u32(out, sizes[0]);
        push_u32(out, sizes.len() as u32);
    } else {
        push_u32(out, 0);
        push_u32(out, sizes.len() as u32);
        for s in sizes {
            push_u32(out, *s);
        }
    }
    finish_box(out, start);
}

fn write_stss_list(out: &mut Vec<u8>, sync: &[u32]) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stss");
    push_u32(out, 0);
    push_u32(out, sync.len() as u32);
    for n in sync {
        push_u32(out, *n);
    }
    finish_box(out, start);
}

fn ftyp_bytes(brand: [u8; 4]) -> Vec<u8> {
    let compat: &[[u8; 4]] = match &brand {
        b"qt  " => &[*b"qt  "],
        b"M4A " => &[*b"M4A ", *b"mp42", *b"isom"],
        _ => &[*b"isom", *b"iso2", *b"mp41"],
    };
    let mut body = Vec::new();
    body.extend_from_slice(&brand);
    body.extend_from_slice(&0u32.to_be_bytes());
    for c in compat {
        body.extend_from_slice(c);
    }
    let mut out = Vec::new();
    push_u32(&mut out, (8 + body.len()) as u32);
    out.extend_from_slice(b"ftyp");
    out.extend_from_slice(&body);
    out
}

fn write_moov(out: &mut Vec<u8>, spec: &MovieSpec, rel: &[Vec<u64>], base: u64) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"moov");
    write_mvhd(out, spec)?;
    for (i, t) in spec.tracks.iter().enumerate() {
        write_trak(out, t, i as u32 + 1, spec.timescale, &rel[i], base)?;
    }
    finish_box(out, start);
    Ok(())
}

fn write_mvhd(out: &mut Vec<u8>, spec: &MovieSpec) -> Result<()> {
    let mut duration = 0u64;
    for t in &spec.tracks {
        let ticks: u64 = t.deltas.iter().map(|d| u64::from(*d)).sum();
        let scaled =
            ticks.saturating_mul(u64::from(spec.timescale)) / u64::from(t.timescale.max(1));
        duration = duration.max(scaled);
    }
    if duration > u64::from(u32::MAX) {
        return Err(Error::Unsupported("movie duration needs mvhd version 1"));
    }
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"mvhd");
    push_u32(out, 0); // version/flags
    push_u32(out, 0); // creation
    push_u32(out, 0); // modification
    push_u32(out, spec.timescale.max(1));
    push_u32(out, duration as u32);
    push_u32(out, 0x0001_0000); // rate
    push_u16(out, 0x0100); // volume
    push_u16(out, 0);
    push_u32(out, 0);
    push_u32(out, 0);
    write_matrix(out);
    for _ in 0..6 {
        push_u32(out, 0);
    }
    push_u32(out, spec.tracks.len() as u32 + 1);
    finish_box(out, start);
    Ok(())
}

fn write_trak(
    out: &mut Vec<u8>,
    t: &TrackSpec,
    id: u32,
    movie_timescale: u32,
    rel: &[u64],
    base: u64,
) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"trak");
    write_tkhd(out, t, id, movie_timescale)?;
    write_mdia(out, t, rel, base)?;
    finish_box(out, start);
    Ok(())
}

fn write_tkhd(out: &mut Vec<u8>, t: &TrackSpec, id: u32, movie_timescale: u32) -> Result<()> {
    let ticks: u64 = t.deltas.iter().map(|d| u64::from(*d)).sum();
    let duration =
        ticks.saturating_mul(u64::from(movie_timescale.max(1))) / u64::from(t.timescale.max(1));
    if duration > u64::from(u32::MAX) {
        return Err(Error::Unsupported("track duration needs tkhd version 1"));
    }
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"tkhd");
    push_u32(out, 0x7); // enabled, in movie, in preview
    push_u32(out, 0);
    push_u32(out, 0);
    push_u32(out, id);
    push_u32(out, 0);
    push_u32(out, duration as u32);
    push_u32(out, 0);
    push_u32(out, 0);
    push_u16(out, 0); // layer
    push_u16(out, 0); // alternate group
    if t.handler == *b"soun" {
        push_u16(out, 0x0100);
    } else {
        push_u16(out, 0);
    }
    push_u16(out, 0);
    write_matrix(out);
    if t.handler == *b"vide" {
        push_u32(out, u32::from(t.width) << 16);
        push_u32(out, u32::from(t.height) << 16);
    } else {
        push_u32(out, 0);
        push_u32(out, 0);
    }
    finish_box(out, start);
    Ok(())
}

fn write_mdia(out: &mut Vec<u8>, t: &TrackSpec, rel: &[u64], base: u64) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"mdia");
    write_mdhd(out, t)?;
    write_hdlr(out, t.handler);
    write_minf(out, t, rel, base)?;
    finish_box(out, start);
    Ok(())
}

fn write_mdhd(out: &mut Vec<u8>, t: &TrackSpec) -> Result<()> {
    let duration: u64 = t.deltas.iter().map(|d| u64::from(*d)).sum();
    if duration > u64::from(u32::MAX) {
        return Err(Error::Unsupported("mdhd version 1"));
    }
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"mdhd");
    push_u32(out, 0);
    push_u32(out, 0);
    push_u32(out, 0);
    push_u32(out, t.timescale.max(1));
    push_u32(out, duration as u32);
    push_u16(out, 0x55c4); // und
    push_u16(out, 0);
    finish_box(out, start);
    Ok(())
}

fn write_hdlr(out: &mut Vec<u8>, handler: [u8; 4]) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"hdlr");
    push_u32(out, 0);
    push_u32(out, 0);
    out.extend_from_slice(&handler);
    push_u32(out, 0);
    push_u32(out, 0);
    push_u32(out, 0);
    let name: &[u8] = if handler == *b"soun" {
        b"SoundHandler\0"
    } else {
        b"VideoHandler\0"
    };
    out.extend_from_slice(name);
    finish_box(out, start);
}

fn write_minf(out: &mut Vec<u8>, t: &TrackSpec, rel: &[u64], base: u64) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"minf");
    if t.handler == *b"soun" {
        write_smhd(out);
    } else {
        write_vmhd(out);
    }
    write_dinf(out);
    write_stbl(out, t, rel, base)?;
    finish_box(out, start);
    Ok(())
}

fn write_smhd(out: &mut Vec<u8>) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"smhd");
    push_u32(out, 0);
    push_u16(out, 0);
    push_u16(out, 0);
    finish_box(out, start);
}

fn write_vmhd(out: &mut Vec<u8>) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"vmhd");
    push_u32(out, 1);
    push_u16(out, 0);
    push_u16(out, 0);
    push_u16(out, 0);
    push_u16(out, 0);
    finish_box(out, start);
}

fn write_dinf(out: &mut Vec<u8>) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"dinf");
    let dref = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"dref");
    push_u32(out, 0);
    push_u32(out, 1);
    let url = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"url ");
    push_u32(out, 1); // self-contained
    finish_box(out, url);
    finish_box(out, dref);
    finish_box(out, start);
}

fn write_stbl(out: &mut Vec<u8>, t: &TrackSpec, rel: &[u64], base: u64) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stbl");
    write_stsd(out, t);
    write_stts(out, &t.deltas);
    // One sample per chunk. Offsets below are per sample, including interleaved layouts.
    write_stsc(out, 1);
    write_stsz(out, &t.payloads);
    write_stco(out, rel, base)?;
    if t.handler == *b"vide" {
        write_stss_all(out, t.payloads.len() as u32);
    }
    finish_box(out, start);
    Ok(())
}

fn write_stsd(out: &mut Vec<u8>, t: &TrackSpec) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stsd");
    push_u32(out, 0);
    push_u32(out, 1);
    if t.handler == *b"soun" {
        write_audio_entry(out, t);
    } else {
        write_video_entry(out, t);
    }
    finish_box(out, start);
}

fn write_audio_entry(out: &mut Vec<u8>, t: &TrackSpec) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(&t.codec);
    out.extend_from_slice(&[0; 6]);
    push_u16(out, 1); // data reference
    push_u16(out, 0); // version
    push_u16(out, 0); // revision
    push_u32(out, 0); // vendor
    push_u16(out, t.channels.max(1));
    push_u16(out, t.bits.max(16));
    push_u16(out, 0);
    push_u16(out, 0);
    push_u32(out, t.sample_rate.max(1) << 16);
    finish_box(out, start);
}

fn write_video_entry(out: &mut Vec<u8>, t: &TrackSpec) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(&t.codec);
    out.extend_from_slice(&[0; 6]);
    push_u16(out, 1);
    push_u16(out, 0);
    push_u16(out, 0);
    push_u32(out, 0);
    push_u32(out, 0);
    push_u32(out, 0);
    push_u16(out, t.width.max(1));
    push_u16(out, t.height.max(1));
    push_u32(out, 0x0048_0000);
    push_u32(out, 0x0048_0000);
    push_u32(out, 0);
    push_u16(out, 1);
    out.extend_from_slice(&[0; 32]);
    push_u16(out, 0x0018);
    push_u16(out, 0xffff);
    finish_box(out, start);
}

fn write_stts(out: &mut Vec<u8>, deltas: &[u32]) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stts");
    push_u32(out, 0);
    let mut runs: Vec<(u32, u32)> = Vec::new();
    for &d in deltas {
        if let Some(last) = runs.last_mut() {
            if last.1 == d {
                last.0 += 1;
                continue;
            }
        }
        runs.push((1, d));
    }
    push_u32(out, runs.len() as u32);
    for (n, d) in runs {
        push_u32(out, n);
        push_u32(out, d);
    }
    finish_box(out, start);
}

fn write_stsc(out: &mut Vec<u8>, samples_per_chunk: u32) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stsc");
    push_u32(out, 0);
    push_u32(out, 1);
    push_u32(out, 1); // first chunk
    push_u32(out, samples_per_chunk);
    push_u32(out, 1);
    finish_box(out, start);
}

fn write_stsz(out: &mut Vec<u8>, payloads: &[Vec<u8>]) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stsz");
    push_u32(out, 0);
    let constant = payloads.iter().all(|p| p.len() == payloads[0].len());
    if constant {
        push_u32(out, payloads[0].len() as u32);
        push_u32(out, payloads.len() as u32);
    } else {
        push_u32(out, 0);
        push_u32(out, payloads.len() as u32);
        for p in payloads {
            push_u32(out, p.len() as u32);
        }
    }
    finish_box(out, start);
}

fn write_stco(out: &mut Vec<u8>, rel: &[u64], base: u64) -> Result<()> {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stco");
    push_u32(out, 0);
    push_u32(out, rel.len() as u32);
    for &r in rel {
        let abs = base + r;
        if abs > u64::from(u32::MAX) {
            return Err(Error::Unsupported("co64"));
        }
        push_u32(out, abs as u32);
    }
    finish_box(out, start);
    Ok(())
}

fn write_stss_all(out: &mut Vec<u8>, n: u32) {
    let start = out.len();
    push_u32(out, 0);
    out.extend_from_slice(b"stss");
    push_u32(out, 0);
    push_u32(out, n);
    for i in 1..=n {
        push_u32(out, i);
    }
    finish_box(out, start);
}

fn write_matrix(out: &mut Vec<u8>) {
    for v in [0x0001_0000u32, 0, 0, 0, 0x0001_0000, 0, 0, 0, 0x4000_0000] {
        push_u32(out, v);
    }
}

fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn push_u16(out: &mut Vec<u8>, v: u16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn finish_box(out: &mut [u8], start: usize) {
    let size = (out.len() - start) as u32;
    out[start..start + 4].copy_from_slice(&size.to_be_bytes());
}
