use export_deflate::{
    compress, compress_with, inflate, inflate_with_limit, repeated_text, Error, Method,
};

fn hex(input: &str) -> Vec<u8> {
    (0..input.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&input[i..i + 2], 16).unwrap())
        .collect()
}

#[test]
fn empty_fixed_and_stored_bytes() {
    assert_eq!(compress_with(b"", Method::Fixed), vec![0x03, 0x00]);
    assert_eq!(
        compress_with(b"", Method::Stored),
        vec![0x01, 0x00, 0x00, 0xff, 0xff]
    );
    assert_eq!(compress(b""), vec![0x03, 0x00]);
}

#[test]
fn stored_bytes_match_the_length_framing() {
    let data: Vec<u8> = (0..=255).collect();
    let mut expected = vec![0x01, 0x00, 0x01, 0xff, 0xfe];
    expected.extend_from_slice(&data);
    assert_eq!(compress_with(&data, Method::Stored), expected);
}

#[test]
fn each_method_roundtrips() {
    let samples: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"a".to_vec(),
        b"hello".to_vec(),
        b"The quick brown fox jumps over the lazy dog. ".repeat(4),
        b"abc".repeat(200),
        (0..=255).collect(),
        vec![0u8; 4096],
        vec![7u8; 70_000],
        repeated_text(48 * 1024),
        {
            let mut state = 0x1234_5678u32;
            let mut out = Vec::with_capacity(8192);
            for _ in 0..8192 {
                state = state.wrapping_mul(1664525).wrapping_add(1013904223);
                out.push((state >> 24) as u8);
            }
            out
        },
    ];
    for sample in samples {
        for method in [Method::Auto, Method::Stored, Method::Fixed, Method::Dynamic] {
            let encoded = compress_with(&sample, method);
            let decoded = inflate(&encoded).unwrap_or_else(|err| {
                panic!("method {method:?} len {} failed: {err}", sample.len())
            });
            assert_eq!(decoded, sample, "method {method:?} len {}", sample.len());
        }
    }
}

#[test]
fn repeated_text_is_smaller_than_stored() {
    let data = repeated_text(64 * 1024);
    let stored = compress_with(&data, Method::Stored);
    let auto = compress(&data);
    let dynamic = compress_with(&data, Method::Dynamic);
    assert!(
        auto.len() * 4 < stored.len(),
        "auto {} stored {}",
        auto.len(),
        stored.len()
    );
    assert!(dynamic.len() * 4 < stored.len());
    assert_eq!(inflate(&auto).unwrap(), data);
}

#[test]
fn first_block_type_follows_the_method() {
    let data = b"hello hello hello";
    assert_eq!(compress_with(data, Method::Stored)[0] & 0b110, 0);
    assert_eq!(compress_with(data, Method::Fixed)[0] & 0b110, 0b010);
    assert_eq!(compress_with(data, Method::Dynamic)[0] & 0b110, 0b100);
}

#[test]
fn zlib_raw_vectors() {
    assert_eq!(inflate(&hex("0300")).unwrap(), b"");
    assert_eq!(inflate(&hex("4b0400")).unwrap(), b"a");
    assert_eq!(inflate(&hex("cb48cdc9c90700")).unwrap(), b"hello");
    let fox = b"The quick brown fox jumps over the lazy dog. ".repeat(4);
    assert_eq!(
        inflate(&hex(
            "0bc94855282ccd4cce56482aca2fcf5348cbaf50c82acd2d2856c82f4b2d5228014ae72456552aa4e4a7eb29840c0ec500"
        ))
        .unwrap(),
        fox
    );
    assert_eq!(
        inflate(&hex("4b4c4a4e1c45a388da0800")).unwrap(),
        b"abc".repeat(200)
    );
    let bytes: Vec<u8> = (0..=255).collect();
    let mut stored = vec![0x01, 0x00, 0x01, 0xff, 0xfe];
    stored.extend_from_slice(&bytes);
    assert_eq!(inflate(&stored).unwrap(), bytes);
}

#[test]
fn truncated_block_and_flipped_bit_error() {
    assert_eq!(inflate(&[0x05, 0x00]), Err(Error::Truncated));
    assert_eq!(inflate(&[0x03]), Err(Error::Truncated));
    assert_eq!(inflate(&[]), Err(Error::Truncated));

    let mut stored = compress_with(b"abcdefghijklmnopqrstuvwxyz", Method::Stored);
    let full = stored.len();
    stored.truncate(full / 2);
    assert_eq!(inflate(&stored), Err(Error::Truncated));

    let mut fixed = compress_with(b"hello", Method::Fixed);
    fixed[0] ^= 0x04;
    assert_eq!(inflate(&fixed), Err(Error::BadInput));

    let mut framed = compress_with(&[0u8; 32], Method::Stored);
    framed[3] ^= 0x01;
    assert_eq!(inflate(&framed), Err(Error::BadInput));
}

fn incompressible(len: usize) -> Vec<u8> {
    let mut state = 0xA5A5_1234u32;
    let mut out = Vec::with_capacity(len);
    for _ in 0..len {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        out.push((state >> 24) as u8);
    }
    out
}

#[test]
fn huffman_that_grows_is_stored() {
    let data = incompressible(8192);
    for method in [Method::Auto, Method::Fixed, Method::Dynamic] {
        let encoded = compress_with(&data, method);
        assert_eq!(encoded[0] & 0b110, 0, "{method:?} stayed Huffman");
        assert_eq!(inflate(&encoded).unwrap(), data);
        let mut truncated = encoded.clone();
        truncated.truncate(encoded.len() / 2);
        assert_eq!(inflate(&truncated), Err(Error::Truncated));
    }
    let stored = compress_with(&data, Method::Stored);
    assert!(compress(&data).len() <= stored.len());
}

#[test]
fn output_limit_is_enforced() {
    let data = vec![0u8; 1000];
    let encoded = compress(&data);
    assert_eq!(inflate_with_limit(&encoded, data.len()).unwrap(), data);
    assert_eq!(
        inflate_with_limit(&encoded, data.len() - 1),
        Err(Error::TooLarge)
    );
}
