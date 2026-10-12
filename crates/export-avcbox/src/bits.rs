use crate::error::Error;

/// MSB-first reader over an RBSP. Running out of bits is a truncated buffer.
pub struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn has(&self, n: u32) -> bool {
        self.pos + n as usize <= self.data.len() * 8
    }

    pub fn bit(&mut self) -> Result<u8, Error> {
        if !self.has(1) {
            return Err(Error::Truncated);
        }
        let b = (self.data[self.pos / 8] >> (7 - (self.pos % 8))) & 1;
        self.pos += 1;
        Ok(b)
    }

    pub fn bits(&mut self, n: u32) -> Result<u32, Error> {
        let mut v = 0u32;
        for _ in 0..n {
            v = (v << 1) | u32::from(self.bit()?);
        }
        Ok(v)
    }

    pub fn peek(&self, n: u32) -> Result<u32, Error> {
        if !self.has(n) {
            return Err(Error::Truncated);
        }
        let mut v = 0u32;
        let mut p = self.pos;
        for _ in 0..n {
            let b = (self.data[p / 8] >> (7 - (p % 8))) & 1;
            v = (v << 1) | u32::from(b);
            p += 1;
        }
        Ok(v)
    }

    pub fn ue(&mut self) -> Result<u32, Error> {
        let mut zeros = 0u32;
        while self.bit()? == 0 {
            zeros += 1;
            if zeros > 31 {
                return Err(Error::Truncated);
            }
        }
        let rest = if zeros == 0 { 0 } else { self.bits(zeros)? };
        let val = ((1u64 << zeros) - 1) + u64::from(rest);
        if val > u64::from(u32::MAX) {
            return Err(Error::Frame);
        }
        Ok(val as u32)
    }

    pub fn se(&mut self) -> Result<i32, Error> {
        let v = self.ue()?;
        if v > i32::MAX as u32 {
            return Err(Error::Frame);
        }
        if v & 1 == 1 {
            Ok(((v + 1) / 2) as i32)
        } else {
            Ok(-((v / 2) as i32))
        }
    }

    /// `rbsp_trailing_bits`: a 1, then zeros to the next byte, and no further bytes.
    pub fn finish(&mut self) -> Result<(), Error> {
        if self.bit()? != 1 {
            return Err(Error::Frame);
        }
        while self.pos % 8 != 0 {
            if self.bit()? != 0 {
                return Err(Error::Frame);
            }
        }
        if self.pos != self.data.len() * 8 {
            return Err(Error::Frame);
        }
        Ok(())
    }
}

/// Drop emulation-prevention bytes. The NAL header is not part of the RBSP.
pub fn rbsp(nal: &[u8]) -> Result<Vec<u8>, Error> {
    if nal.is_empty() {
        return Err(Error::Truncated);
    }
    let mut out = Vec::with_capacity(nal.len().saturating_sub(1));
    let mut zeros = 0u8;
    for &byte in &nal[1..] {
        if zeros == 2 && byte == 0x03 {
            zeros = 0;
            continue;
        }
        if zeros == 2 && byte <= 3 {
            return Err(Error::Frame);
        }
        out.push(byte);
        zeros = if byte == 0 {
            zeros.saturating_add(1)
        } else {
            0
        };
    }
    if out.is_empty() {
        return Err(Error::Truncated);
    }
    Ok(out)
}
