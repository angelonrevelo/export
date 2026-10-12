//! CRC-32/ISO-HDLC (reflected poly `0xEDB88320`).
//!
//! Same parameters as the index file footer: init `0xFFFFFFFF`, final invert.
//! It detects accidental truncation and bit-flips. It is not a MAC; a chosen
//! collision is not expensive. That limit is stated on the format, not here.

/// Running checksum. One-shot [`crc32`] and this stream agree at every chunk size.
#[derive(Clone, Debug)]
pub struct Crc32 {
    state: u32,
}

impl Crc32 {
    pub fn new() -> Self {
        Self { state: 0xFFFF_FFFF }
    }

    pub fn update(&mut self, data: &[u8]) {
        let mut crc = self.state;
        for &b in data {
            let idx = ((crc ^ u32::from(b)) & 0xFF) as usize;
            crc = TABLE[idx] ^ (crc >> 8);
        }
        self.state = crc;
    }

    pub fn finish(self) -> u32 {
        !self.state
    }
}

impl Default for Crc32 {
    fn default() -> Self {
        Self::new()
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc32::new();
    c.update(data);
    c.finish()
}

const fn make_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            if c & 1 != 0 {
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

const TABLE: [u32; 256] = make_table();

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_vectors() {
        // CRC-32/ISO-HDLC of the empty string, and the check string from ITU-T V.42.
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }

    #[test]
    fn stream_matches_oneshot_at_every_chunk() {
        let data: Vec<u8> = (0..2000u32).map(|n| (n * 17) as u8).collect();
        let want = crc32(&data);
        for chunk in [1usize, 2, 3, 7, 64, 1000, 4096] {
            let mut c = Crc32::new();
            for part in data.chunks(chunk) {
                c.update(part);
            }
            assert_eq!(c.finish(), want, "chunk {chunk}");
        }
    }
}
