//! SHA-1 (FIPS 180-4). Standard OOXML encryption hashes with this, not SHA-256.
//!
//! Addition is wrapping: the spec's mod-2^32 sum is not a debug-build panic.

const H0: [u32; 5] = [
    0x6745_2301,
    0xEFCD_AB89,
    0x98BA_DCFE,
    0x1032_5476,
    0xC3D2_E1F0,
];

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h = H0;
    let mut offset = 0;
    while offset + 64 <= data.len() {
        block(&mut h, &data[offset..offset + 64]);
        offset += 64;
    }
    let mut tail = [0u8; 128];
    let rest = data.len() - offset;
    tail[..rest].copy_from_slice(&data[offset..]);
    tail[rest] = 0x80;
    let bits = (data.len() as u64).wrapping_mul(8);
    let padded = if rest + 1 + 8 <= 64 { 64 } else { 128 };
    tail[padded - 8..padded].copy_from_slice(&bits.to_be_bytes());
    block(&mut h, &tail[..64]);
    if padded == 128 {
        block(&mut h, &tail[64..128]);
    }
    let mut out = [0u8; 20];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

fn block(h: &mut [u32; 5], chunk: &[u8]) {
    let mut w = [0u32; 80];
    for i in 0..16 {
        w[i] = u32::from_be_bytes([
            chunk[i * 4],
            chunk[i * 4 + 1],
            chunk[i * 4 + 2],
            chunk[i * 4 + 3],
        ]);
    }
    for i in 16..80 {
        w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
    }
    let mut a = h[0];
    let mut b = h[1];
    let mut c = h[2];
    let mut d = h[3];
    let mut e = h[4];
    for (i, word) in w.iter().enumerate() {
        let (f, k) = if i < 20 {
            ((b & c) | ((!b) & d), 0x5A82_7999)
        } else if i < 40 {
            (b ^ c ^ d, 0x6ED9_EBA1)
        } else if i < 60 {
            ((b & c) | (b & d) | (c & d), 0x8F1B_BCDC)
        } else {
            (b ^ c ^ d, 0xCA62_C1D6)
        };
        let temp = a
            .rotate_left(5)
            .wrapping_add(f)
            .wrapping_add(e)
            .wrapping_add(k)
            .wrapping_add(*word);
        e = d;
        d = c;
        c = b.rotate_left(30);
        b = a;
        a = temp;
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fips_vectors() {
        assert_eq!(sha1(b""), hex("da39a3ee5e6b4b0d3255bfef95601890afd80709"));
        assert_eq!(
            sha1(b"abc"),
            hex("a9993e364706816aba3e25717850c26c9cd0d89d")
        );
        assert_eq!(
            sha1(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            hex("84983e441c3bd26ebaae4aa1f95129e5e54670f1")
        );
    }

    fn hex(s: &str) -> [u8; 20] {
        let mut out = [0u8; 20];
        for i in 0..20 {
            out[i] = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).unwrap();
        }
        out
    }
}
