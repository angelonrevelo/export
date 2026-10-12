use crate::bits::BitReader;
use crate::Error;

pub struct BoolDecoder<'a, 'b> {
    br: &'b mut BitReader<'a>,
    value: u32,
    range: u32,
    max_bits: i32,
}

impl<'a, 'b> BoolDecoder<'a, 'b> {
    pub fn new(br: &'b mut BitReader<'a>, size: usize) -> Result<Self, Error> {
        if size < 1 || br.bits_left() < size.saturating_mul(8) {
            return Err(Error::Truncated);
        }
        let mut decoder = Self {
            br,
            value: 0,
            range: 255,
            max_bits: (size as i32) * 8 - 8,
        };
        decoder.value = decoder.br.f(8)?;
        if decoder.read(128)? != 0 {
            return Err(Error::BadHeader);
        }
        Ok(decoder)
    }

    #[inline]
    pub fn read(&mut self, prob: u8) -> Result<u8, Error> {
        let split = 1 + (((self.range - 1) * u32::from(prob)) >> 8);
        let bit = if self.value < split {
            self.range = split;
            0
        } else {
            self.range -= split;
            self.value -= split;
            1
        };
        while self.range < 128 {
            let new_bit = if self.max_bits > 0 {
                let bit = self.br.bit()?;
                self.max_bits -= 1;
                bit
            } else {
                return Err(Error::BadHeader);
            };
            self.range <<= 1;
            self.value = (self.value << 1) + new_bit;
        }
        Ok(bit)
    }

    pub fn literal(&mut self, n: u32) -> Result<u32, Error> {
        let mut value = 0u32;
        for _ in 0..n {
            value = (value << 1) | u32::from(self.read(128)?);
        }
        Ok(value)
    }

    pub fn exit(&mut self) -> Result<(), Error> {
        while self.max_bits > 0 {
            if self.br.f(1)? != 0 {
                return Err(Error::BadHeader);
            }
            self.max_bits -= 1;
        }
        Ok(())
    }

    pub fn tree(&mut self, table: &[i16], mut prob: impl FnMut(usize) -> u8) -> Result<i32, Error> {
        let mut n: i16 = 0;
        loop {
            let node = (n as usize) >> 1;
            let bit = self.read(prob(node))?;
            let index = (n as usize) + usize::from(bit);
            if index >= table.len() {
                return Err(Error::BadHeader);
            }
            n = table[index];
            if n <= 0 {
                return Ok(i32::from(-n));
            }
        }
    }
}
