use crate::Error;

/// MSB-first bit packer. Alignment padding is zero.
#[derive(Debug)]
pub struct BitWriter {
    bytes: Vec<u8>,
    acc: u64,
    nbits: u32,
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            bytes: Vec::new(),
            acc: 0,
            nbits: 0,
        }
    }

    pub fn put(&mut self, value: u32, n: u32) {
        debug_assert!(n <= 32);
        let masked = if n == 0 {
            0
        } else if n == 32 {
            u64::from(value)
        } else {
            u64::from(value) & ((1u64 << n) - 1)
        };
        self.acc = (self.acc << n) | masked;
        self.nbits += n;
        while self.nbits >= 8 {
            self.nbits -= 8;
            self.bytes.push((self.acc >> self.nbits) as u8);
        }
        if self.nbits == 0 {
            self.acc = 0;
        } else {
            self.acc &= (1u64 << self.nbits) - 1;
        }
    }

    pub fn byte_align(&mut self) {
        if self.nbits > 0 {
            let pad = 8 - self.nbits;
            self.put(0, pad);
        }
    }

    pub fn finish(mut self) -> Vec<u8> {
        self.byte_align();
        self.bytes
    }
}

impl Default for BitWriter {
    fn default() -> Self {
        Self::new()
    }
}

/// MSB-first reader over a fixed buffer.
#[derive(Debug)]
pub struct BitReader<'a> {
    data: &'a [u8],
    /// Next bit index.
    index: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, index: 0 }
    }

    pub fn read(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        if n > 32 {
            return Err(Error::Invalid);
        }
        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | self.read_bit()?;
        }
        Ok(value)
    }

    pub fn read_bit(&mut self) -> Result<u32, Error> {
        let byte_index = self.index / 8;
        if byte_index >= self.data.len() {
            return Err(Error::Truncated);
        }
        let shift = 7 - (self.index % 8);
        let bit = u32::from((self.data[byte_index] >> shift) & 1);
        self.index += 1;
        Ok(bit)
    }

    pub fn byte_align(&mut self) -> Result<(), Error> {
        let rem = self.index % 8;
        if rem != 0 {
            let skip = 8 - rem;
            for _ in 0..skip {
                // Padding may sit in the last declared byte. Running off the
                // buffer here means the frame ended mid-alignment.
                let _ = self.read_bit()?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_across_bytes() {
        let mut w = BitWriter::new();
        w.put(0xFFF, 12);
        w.put(1, 1);
        w.put(0, 2);
        w.put(0b1010, 4);
        w.put(0x3, 3);
        let bytes = w.finish();
        let mut r = BitReader::new(&bytes);
        assert_eq!(r.read(12).unwrap(), 0xFFF);
        assert_eq!(r.read(1).unwrap(), 1);
        assert_eq!(r.read(2).unwrap(), 0);
        assert_eq!(r.read(4).unwrap(), 0b1010);
        assert_eq!(r.read(3).unwrap(), 0x3);
    }

    #[test]
    fn reader_truncated() {
        let mut r = BitReader::new(&[0x80]);
        assert_eq!(r.read(8).unwrap(), 0x80);
        assert_eq!(r.read(1).unwrap_err(), Error::Truncated);
    }
}
