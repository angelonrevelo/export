use export_kernel::{read_box, write_box, BoxError, Section};

use crate::parse::{Cue, Error};

const CUE_SECTION: u16 = 1;

pub fn write_cue_file(cues: &[Cue]) -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&(cues.len() as u32).to_le_bytes());
    for cue in cues {
        body.extend_from_slice(&cue.start_ms.to_le_bytes());
        body.extend_from_slice(&cue.end_ms.to_le_bytes());
        let text = cue.text.as_bytes();
        body.extend_from_slice(&(text.len() as u32).to_le_bytes());
        body.extend_from_slice(text);
    }
    write_box(&[Section {
        id: CUE_SECTION,
        bytes: body,
    }])
}

pub fn read_cue_file(bytes: &[u8]) -> Result<Vec<Cue>, Error> {
    let sections = read_box(bytes).map_err(map_box)?;
    let section = sections
        .into_iter()
        .find(|s| s.id == CUE_SECTION)
        .ok_or(Error::Truncated)?;
    let mut body = section.bytes.as_slice();
    let count = take_u32(&mut body)? as usize;
    let mut cues = Vec::with_capacity(count);
    for _ in 0..count {
        let start_ms = take_u64(&mut body)?;
        let end_ms = take_u64(&mut body)?;
        let len = take_u32(&mut body)? as usize;
        if body.len() < len {
            return Err(Error::Truncated);
        }
        let (text, rest) = body.split_at(len);
        let text = String::from_utf8(text.to_vec()).map_err(|_| Error::BadCue)?;
        body = rest;
        cues.push(Cue {
            start_ms,
            end_ms,
            text,
        });
    }
    if !body.is_empty() {
        return Err(Error::BadCue);
    }
    Ok(cues)
}

fn map_box(err: BoxError) -> Error {
    match err {
        BoxError::Truncated | BoxError::BadCrc | BoxError::BadSection => Error::Truncated,
        BoxError::BadMagic | BoxError::BadVersion => Error::BadCue,
    }
}

fn take_u32(bytes: &mut &[u8]) -> Result<u32, Error> {
    if bytes.len() < 4 {
        return Err(Error::Truncated);
    }
    let n = u32::from_le_bytes(bytes[..4].try_into().unwrap());
    *bytes = &bytes[4..];
    Ok(n)
}

fn take_u64(bytes: &mut &[u8]) -> Result<u64, Error> {
    if bytes.len() < 8 {
        return Err(Error::Truncated);
    }
    let n = u64::from_le_bytes(bytes[..8].try_into().unwrap());
    *bytes = &bytes[8..];
    Ok(n)
}
