//! FLAC CRC-8 (poly 0x07) and CRC-16 (poly 0x8005). Both init 0, not reflected.

const CRC8_TABLE: [u8; 256] = crc8_table();
const CRC16_TABLE: [u16; 256] = crc16_table();

const fn crc8_table() -> [u8; 256] {
    let mut table = [0u8; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u8;
        let mut bit = 0;
        while bit < 8 {
            if crc & 0x80 != 0 {
                crc = (crc << 1) ^ 0x07;
            } else {
                crc <<= 1;
            }
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

const fn crc16_table() -> [u16; 256] {
    let mut table = [0u16; 256];
    let mut i = 0;
    while i < 256 {
        let mut crc = (i as u16) << 8;
        let mut bit = 0;
        while bit < 8 {
            if crc & 0x8000 != 0 {
                crc = (crc << 1) ^ 0x8005;
            } else {
                crc <<= 1;
            }
            bit += 1;
        }
        table[i] = crc;
        i += 1;
    }
    table
}

pub fn crc8(data: &[u8]) -> u8 {
    let mut crc = 0u8;
    for &b in data {
        crc = CRC8_TABLE[(crc ^ b) as usize];
    }
    crc
}

pub fn crc16(data: &[u8]) -> u16 {
    let mut crc = 0u16;
    for &b in data {
        let idx = ((crc >> 8) ^ u16::from(b)) as u8 as usize;
        crc = (crc << 8) ^ CRC16_TABLE[idx];
    }
    crc
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crc16_bits(data: &[u8]) -> u16 {
        let mut crc = 0u16;
        for &b in data {
            crc ^= u16::from(b) << 8;
            for _ in 0..8 {
                if crc & 0x8000 != 0 {
                    crc = (crc << 1) ^ 0x8005;
                } else {
                    crc <<= 1;
                }
            }
        }
        crc
    }

    #[test]
    fn known_checks() {
        let msg = b"123456789";
        assert_eq!(crc8(msg), 0xF4);
        assert_eq!(crc16(msg), 0xFEE8);
        assert_eq!(crc16(msg), crc16_bits(msg));
        assert_eq!(crc8(b""), 0);
        assert_eq!(crc16(b""), 0);
    }
}
