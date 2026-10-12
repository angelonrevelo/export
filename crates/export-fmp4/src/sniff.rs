//! Container and codec identity from box types. Sample bytes are not read.

use crate::atom::atoms;
use crate::error::Error;

/// H.264 sample entries. `avcC` is the decoder config; seeing the entry is enough.
const H264: [[u8; 4]; 4] = [*b"avc1", *b"avc2", *b"avc3", *b"avc4"];
/// `mp4a` is the MPEG-4 audio sample entry used for AAC. Refusing the fourcc
/// means AudioSpecificConfig is never parsed.
const AAC: [u8; 4] = *b"mp4a";
const OPUS: [u8; 4] = *b"Opus";
const VP9: [u8; 4] = *b"vp09";

const CONTAINERS: [[u8; 4]; 11] = [
    *b"moov", *b"trak", *b"mdia", *b"minf", *b"stbl", *b"dinf", *b"mvex", *b"moof", *b"traf",
    *b"edts", *b"udta",
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sniff {
    pub brand: [u8; 4],
}

/// Walk boxes, require a fragmented movie, and refuse H.264, AAC, Opus, and VP9.
///
/// A box whose declared size runs past `input` is [`Error::Truncated`].
/// Codec checks look at sample-entry fourccs only.
pub fn sniff(input: &[u8]) -> Result<Sniff, Error> {
    if input.len() < 8 {
        return Err(Error::Truncated);
    }
    let top = atoms(input, 0, input.len())?;
    let Some(first) = top.first() else {
        return Err(Error::Truncated);
    };
    if !is_ascii_type(first.typ) {
        return Err(Error::Invalid);
    }
    let mut brand = None;
    let mut saw_moov = false;
    let mut saw_moof = false;
    walk(input, &top, 0, &mut brand, &mut saw_moov, &mut saw_moof)?;
    let brand = brand.ok_or(Error::Invalid)?;
    if !saw_moov || !saw_moof {
        return Err(Error::Invalid);
    }
    Ok(Sniff { brand })
}

fn walk(
    input: &[u8],
    list: &[crate::atom::Atom],
    depth: usize,
    brand: &mut Option<[u8; 4]>,
    saw_moov: &mut bool,
    saw_moof: &mut bool,
) -> Result<(), Error> {
    if depth > 12 {
        return Err(Error::Invalid);
    }
    for atom in list {
        if atom.typ == *b"ftyp" {
            let payload = &input[atom.payload.clone()];
            if payload.len() < 8 {
                return Err(Error::Invalid);
            }
            let mut major = [0u8; 4];
            major.copy_from_slice(&payload[..4]);
            *brand = Some(major);
        } else if atom.typ == *b"moov" {
            *saw_moov = true;
        } else if atom.typ == *b"moof" {
            *saw_moof = true;
        }
        if is_forbidden(atom.typ) {
            return Err(Error::Unsupported);
        }
        if atom.typ == *b"stsd" {
            sniff_stsd(input, atom)?;
        } else if is_container(atom.typ) {
            let children = atoms(input, atom.payload.start, atom.payload.end)?;
            walk(input, &children, depth + 1, brand, saw_moov, saw_moof)?;
        }
    }
    Ok(())
}

fn sniff_stsd(input: &[u8], atom: &crate::atom::Atom) -> Result<(), Error> {
    let payload = &input[atom.payload.clone()];
    if payload.len() < 8 {
        return Err(Error::Invalid);
    }
    let entries = atoms(input, atom.payload.start + 8, atom.payload.end)?;
    for entry in &entries {
        if is_forbidden(entry.typ) {
            return Err(Error::Unsupported);
        }
    }
    Ok(())
}

pub fn is_forbidden(typ: [u8; 4]) -> bool {
    typ == AAC || typ == OPUS || typ == VP9 || H264.contains(&typ)
}

fn is_container(typ: [u8; 4]) -> bool {
    CONTAINERS.contains(&typ)
}

fn is_ascii_type(typ: [u8; 4]) -> bool {
    typ.iter().all(|byte| (0x20..0x7f).contains(byte))
}
