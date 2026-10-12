use export_gif::{
    decode_gif, encode_gif, read_gif, sniff, video_to_gif, write_gif, Cache, ErrorCode, Format,
    Frame, Gif, Version, THREADING,
};

#[test]
fn threading_stays_off() {
    assert!(!std::hint::black_box(THREADING));
}

#[test]
fn sniff_both_gif_versions_and_video() {
    assert_eq!(sniff(b"GIF87a"), Format::Gif87a);
    assert_eq!(sniff(b"GIF89a\x00"), Format::Gif89a);
    assert_eq!(
        sniff(&[0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm']),
        Format::Video
    );
    let err = decode_gif(&[0, 0, 0, 0x20, b'f', b't', b'y', b'p']).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unsupported);
    assert_eq!(err.message(), "video to gif is unsupported");
}

#[test]
fn classic_one_pixel_gif() {
    let bytes = [
        b'G', b'I', b'F', b'8', b'9', b'a', 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02,
        0x02, 0x44, 0x01, 0x00, 0x3b,
    ];
    let gif = decode_gif(&bytes).unwrap();
    assert_eq!(gif.version, Version::Gif89a);
    assert_eq!(gif.frames[0].indices, vec![0]);
    assert_eq!(gif.frames[0].delay_cs, 0);
    assert_eq!(gif.global_table.unwrap(), vec![[0, 0, 0], [255, 255, 255]]);
}

#[test]
fn multi_frame_round_trip_keeps_pixels_delay_and_local_table() {
    let gif = sample();
    let bytes = encode_gif(&gif).unwrap();
    assert_eq!(sniff(&bytes), Format::Gif89a);
    let back = decode_gif(&bytes).unwrap();
    assert_eq!(back.frames.len(), 3);
    for (got, want) in back.frames.iter().zip(&gif.frames) {
        assert_eq!(got.indices, want.indices);
        assert_eq!(got.delay_cs, want.delay_cs);
        assert_eq!(got.local_table, want.local_table);
        assert_eq!(got.transparent, want.transparent);
        assert_eq!(got.left, want.left);
        assert_eq!(got.width, want.width);
    }
    assert_eq!(back.loop_count, Some(0));
    assert_ne!(
        back.frames[1].local_table.as_ref().unwrap()[1],
        back.global_table.as_ref().unwrap()[1]
    );
}

#[test]
fn clear_code_survives_a_full_table() {
    let mut state = 0x1234_5678u32;
    let mut indices = Vec::with_capacity(80 * 80);
    for _ in 0..80 * 80 {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        indices.push((state >> 24) as u8);
    }
    let table: Vec<[u8; 3]> = (0..256).map(|i| [i as u8, 0, 255 - i as u8]).collect();
    let gif = Gif {
        version: Version::Gif89a,
        width: 80,
        height: 80,
        background: 0,
        aspect: 0,
        global_table: Some(table),
        loop_count: None,
        frames: vec![Frame {
            left: 0,
            top: 0,
            width: 80,
            height: 80,
            delay_cs: 8,
            disposal: 1,
            transparent: None,
            local_table: None,
            indices: indices.clone(),
        }],
    };
    let back = decode_gif(&encode_gif(&gif).unwrap()).unwrap();
    assert_eq!(back.frames[0].indices, indices);
    assert_eq!(back.frames[0].delay_cs, 8);
}

#[test]
fn truncated_stream_errors() {
    let bytes = encode_gif(&sample()).unwrap();
    let err = decode_gif(&bytes[..bytes.len() / 2]).unwrap_err();
    assert_eq!(err.code, ErrorCode::Truncated);
    let mut cut = bytes.clone();
    cut.pop();
    let err = decode_gif(&cut).unwrap_err();
    assert_eq!(err.code, ErrorCode::Truncated);
    let short = b"GIF89a\x01\x00\x01\x00\x00\x00";
    assert_eq!(decode_gif(short).unwrap_err().code, ErrorCode::Truncated);
}

#[test]
fn bad_lzw_code_errors() {
    let bytes = [
        b'G', b'I', b'F', b'8', b'9', b'a', 0x01, 0x00, 0x01, 0x00, 0x80, 0x00, 0x00, 0x00, 0x00,
        0x00, 0xff, 0xff, 0xff, 0x2c, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x02,
        0x01, 0x34, 0x00, 0x3b,
    ];
    let err = decode_gif(&bytes).unwrap_err();
    assert_eq!(err.code, ErrorCode::BadLzw);
}

#[test]
fn video_to_gif_errors_without_a_decode() {
    let mp4 = [
        0u8, 0, 0, 0x18, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'2',
    ];
    let err = video_to_gif(&mp4).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unsupported);
    let gif = encode_gif(&sample()).unwrap();
    let err = video_to_gif(&gif).unwrap_err();
    assert_eq!(err.code, ErrorCode::Unsupported);
    assert_eq!(err.message(), "video to gif is unsupported");
}

