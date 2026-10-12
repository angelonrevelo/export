use crate::Error;

pub struct BitReader<'a> {
    data: &'a [u8],
    bit: usize,
}

impl<'a> BitReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, bit: 0 }
    }

    pub fn bits_left(&self) -> usize {
        self.data.len().saturating_mul(8).saturating_sub(self.bit)
    }

    pub fn f(&mut self, n: u32) -> Result<u32, Error> {
        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | self.bit()?;
        }
        Ok(value)
    }

    #[inline]
    pub fn bit(&mut self) -> Result<u32, Error> {
        if self.bit >= self.data.len().saturating_mul(8) {
            return Err(Error::Truncated);
        }
        let byte = self.data[self.bit >> 3];
        let shift = 7 - (self.bit & 7);
        self.bit += 1;
        Ok(u32::from((byte >> shift) & 1))
    }
}
