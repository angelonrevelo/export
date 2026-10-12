/// CRC-32 (ISO-HDLC / ZIP / PNG). Init `0xFFFFFFFF`, poly `0xEDB88320`, final invert.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

/// Running CRC so a stream can be checksummed without a second buffer.
#[derive(Clone, Debug)]
pub struct Crc32 {
    crc: u32,
}

impl Crc32 {
    pub fn new() -> Self {
        Self { crc: 0xFFFF_FFFF }
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut crc = self.crc;
        for &b in data {
            crc ^= u32::from(b);
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        self.crc = crc;
    }

    pub fn finish(self) -> u32 {
        !self.crc
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ietf_check_value() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let mut r = Crc32::new();
        r.update(b"1234");
        r.update(b"56789");
        assert_eq!(r.finish(), 0xCBF4_3926);
    }
}