#[test]
fn dedup_hashes_once_then_skips_the_codec() {
    let bytes = encode_gif(&sample()).unwrap();
    let mut cache = Cache::new();
    let first = read_gif(&bytes, &mut cache).unwrap();
    assert!(!first.dedup_hit);
    assert_eq!(cache.hash_passes, 1);
    assert_eq!(cache.decodes, 1);
    let second = read_gif(&bytes, &mut cache).unwrap();
    assert!(second.dedup_hit);
    assert_eq!(cache.decodes, 1);
    assert_eq!(cache.hash_passes, 2);
    assert_eq!(second.gif.frames[0].indices, first.gif.frames[0].indices);
    assert_eq!(second.gif.frames[0].delay_cs, 5);

    let mut cache = Cache::new();
    let w1 = write_gif(&sample(), &mut cache).unwrap();
    assert!(!w1.dedup_hit);
    assert_eq!(cache.encodes, 1);
    let w2 = write_gif(&sample(), &mut cache).unwrap();
    assert!(w2.dedup_hit);
    assert_eq!(cache.encodes, 1);
    assert_eq!(w2.bytes, w1.bytes);
}

#[test]
fn interlaced_rows_are_restored() {
    let mut indices = Vec::new();
    for y in 0..8u8 {
        indices.push(y);
    }
    let gif = Gif {
        version: Version::Gif89a,
        width: 1,
        height: 8,
        background: 0,
        aspect: 0,
        global_table: Some((0..16).map(|i| [i * 15, 0, 0]).collect()),
        loop_count: None,
        frames: vec![Frame {
            left: 0,
            top: 0,
            width: 1,
            height: 8,
            delay_cs: 3,
            disposal: 0,
            transparent: None,
            local_table: None,
            indices,
        }],
    };
    let mut bytes = encode_gif(&gif).unwrap();
    let marker = bytes.iter().position(|b| *b == 0x2c).unwrap();
    let packed_at = marker + 9;
    assert_eq!(bytes[packed_at], 0);
    bytes[packed_at] |= 0x40;
    let back = decode_gif(&bytes).unwrap();
    assert_eq!(back.frames[0].indices, vec![0, 4, 2, 5, 1, 6, 3, 7]);
    assert_eq!(back.frames[0].delay_cs, 3);
}

fn sample() -> Gif {
    let global = vec![[0, 0, 0], [255, 0, 0], [0, 255, 0], [0, 0, 255]];
    let local = vec![[255, 255, 255], [10, 20, 30], [40, 50, 60], [70, 80, 90]];
    let mut a = Vec::new();
    let mut b = Vec::new();
    let mut c = Vec::new();
    for y in 0..16u8 {
        for x in 0..24u8 {
            a.push((x + y) % 4);
            b.push((x * 3 + y) % 4);
        }
    }
    for y in 0..8u8 {
        for x in 0..8u8 {
            c.push(if x == y { 0 } else { 2 });
        }
    }
    Gif {
        version: Version::Gif89a,
        width: 24,
        height: 16,
        background: 0,
        aspect: 0,
        global_table: Some(global),
        loop_count: Some(0),
        frames: vec![
            Frame {
                left: 0,
                top: 0,
                width: 24,
                height: 16,
                delay_cs: 5,
                disposal: 0,
                transparent: None,
                local_table: None,
                indices: a,
            },
            Frame {
                left: 0,
                top: 0,
                width: 24,
                height: 16,
                delay_cs: 12,
                disposal: 2,
                transparent: Some(1),
                local_table: Some(local),
                indices: b,
            },
            Frame {
                left: 4,
                top: 2,
                width: 8,
                height: 8,
                delay_cs: 40,
                disposal: 1,
                transparent: None,
                local_table: None,
                indices: c,
            },
        ],
    }
}
