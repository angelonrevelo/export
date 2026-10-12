//! Stream-copy concat of two non-fragmented MP4s that share a sample entry.
//! Samples are appended. Nothing is decoded or re-encoded.

use super::write::remux_tracks;
use super::{append_samples, parse, Movie, SampleTable, Track};
use crate::error::{Error, Result};
use crate::sniff;

pub fn concat_mp4(a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
    let kind_a = sniff::sniff(a);
    let kind_b = sniff::sniff(b);
    if !kind_a.is_bmff() {
        return Err(Error::Kind {
            found: kind_a,
            job: "concat",
        });
    }
    if !kind_b.is_bmff() {
        return Err(Error::Kind {
            found: kind_b,
            job: "concat",
        });
    }
    let movie_a = parse(a)?;
    let movie_b = parse(b)?;
    if movie_a.tracks.len() != movie_b.tracks.len() {
        return Err(Error::Unsupported("track count mismatch"));
    }
    if movie_a.tracks.is_empty() {
        return Err(Error::Unsupported("no tracks"));
    }
    if movie_a.timescale != movie_b.timescale {
        return Err(Error::Unsupported("movie timescale mismatch"));
    }
    let mut arena = Vec::new();
    let mut tables = Vec::with_capacity(movie_a.tracks.len());
    for (left, right) in movie_a.tracks.iter().zip(movie_b.tracks.iter()) {
        tables.push(merge_track(a, b, left, right, &mut arena)?);
    }
    if arena.len() > (u32::MAX as usize).saturating_sub(8) {
        return Err(Error::Unsupported("mdat larger than 4 GiB"));
    }
    let tracks: Vec<Track<'_>> = movie_a
        .tracks
        .iter()
        .zip(tables)
        .map(|(src, samples)| Track {
            id: src.id,
            handler: src.handler,
            codec: src.codec,
            timescale: src.timescale,
            width: src.width,
            height: src.height,
            channels: src.channels,
            bits: src.bits,
            sample_rate: src.sample_rate,
            stsd: src.stsd,
            stsd_off: src.stsd_off,
            samples,
        })
        .collect();
    let merged = Movie {
        data: &[],
        brand: movie_a.brand,
        ftyp: movie_a.ftyp,
        timescale: movie_a.timescale,
        duration: 0,
        tracks,
        moov_pos: 0,
        mdat_pos: 0,
        fragmented: false,
    };
    let refs: Vec<&Track<'_>> = merged.tracks.iter().collect();
    remux_tracks(&arena, &merged, &refs, true)
}

fn merge_track(
    a: &[u8],
    b: &[u8],
    left: &Track<'_>,
    right: &Track<'_>,
    arena: &mut Vec<u8>,
) -> Result<SampleTable> {
    if left.samples.is_empty() || right.samples.is_empty() {
        return Err(Error::Unsupported("track has no samples"));
    }
    if left.codec != right.codec {
        return Err(Error::Unsupported("codec mismatch"));
    }
    if left.handler != right.handler || left.timescale != right.timescale {
        return Err(Error::Unsupported("track timescale mismatch"));
    }
    if left.stsd != right.stsd {
        return Err(Error::Unsupported("sample entry mismatch"));
    }
    let start = arena.len() as u64;
    append_samples(a, &left.samples, arena)?;
    append_samples(b, &right.samples, arena)?;
    let mut sizes = left.samples.sizes.clone();
    sizes.extend_from_slice(&right.samples.sizes);
    let mut deltas = left.samples.deltas.clone();
    deltas.extend_from_slice(&right.samples.deltas);
    if sizes.len() != deltas.len() {
        return Err(Error::Corrupt("sample table width"));
    }
    let mut offsets = Vec::with_capacity(sizes.len());
    let mut cursor = start;
    for size in &sizes {
        offsets.push(cursor);
        cursor = cursor
            .checked_add(u64::from(*size))
            .ok_or(Error::Corrupt("sample end"))?;
    }
    let count = left.samples.len() as u32;
    let sync = merge_sync(
        &left.samples.sync,
        count,
        &right.samples.sync,
        right.samples.len() as u32,
    );
    Ok(SampleTable {
        sizes,
        offsets,
        deltas,
        sync,
    })
}

fn merge_sync(left: &[u32], left_count: u32, right: &[u32], right_count: u32) -> Vec<u32> {
    if left.is_empty() && right.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    if left.is_empty() {
        for n in 1..=left_count {
            out.push(n);
        }
    } else {
        out.extend_from_slice(left);
    }
    if right.is_empty() {
        for n in 1..=right_count {
            out.push(left_count + n);
        }
    } else {
        for n in right {
            out.push(left_count + n);
        }
    }
    out
}
