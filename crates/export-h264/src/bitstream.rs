/// MSB-first bit writer, Exp-Golomb, and Annex B NAL wrapping.
pub struct BitWriter {
    data: Vec<u8>,
    cur: u8,
    nbits: u8,
}

impl BitWriter {
    pub fn new() -> Self {
        Self {
            data: Vec::new(),
            cur: 0,
            nbits: 0,
        }
    }

    pub fn write_bit(&mut self, bit: u32) {
        self.cur = (self.cur << 1) | ((bit & 1) as u8);
        self.nbits += 1;
        if self.nbits == 8 {
            self.data.push(self.cur);
            self.cur = 0;
            self.nbits = 0;
        }
    }

    pub fn write_bits(&mut self, val: u32, n: u32) {
        for i in (0..n).rev() {
            self.write_bit((val >> i) & 1);
        }
    }

    pub fn write_zeros(&mut self, n: u32) {
        for _ in 0..n {
            self.write_bit(0);
        }
    }

    pub fn write_ue(&mut self, v: u32) {
        let x = v.wrapping_add(1);
        let width = 32 - x.leading_zeros();
        if width > 1 {
            self.write_zeros(width - 1);
        }
        self.write_bits(x, width);
    }

    pub fn write_se(&mut self, v: i32) {
        let code = if v <= 0 {
            ((-v) as u32) << 1
        } else {
            ((v as u32) << 1) - 1
        };
        self.write_ue(code);
    }

    /// Stop bit, then zero pad to a byte.
    pub fn rbsp_trailing(&mut self) {
        self.write_bit(1);
        if self.nbits != 0 {
            self.cur <<= 8 - self.nbits;
            self.data.push(self.cur);
            self.cur = 0;
            self.nbits = 0;
        }
    }

    pub fn into_bytes(self) -> Vec<u8> {
        self.data
    }
}

/// Annex B NAL: 4-byte start code, header, RBSP with emulation prevention.
pub fn nal_unit(ref_idc: u8, nal_type: u8, rbsp: &[u8]) -> Vec<u8> {
    let mut out = vec![0, 0, 0, 1, (ref_idc << 5) | (nal_type & 0x1f)];
    let mut zeros = 0u8;
    for &b in rbsp {
        if zeros == 2 && b <= 3 {
            out.push(3);
            zeros = 0;
        }
        out.push(b);
        if b == 0 {
            zeros += 1;
        } else {
            zeros = 0;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::BitWriter;

    #[test]
    fn ue_zero_is_one_bit() {
        let mut w = BitWriter::new();
        w.write_ue(0);
        w.rbsp_trailing();
        let b = w.into_bytes();
        // `1` then stop bit fills the byte: 1100_0000
        assert_eq!(b, vec![0b1100_0000]);
    }

    #[test]
    fn se_zero_matches_ue_zero() {
        let mut w = BitWriter::new();
        w.write_se(0);
        w.rbsp_trailing();
        assert_eq!(w.into_bytes(), vec![0b1100_0000]);
    }
}
