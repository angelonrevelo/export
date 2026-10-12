//! CRC-32 (ISO-HDLC). Init `0xFFFFFFFF`, poly `0xEDB88320`, final invert.

const TABLE: [u32; 256] = crc_table();

const fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 == 1 {
                c = 0xEDB8_8320 ^ (c >> 1);
            } else {
                c >>= 1;
            }
            k += 1;
        }
        table[i] = c;
        i += 1;
    }
    table
}

/// Running CRC-32 of the bytes written or read.
#[derive(Clone, Debug)]
pub struct Crc32 {
    state: u32,
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32 {
    pub const fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut c = self.state;
        for &byte in data {
            let idx = ((c ^ u32::from(byte)) & 0xFF) as usize;
            c = TABLE[idx] ^ (c >> 8);
        }
        self.state = c;
    }

    pub const fn finish(self) -> u32 {
        !self.state
    }

    pub fn checksum_of(data: &[u8]) -> u32 {
        let mut crc = Self::new();
        crc.update(data);
        crc.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::Crc32;

    #[test]
    fn iso_hdlc_vector() {
        assert_eq!(Crc32::checksum_of(b"123456789"), 0xCBF4_3926);
        assert_eq!(Crc32::checksum_of(b""), 0);
    }

    #[test]
    fn chunked_matches_one_shot() {
        let data: Vec<u8> = (0..10_000).map(|i| (i * 17) as u8).collect();
        let once = Crc32::checksum_of(&data);
        for chunk in [1usize, 2, 3, 7, 64, 1000, 4096, 9999] {
            let mut crc = Crc32::new();
            for part in data.chunks(chunk) {
                crc.update(part);
            }
            assert_eq!(crc.finish(), once, "chunk {chunk}");
        }
    }
}
