//! ISO BMFF boxes. A box declares its size. A size that runs past the buffer
//! is truncated. A size of zero is refused so a short file cannot look finished.

use crate::error::Error;

#[derive(Clone, Debug)]
pub struct Atom {
    pub typ: [u8; 4],
    pub start: usize,
    pub payload: std::ops::Range<usize>,
}

pub fn atoms(data: &[u8], start: usize, end: usize) -> Result<Vec<Atom>, Error> {
    if end > data.len() || start > end {
        return Err(Error::Invalid);
    }
    let mut out = Vec::new();
    let mut off = start;
    while off < end {
        if end - off < 8 {
            return Err(Error::Truncated);
        }
        let size32 = be_u32(&data[off..off + 4]);
        let typ = [data[off + 4], data[off + 5], data[off + 6], data[off + 7]];
        let (header, size) = if size32 == 1 {
            if end - off < 16 {
                return Err(Error::Truncated);
            }
            (16usize, be_u64(&data[off + 8..off + 16]))
        } else if size32 == 0 {
            return Err(Error::Invalid);
        } else {
            (8usize, u64::from(size32))
        };
        if size < header as u64 {
            return Err(Error::Invalid);
        }
        let size_us = usize::try_from(size).map_err(|_| Error::Invalid)?;
        let box_end = off.checked_add(size_us).ok_or(Error::Invalid)?;
        if box_end > end {
            return Err(Error::Truncated);
        }
        out.push(Atom {
            typ,
            start: off,
            payload: off + header..box_end,
        });
        off = box_end;
    }
    Ok(out)
}

pub fn be_u32(bytes: &[u8]) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&bytes[..4]);
    u32::from_be_bytes(buf)
}

pub fn be_u64(bytes: &[u8]) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&bytes[..8]);
    u64::from_be_bytes(buf)
}

pub struct Cur<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Cur<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub fn u32(&mut self) -> Result<u32, Error> {
        self.bytes(4).map(be_u32)
    }

    pub fn i32(&mut self) -> Result<i32, Error> {
        self.u32().map(|v| v as i32)
    }

    pub fn u64(&mut self) -> Result<u64, Error> {
        self.bytes(8).map(be_u64)
    }

    pub fn skip(&mut self, n: usize) -> Result<(), Error> {
        self.bytes(n).map(|_| ())
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::Invalid)?;
        if end > self.data.len() {
            return Err(Error::Invalid);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }
}

pub fn push_box(out: &mut Vec<u8>, typ: [u8; 4], payload: &[u8]) -> Result<(), Error> {
    let size = 8usize.checked_add(payload.len()).ok_or(Error::Invalid)?;
    let size_u = u32::try_from(size).map_err(|_| Error::Invalid)?;
    out.extend_from_slice(&size_u.to_be_bytes());
    out.extend_from_slice(&typ);
    out.extend_from_slice(payload);
    Ok(())
}

pub fn push_full(
    out: &mut Vec<u8>,
    typ: [u8; 4],
    version: u8,
    flags: u32,
    payload: &[u8],
) -> Result<(), Error> {
    if flags > 0x00ff_ffff {
        return Err(Error::Invalid);
    }
    let mut body = Vec::with_capacity(4 + payload.len());
    let head = (u32::from(version) << 24) | flags;
    body.extend_from_slice(&head.to_be_bytes());
    body.extend_from_slice(payload);
    push_box(out, typ, &body)
}

pub fn full_boxed(typ: [u8; 4], version: u8, flags: u32, payload: &[u8]) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    push_full(&mut out, typ, version, flags, payload)?;
    Ok(out)
}
