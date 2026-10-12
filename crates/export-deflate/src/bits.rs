use crate::Error;

/// LSB-first bit packer, matching RFC 1951 byte layout.
pub struct BitWriter {
    bytes: Vec<u8>,
    acc: u32,
    bits: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            acc: 0,
            bits: 0,
        }
    }

    /// Comparison writer already `offset` bits into a byte (0..7).
    pub fn with_bit_offset(offset: u32) -> Self {
        debug_assert!(offset < 8);
        Self {
            bytes: Vec::new(),
            acc: 0,
            bits: offset,
        }
    }

    pub fn pending(&self) -> u32 {
        self.bits
    }

    pub fn bit_len(&self) -> usize {
        self.bytes.len() * 8 + self.bits as usize
    }

    pub fn write_bits(&mut self, value: u32, n: u32) {
        debug_assert!(n > 0 && n <= 16);
        debug_assert!(self.bits < 8);
        let mask = (1u32 << n) - 1;
        self.acc |= (value & mask) << self.bits;
        self.bits += n;
        while self.bits >= 8 {
            self.bytes.push(self.acc as u8);
            self.acc >>= 8;
            self.bits -= 8;
        }
    }

    pub fn align(&mut self) {
        if self.bits > 0 {
            self.bytes.push(self.acc as u8);
            self.acc = 0;
            self.bits = 0;
        }
    }

    pub fn write_bytes(&mut self, data: &[u8]) {
        debug_assert_eq!(self.bits, 0);
        self.bytes.extend_from_slice(data);
    }

    pub fn finish(mut self) -> Vec<u8> {
        if self.bits > 0 {
            self.bytes.push(self.acc as u8);
        }
        self.bytes
    }
}

/// LSB-first bit reader over a borrowed buffer.
pub struct BitReader<'a> {
    data: &'a [u8],
    index: usize,
    acc: u32,
    bits: u32,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            index: 0,
            acc: 0,
            bits: 0,
        }
    }

    fn ensure(&mut self, n: u32) -> Result<(), Error> {
        while self.bits < n {
            if self.index >= self.data.len() {
                return Err(Error::Truncated);
            }
            self.acc |= u32::from(self.data[self.index]) << self.bits;
            self.index += 1;
            self.bits += 8;
        }
        Ok(())
    }

    pub fn pull(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        debug_assert!(n <= 16);
        self.ensure(n)?;
        let value = self.acc & ((1u32 << n) - 1);
        self.acc >>= n;
        self.bits -= n;
        Ok(value)
    }

    pub fn align(&mut self) -> Result<(), Error> {
        let rem = self.bits % 8;
        if rem != 0 {
            let _ = self.pull(rem)?;
        }
        Ok(())
    }

    /// Read `n` bytes starting at the next byte boundary.
    pub fn take_bytes(&mut self, n: usize) -> Result<&'a [u8], Error> {
        self.align()?;
        debug_assert_eq!(self.bits % 8, 0);
        let buffered = (self.bits / 8) as usize;
        let start = self.index - buffered;
        let end = start.checked_add(n).ok_or(Error::BadInput)?;
        if end > self.data.len() {
            return Err(Error::Truncated);
        }
        self.index = end;
        self.acc = 0;
        self.bits = 0;
        Ok(&self.data[start..end])
    }
}
