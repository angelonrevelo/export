use export_h264::{
    decode_iframe, decode_motion, decode_pair, encode, encode_ip, encode_ip_motion,
    encode_ip_split, encode_sequence, encode_sequence_split, fixture_1280x720,
    fixture_1280x720_split, fixture_1280x720_split_eight, fixture_1280x720_split_four,
    fixture_1280x720_split_sixteen, fixture_1280x720_split_thirtytwo, fixture_160x160,
    fixture_160x160_eight, fixture_160x160_predicted, fixture_160x160_shifted,
    fixture_160x160_sixteen, fixture_320x240, fixture_320x240_predicted, fixture_320x240_shifted,
    fixture_320x240_split, fixture_320x240_split_four, fixture_32x32, fixture_32x32_eight,
    fixture_32x32_predicted, fixture_32x32_shifted, fixture_640x480, fixture_640x480_split,
    fixture_640x480_split_four, mean_absolute_error, still, EncodeOptions, Error, Motion, Pass,
    Picture, PredictedFrame, Yuv420, SHIFTED_MV, SPLIT_RIGHT_MV,
};

#[test]
fn refuses_p_b_second_pass_and_threads() {
    let frame = fixture_32x32();
    let predicted = EncodeOptions {
        picture: Picture::Predicted,
        ..EncodeOptions::default()
    };
    assert_eq!(
        encode(&frame, &predicted).unwrap_err(),
        Error::PredictedFrame
    );
    let bipred = EncodeOptions {
        picture: Picture::Bipredicted,
        ..EncodeOptions::default()
    };
    assert_eq!(
        encode(&frame, &bipred).unwrap_err(),
        Error::BipredictedFrame
    );
    let second = EncodeOptions {
        pass: Pass::Second,
        ..EncodeOptions::default()
    };
    assert_eq!(encode(&frame, &second).unwrap_err(), Error::SecondPass);
    let threaded = EncodeOptions {
        threads: 4,
        ..EncodeOptions::default()
    };
    assert_eq!(encode(&frame, &threaded).unwrap_err(), Error::Threading);
    assert_eq!(
        Error::PredictedFrame.to_string(),
        "a P-frame is encoded only after the I-frame"
    );
    assert_eq!(
        encode_ip(&frame, &frame, &bipred).unwrap_err(),
        Error::BipredictedFrame
    );
    assert_eq!(
        encode_ip(&frame, &frame, &second).unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn iframe_bytes_stay_in_front_of_the_pframe() {
    let intra = fixture_32x32();
    let alone = encode(&intra, &EncodeOptions::default()).unwrap();
    assert_eq!(alone.annex_b.len(), 272);
    assert!(alone
        .annex_b
        .windows(7)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x0a, 0xda, 0x25, 0x90]));
    let pair = encode_ip(
        &intra,
        &fixture_32x32_predicted(),
        &EncodeOptions::default(),
    )
    .unwrap();
    assert_eq!(pair.annex_b.len(), 415);
    assert!(pair.annex_b.starts_with(&alone.annex_b));
    let zero = encode_ip_motion(
        &intra,
        &fixture_32x32_predicted(),
        &EncodeOptions::default(),
        Motion { mvx: 0, mvy: 0 },
    )
    .unwrap();
    assert_eq!(zero.annex_b, pair.annex_b);
    assert!(pair
        .annex_b
        .windows(5)
        .any(|w| w[0..4] == [0, 0, 0, 1] && w[4] == 0x01));
}

#[test]
fn flat_gray_reconstructs_exactly() {
    let frame = Yuv420::flat(32, 32, 128, 128, 128).unwrap();
    let encoded = encode(&frame, &EncodeOptions::default()).unwrap();
    assert!(encoded.annex_b.starts_with(&[0, 0, 0, 1]));
    assert!(encoded
        .annex_b
        .windows(5)
        .any(|w| w[0..4] == [0, 0, 0, 1] && w[4] == 0x67));
    assert!(encoded
        .annex_b
        .windows(5)
        .any(|w| w[0..4] == [0, 0, 0, 1] && w[4] == 0x68));
    assert!(encoded
        .annex_b
        .windows(5)
        .any(|w| w[0..4] == [0, 0, 0, 1] && w[4] == 0x65));
    assert_eq!(mean_absolute_error(&frame, &encoded.reconstructed), 0.0);
}

#[test]
fn fixture_is_one_iframe() {
    let frame = fixture_32x32();
    let encoded = encode(&frame, &EncodeOptions::default()).unwrap();
    assert_eq!(encoded.qp, 26);
    assert_eq!(encoded.reconstructed.width, 32);
    assert!(encoded.annex_b.len() > 16);
    let mae = mean_absolute_error(&frame, &encoded.reconstructed);
    assert!(mae < 40.0, "internal mae {mae}");
}

#[test]
fn still_is_the_measured_iframe_reconstruction() {
    let frame = fixture_32x32();
    let opts = EncodeOptions::default();
    let encoded = encode(&frame, &opts).unwrap();
    assert_eq!(encoded.annex_b.len(), 272);
    let pixels = still(&encoded);
    assert_eq!(pixels, encoded.reconstructed.i420());
    assert_eq!(pixels.len(), 32 * 32 + 16 * 16 * 2);
    let mae = mean_absolute_error(&frame, &encoded.reconstructed);
    assert!(
        (mae - 2.4974).abs() < 0.00005,
        "still mae {mae:.4} is not the measured reconstruction"
    );
    assert_eq!(
        encode(
            &frame,
            &EncodeOptions {
                picture: Picture::Bipredicted,
                ..opts
            },
        )
        .unwrap_err(),
        Error::BipredictedFrame
    );
    assert_eq!(
        encode(
            &frame,
            &EncodeOptions {
                pass: Pass::Second,
                ..opts
            },
        )
        .unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn decode_reads_the_272_byte_iframe() {
    let frame = fixture_32x32();
    let opts = EncodeOptions::default();
    let encoded = encode(&frame, &opts).unwrap();
    assert_eq!(encoded.annex_b.len(), 272);
    assert_eq!(still(&encoded).len(), 1536);
    let decoded = decode_iframe(&encoded.annex_b).unwrap();
    assert_eq!(decoded, encoded.reconstructed);
    let mae = mean_absolute_error(&frame, &decoded);
    assert!(
        (mae - 2.4974).abs() < 0.00005,
        "decode mae {mae:.4} is not the measured reconstruction"
    );
    let mut changed = false;
    for i in 16..encoded.annex_b.len() {
        let mut flipped = encoded.annex_b.clone();
        flipped[i] ^= 0x1f;
        match decode_iframe(&flipped) {
            Ok(y) if y == encoded.reconstructed => {}
            _ => {
                changed = true;
                break;
            }
        }
    }
    assert!(changed, "decode ignored the Annex B bytes");
    let pair = encode_ip(&frame, &fixture_32x32_predicted(), &opts).unwrap();
    assert_eq!(decode_iframe(&pair.annex_b).unwrap_err(), Error::Bytes);
}

#[test]
fn decode_reads_the_415_byte_pair() {
    let intra = fixture_32x32();
    let predicted = fixture_32x32_predicted();
    let opts = EncodeOptions::default();
    let alone = encode(&intra, &opts).unwrap();
    assert_eq!(alone.annex_b.len(), 272);
    let pair = encode_ip(&intra, &predicted, &opts).unwrap();
    assert_eq!(pair.annex_b.len(), 415);
    assert!(pair.annex_b.starts_with(&alone.annex_b));
    let (dec_i, dec_p) = decode_pair(&pair.annex_b).unwrap();
    assert_eq!(dec_i, pair.intra);
    assert_eq!(dec_p, pair.predicted);
    assert_eq!(decode_iframe(&pair.annex_b).unwrap_err(), Error::Bytes);
    let mut changed = false;
    for i in alone.annex_b.len()..pair.annex_b.len() {
        let mut flipped = pair.annex_b.clone();
        flipped[i] ^= 0x1f;
        match decode_pair(&flipped) {
            Ok((y, p)) if y == pair.intra && p == pair.predicted => {}
            _ => {
                changed = true;
                break;
            }
        }
    }
    assert!(changed, "decode ignored the P-frame bytes");
    let shifted = encode_ip_motion(&intra, &fixture_32x32_shifted(), &opts, SHIFTED_MV).unwrap();
    assert_eq!(decode_pair(&shifted.annex_b).unwrap_err(), Error::Bytes);
    assert_eq!(
        encode_ip(
            &intra,
            &predicted,
            &EncodeOptions {
                picture: Picture::Bipredicted,
                ..opts
            },
        )
        .unwrap_err(),
        Error::BipredictedFrame
    );
    assert_eq!(
        encode_ip(
            &intra,
            &predicted,
            &EncodeOptions {
                pass: Pass::Second,
                ..opts
            },
        )
        .unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn decode_reads_the_shifted_pair() {
    let intra = fixture_32x32();
    let shifted = fixture_32x32_shifted();
    let opts = EncodeOptions::default();
    let alone = encode(&intra, &opts).unwrap();
    assert_eq!(alone.annex_b.len(), 272);
    let pair = encode_ip_motion(&intra, &shifted, &opts, SHIFTED_MV).unwrap();
    assert_eq!(pair.annex_b.len(), 304);
    assert!(pair.annex_b.starts_with(&alone.annex_b));
    assert_eq!(pair.annex_b[0..4], [0, 0, 0, 1]);
    let (dec_i, dec_p) = decode_motion(&pair.annex_b).unwrap();
    assert_eq!(dec_i, pair.intra);
    assert_eq!(dec_p, pair.predicted);
    assert_eq!(decode_pair(&pair.annex_b).unwrap_err(), Error::Bytes);
    let mut changed = false;
    for i in alone.annex_b.len()..pair.annex_b.len() {
        let mut flipped = pair.annex_b.clone();
        flipped[i] ^= 0x1f;
        match decode_motion(&flipped) {
            Ok((y, p)) if y == pair.intra && p == pair.predicted => {}
            _ => {
                changed = true;
                break;
            }
        }
    }
    assert!(changed, "decode ignored the shifted P-frame bytes");
    assert_eq!(
        encode_ip_motion(
            &intra,
            &shifted,
            &EncodeOptions {
                picture: Picture::Bipredicted,
                ..opts
            },
            SHIFTED_MV,
        )
        .unwrap_err(),
        Error::BipredictedFrame
    );
    assert_eq!(
        encode_ip_motion(
            &intra,
            &shifted,
            &EncodeOptions {
                pass: Pass::Second,
                ..opts
            },
            SHIFTED_MV,
        )
        .unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn shifted_picture_uses_one_motion_vector() {
    let intra = fixture_32x32();
    let shifted = fixture_32x32_shifted();
    let opts = EncodeOptions::default();
    let alone = encode(&intra, &opts).unwrap();
    let moved = encode_ip_motion(&intra, &shifted, &opts, SHIFTED_MV).unwrap();
    assert!(moved.annex_b.starts_with(&alone.annex_b));
    assert_eq!(&moved.annex_b[..272], &alone.annex_b[..]);
    assert_eq!(moved.annex_b.len(), 304);
    assert_ne!(
        moved.annex_b,
        encode_ip(&intra, &shifted, &opts).unwrap().annex_b
    );
    let mae = mean_absolute_error(&shifted, &moved.predicted);
    assert!(mae < 8.0, "shifted mae {mae}");
    let bipred = EncodeOptions {
        picture: Picture::Bipredicted,
        ..opts
    };
    assert_eq!(
        encode_ip_motion(&intra, &shifted, &bipred, SHIFTED_MV).unwrap_err(),
        Error::BipredictedFrame
    );
    let second = EncodeOptions {
        pass: Pass::Second,
        ..opts
    };
    assert_eq!(
        encode_ip_motion(&intra, &shifted, &second, SHIFTED_MV).unwrap_err(),
        Error::SecondPass
    );
    assert_eq!(
        encode_ip_motion(&intra, &shifted, &opts, Motion { mvx: 4, mvy: 0 }).unwrap_err(),
        Error::Motion
    );
    assert_eq!(
        encode(
            &shifted,
            &EncodeOptions {
                picture: Picture::Predicted,
                ..opts
            }
        )
        .unwrap_err(),
        Error::PredictedFrame
    );
}

#[test]
fn eight_pictures_keep_the_iframe_prefix() {
    let intra = fixture_32x32();
    let opts = EncodeOptions::default();
    let alone = encode(&intra, &opts).unwrap();
    let (seq_intra, predicted) = fixture_32x32_eight();
    assert_eq!(seq_intra, intra);
    assert_eq!(predicted.len(), 7);
    let sequence = encode_sequence(&intra, &predicted, &opts).unwrap();
    assert_eq!(sequence.annex_b.len(), 405);
    assert_eq!(&sequence.annex_b[..272], &alone.annex_b[..]);
    assert_eq!(sequence.reconstructed.len(), 8);
    let one = encode_sequence(
        &intra,
        &[PredictedFrame {
            picture: fixture_32x32_predicted(),
            motion: Motion { mvx: 0, mvy: 0 },
        }],
        &opts,
    )
    .unwrap();
    let pair = encode_ip(&intra, &fixture_32x32_predicted(), &opts).unwrap();
    assert_eq!(one.annex_b, pair.annex_b);
    let shifted = encode_sequence(
        &intra,
        &[PredictedFrame {
            picture: fixture_32x32_shifted(),
            motion: SHIFTED_MV,
        }],
        &opts,
    )
    .unwrap();
    assert_eq!(
        shifted.annex_b,
        encode_ip_motion(&intra, &fixture_32x32_shifted(), &opts, SHIFTED_MV)
            .unwrap()
            .annex_b
    );
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(&src.picture, rec);
    }
    let mae = sum / 7.0;
    assert!(mae < 8.0, "sequence mae {mae}");
    assert_eq!(
        encode_sequence(&intra, &[], &opts).unwrap_err(),
        Error::Count
    );
    let too_many = vec![
        PredictedFrame {
            picture: intra.clone(),
            motion: Motion { mvx: 0, mvy: 0 },
        };
        16
    ];
    assert_eq!(
        encode_sequence(&intra, &too_many, &opts).unwrap_err(),
        Error::Count
    );
    let bipred = EncodeOptions {
        picture: Picture::Bipredicted,
        ..opts
    };
    assert_eq!(
        encode_sequence(&intra, &predicted, &bipred).unwrap_err(),
        Error::BipredictedFrame
    );
    let second = EncodeOptions {
        pass: Pass::Second,
        ..opts
    };
    assert_eq!(
        encode_sequence(&intra, &predicted, &second).unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn larger_iframe_is_constrained_baseline() {
    let frame = fixture_160x160();
    let opts = EncodeOptions::default();
    let encoded = encode(&frame, &opts).unwrap();
    assert_eq!(encoded.qp, 26);
    assert_eq!(encoded.reconstructed.width, 160);
    assert_eq!(encoded.annex_b.len(), 6432);
    assert!(encoded
        .annex_b
        .windows(4)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x0b]));
    let mae = mean_absolute_error(&frame, &encoded.reconstructed);
    assert!(mae < 40.0, "internal mae {mae}");
    assert_eq!(
        encode(
            &frame,
            &EncodeOptions {
                picture: Picture::Predicted,
                ..opts
            }
        )
        .unwrap_err(),
        Error::PredictedFrame
    );
    assert!(Yuv420::new(
        1296,
        720,
        vec![0; 1296 * 720],
        vec![0; 648 * 360],
        vec![0; 648 * 360]
    )
    .is_err());
    let predicted = fixture_160x160_predicted();
    let pair = encode_ip(&frame, &predicted, &opts).unwrap();
    assert_eq!(pair.annex_b.len(), 9689);
    assert!(pair.annex_b.starts_with(&encoded.annex_b));
    assert_eq!(&pair.annex_b[..6432], &encoded.annex_b[..]);
    let zero = encode_ip_motion(&frame, &predicted, &opts, Motion { mvx: 0, mvy: 0 }).unwrap();
    assert_eq!(zero.annex_b, pair.annex_b);
    assert_eq!(
        encode_ip(
            &frame,
            &predicted,
            &EncodeOptions {
                picture: Picture::Bipredicted,
                ..opts
            }
        )
        .unwrap_err(),
        Error::BipredictedFrame
    );
}

#[test]
fn larger_shifted_picture_uses_one_motion_vector() {
    let intra = fixture_160x160();
    let shifted = fixture_160x160_shifted();
    let opts = EncodeOptions::default();
    let alone = encode(&intra, &opts).unwrap();
    assert_eq!(alone.annex_b.len(), 6432);
    let zero = encode_ip(&intra, &fixture_160x160_predicted(), &opts).unwrap();
    assert_eq!(zero.annex_b.len(), 9689);
    let moved = encode_ip_motion(&intra, &shifted, &opts, SHIFTED_MV).unwrap();
    assert_eq!(moved.annex_b.len(), 6694);
    assert_eq!(&moved.annex_b[..6432], &alone.annex_b[..]);
    assert_ne!(
        moved.annex_b,
        encode_ip(&intra, &shifted, &opts).unwrap().annex_b
    );
    let mae = mean_absolute_error(&shifted, &moved.predicted);
    assert!(mae < 8.0, "shifted mae {mae}");
    assert_eq!(
        encode_ip_motion(
            &intra,
            &shifted,
            &EncodeOptions {
                picture: Picture::Bipredicted,
                ..opts
            },
            SHIFTED_MV,
        )
        .unwrap_err(),
        Error::BipredictedFrame
    );
    assert_eq!(
        encode_ip_motion(
            &intra,
            &shifted,
            &EncodeOptions {
                pass: Pass::Second,
                ..opts
            },
            SHIFTED_MV,
        )
        .unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn larger_sequence_keeps_the_iframe_prefix() {
    let opts = EncodeOptions::default();
    let (intra, predicted) = fixture_160x160_eight();
    assert_eq!(predicted.len(), 7);
    let alone = encode(&intra, &opts).unwrap();
    assert_eq!(alone.annex_b.len(), 6432);
    let zero = encode_ip(&intra, &fixture_160x160_predicted(), &opts).unwrap();
    assert_eq!(zero.annex_b.len(), 9689);
    let moved = encode_ip_motion(&intra, &fixture_160x160_shifted(), &opts, SHIFTED_MV).unwrap();
    assert_eq!(moved.annex_b.len(), 6694);
    let sequence = encode_sequence(&intra, &predicted, &opts).unwrap();
    assert_eq!(sequence.annex_b.len(), 7197);
    assert_eq!(&sequence.annex_b[..6432], &alone.annex_b[..]);
    assert_eq!(sequence.reconstructed.len(), 8);
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(&src.picture, rec);
    }
    let mae = sum / 7.0;
    assert!(mae < 8.0, "sequence mae {mae}");
    assert_eq!(
        encode_sequence(
            &intra,
            &predicted,
            &EncodeOptions {
                picture: Picture::Bipredicted,
                ..opts
            }
        )
        .unwrap_err(),
        Error::BipredictedFrame
    );
    assert_eq!(
        encode_sequence(
            &intra,
            &predicted,
            &EncodeOptions {
                pass: Pass::Second,
                ..opts
            }
        )
        .unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn sixteen_pictures_keep_the_iframe() {
    let opts = EncodeOptions::default();
    let (intra, predicted) = fixture_160x160_sixteen();
    assert_eq!(predicted.len(), 15);
    let alone = encode(&intra, &opts).unwrap();
    assert_eq!(alone.annex_b.len(), 6432);
    let eight = encode_sequence(&intra, &fixture_160x160_eight().1, &opts).unwrap();
    assert_eq!(eight.annex_b.len(), 7197);
    let sequence = encode_sequence(&intra, &predicted, &opts).unwrap();
    assert_eq!(sequence.annex_b.len(), 7814);
    assert_eq!(sequence.reconstructed.len(), 16);
    assert_eq!(&sequence.annex_b[..6432], &alone.annex_b[..]);
    assert_ne!(&sequence.annex_b[..7197], &eight.annex_b[..]);
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(&src.picture, rec);
    }
    let mae = sum / 15.0;
    assert!(mae < 8.0, "sixteen mae {mae}");
    let too_many = vec![
        PredictedFrame {
            picture: intra.clone(),
            motion: Motion { mvx: 0, mvy: 0 },
        };
        16
    ];
    assert_eq!(
        encode_sequence(&intra, &too_many, &opts).unwrap_err(),
        Error::Count
    );
}

#[test]
fn qvga_iframe_keeps_the_sixteen_picture_file() {
    let opts = EncodeOptions::default();
    let (intra, predicted) = fixture_160x160_sixteen();
    let sixteen = encode_sequence(&intra, &predicted, &opts).unwrap();
    assert_eq!(sixteen.annex_b.len(), 7814);
    let frame = fixture_320x240();
    let encoded = encode(&frame, &opts).unwrap();
    assert_eq!(encoded.annex_b.len(), 19337);
    assert_eq!(encoded.qp, 26);
    assert_eq!(encoded.reconstructed.width, 320);
    assert_eq!(encoded.reconstructed.height, 240);
    assert!(encoded
        .annex_b
        .windows(4)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x0b]));
    assert_eq!(
        encode(
            &frame,
            &EncodeOptions {
                picture: Picture::Predicted,
                ..opts
            }
        )
        .unwrap_err(),
        Error::PredictedFrame
    );
    let brighter = fixture_320x240_predicted();
    let pair = encode_ip(&frame, &brighter, &opts).unwrap();
    assert_eq!(pair.annex_b.len(), 29090);
    assert!(pair.annex_b.starts_with(&encoded.annex_b));
    assert_eq!(&pair.annex_b[..19337], &encoded.annex_b[..]);
    let zero = encode_ip_motion(&frame, &brighter, &opts, Motion { mvx: 0, mvy: 0 }).unwrap();
    assert_eq!(zero.annex_b, pair.annex_b);
    let shifted = fixture_320x240_shifted();
    let moved = encode_ip_motion(&frame, &shifted, &opts, SHIFTED_MV).unwrap();
    assert_eq!(moved.annex_b.len(), 20024);
    assert_eq!(&moved.annex_b[..19337], &encoded.annex_b[..]);
    assert_ne!(moved.annex_b, pair.annex_b);
    let mae = mean_absolute_error(&shifted, &moved.predicted);
    assert!(mae < 8.0, "shifted mae {mae}");
}

#[test]
fn qvga_split_keeps_the_shifted_pair() {
    let opts = EncodeOptions::default();
    let frame = fixture_320x240();
    let encoded = encode(&frame, &opts).unwrap();
    assert_eq!(encoded.annex_b.len(), 19337);
    let shifted = fixture_320x240_shifted();
    let moved = encode_ip_motion(&frame, &shifted, &opts, SHIFTED_MV).unwrap();
    assert_eq!(moved.annex_b.len(), 20024);
    let uniform = encode_ip_split(&frame, &shifted, &opts, SHIFTED_MV, SHIFTED_MV).unwrap();
    assert_eq!(uniform.annex_b, moved.annex_b);
    let brighter = fixture_320x240_predicted();
    let zero = encode_ip(&frame, &brighter, &opts).unwrap();
    assert_eq!(zero.annex_b.len(), 29090);
    let zero_split = encode_ip_split(
        &frame,
        &brighter,
        &opts,
        Motion { mvx: 0, mvy: 0 },
        Motion { mvx: 0, mvy: 0 },
    )
    .unwrap();
    assert_eq!(zero_split.annex_b, zero.annex_b);
    let (intra, predicted) = fixture_160x160_sixteen();
    assert_eq!(
        encode_sequence(&intra, &predicted, &opts)
            .unwrap()
            .annex_b
            .len(),
        7814
    );
    let picture = fixture_320x240_split();
    let pair = encode_ip_split(&frame, &picture, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(pair.annex_b.len(), 19954);
    assert_eq!(&pair.annex_b[..19337], &encoded.annex_b[..]);
    assert_ne!(pair.annex_b, moved.annex_b);
    assert_ne!(pair.annex_b, zero.annex_b);
    let mae = mean_absolute_error(&picture, &pair.predicted);
    assert!(mae < 8.0, "split mae {mae}");
    assert_eq!(
        encode_ip_split(
            &frame,
            &picture,
            &EncodeOptions {
                picture: Picture::Bipredicted,
                ..opts
            },
            SHIFTED_MV,
            SPLIT_RIGHT_MV,
        )
        .unwrap_err(),
        Error::BipredictedFrame
    );
    assert_eq!(
        encode_ip_split(
            &frame,
            &picture,
            &EncodeOptions {
                pass: Pass::Second,
                ..opts
            },
            SHIFTED_MV,
            SPLIT_RIGHT_MV,
        )
        .unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn qvga_split_sequence_keeps_the_pair() {
    let opts = EncodeOptions::default();
    let frame = fixture_320x240();
    let encoded = encode(&frame, &opts).unwrap();
    assert_eq!(encoded.annex_b.len(), 19337);
    let shifted = encode_ip_motion(&frame, &fixture_320x240_shifted(), &opts, SHIFTED_MV).unwrap();
    assert_eq!(shifted.annex_b.len(), 20024);
    let picture = fixture_320x240_split();
    let pair = encode_ip_split(&frame, &picture, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(pair.annex_b.len(), 19954);
    let one = encode_sequence_split(
        &frame,
        &[picture.clone()],
        &opts,
        SHIFTED_MV,
        SPLIT_RIGHT_MV,
    )
    .unwrap();
    assert_eq!(one.annex_b, pair.annex_b);
    let (intra, predicted) = fixture_320x240_split_four();
    assert_eq!(intra, frame);
    assert_eq!(predicted.len(), 3);
    assert_eq!(predicted[0], picture);
    let sequence =
        encode_sequence_split(&intra, &predicted, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(sequence.annex_b.len(), 20414);
    assert_eq!(sequence.reconstructed.len(), 4);
    assert_eq!(&sequence.annex_b[..19337], &encoded.annex_b[..]);
    assert_ne!(&sequence.annex_b[..19954], &pair.annex_b[..]);
    let (sixteen_intra, sixteen) = fixture_160x160_sixteen();
    assert_eq!(
        encode_sequence(&sixteen_intra, &sixteen, &opts)
            .unwrap()
            .annex_b
            .len(),
        7814
    );
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(src, rec);
    }
    let mae = sum / 3.0;
    assert!(mae < 8.0, "split sequence mae {mae}");
    assert_eq!(
        encode_sequence_split(&frame, &[], &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap_err(),
        Error::Count
    );
    assert_eq!(
        encode_sequence_split(
            &frame,
            &predicted,
            &EncodeOptions {
                picture: Picture::Bipredicted,
                ..opts
            },
            SHIFTED_MV,
            SPLIT_RIGHT_MV,
        )
        .unwrap_err(),
        Error::BipredictedFrame
    );
    assert_eq!(
        encode_sequence_split(
            &frame,
            &predicted,
            &EncodeOptions {
                pass: Pass::Second,
                ..opts
            },
            SHIFTED_MV,
            SPLIT_RIGHT_MV,
        )
        .unwrap_err(),
        Error::SecondPass
    );
}

#[test]
fn vga_split_keeps_the_qvga_rows() {
    let opts = EncodeOptions::default();
    let qvga = fixture_320x240();
    let qvga_i = encode(&qvga, &opts).unwrap();
    assert_eq!(qvga_i.annex_b.len(), 19337);
    assert!(qvga_i
        .annex_b
        .windows(4)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x0b]));
    let pair = encode_ip_split(
        &qvga,
        &fixture_320x240_split(),
        &opts,
        SHIFTED_MV,
        SPLIT_RIGHT_MV,
    )
    .unwrap();
    assert_eq!(pair.annex_b.len(), 19954);
    let (intra, predicted) = fixture_320x240_split_four();
    assert_eq!(
        encode_sequence_split(&intra, &predicted, &opts, SHIFTED_MV, SPLIT_RIGHT_MV)
            .unwrap()
            .annex_b
            .len(),
        20414
    );
    let frame = fixture_640x480();
    let encoded = encode(&frame, &opts).unwrap();
    assert_eq!(encoded.reconstructed.width, 640);
    assert_eq!(encoded.reconstructed.height, 480);
    assert!(encoded
        .annex_b
        .windows(4)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x1e]));
    let picture = fixture_640x480_split();
    let vga = encode_ip_split(&frame, &picture, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(vga.annex_b.len(), 80042);
    assert_eq!(&vga.annex_b[..encoded.annex_b.len()], &encoded.annex_b[..]);
    let mae = (mean_absolute_error(&frame, &vga.intra)
        + mean_absolute_error(&picture, &vga.predicted))
        / 2.0;
    assert!(mae < 3.4, "vga mae {mae}");
    assert_eq!(
        encode(
            &frame,
            &EncodeOptions {
                picture: Picture::Predicted,
                ..opts
            }
        )
        .unwrap_err(),
        Error::PredictedFrame
    );
}

#[test]
fn vga_split_sequence_keeps_the_pair() {
    let opts = EncodeOptions::default();
    let frame = fixture_640x480();
    let encoded = encode(&frame, &opts).unwrap();
    let picture = fixture_640x480_split();
    let pair = encode_ip_split(&frame, &picture, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(pair.annex_b.len(), 80042);
    let one = encode_sequence_split(
        &frame,
        std::slice::from_ref(&picture),
        &opts,
        SHIFTED_MV,
        SPLIT_RIGHT_MV,
    )
    .unwrap();
    assert_eq!(one.annex_b, pair.annex_b);
    let (intra, predicted) = fixture_640x480_split_four();
    assert_eq!(intra, frame);
    assert_eq!(predicted.len(), 3);
    assert_eq!(predicted[0], picture);
    let sequence =
        encode_sequence_split(&intra, &predicted, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(sequence.annex_b.len(), 81780);
    assert_eq!(sequence.reconstructed.len(), 4);
    assert_eq!(
        &sequence.annex_b[..encoded.annex_b.len()],
        &encoded.annex_b[..]
    );
    assert_ne!(&sequence.annex_b[..80042], &pair.annex_b[..]);
    let (qvga, qvga_p) = fixture_320x240_split_four();
    assert_eq!(
        encode_sequence_split(&qvga, &qvga_p, &opts, SHIFTED_MV, SPLIT_RIGHT_MV)
            .unwrap()
            .annex_b
            .len(),
        20414
    );
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(src, rec);
    }
    let mae = sum / 3.0;
    let i_mae = mean_absolute_error(&frame, &sequence.reconstructed[0]);
    let all = (i_mae + sum) / 4.0;
    assert!(all < 3.5, "vga sequence mae {all} p {mae}");
}

#[test]
fn hd_split_keeps_the_vga_rows() {
    let opts = EncodeOptions::default();
    let vga = fixture_640x480();
    let vga_pair = encode_ip_split(
        &vga,
        &fixture_640x480_split(),
        &opts,
        SHIFTED_MV,
        SPLIT_RIGHT_MV,
    )
    .unwrap();
    assert_eq!(vga_pair.annex_b.len(), 80042);
    let (vga_i, vga_p) = fixture_640x480_split_four();
    assert_eq!(
        encode_sequence_split(&vga_i, &vga_p, &opts, SHIFTED_MV, SPLIT_RIGHT_MV)
            .unwrap()
            .annex_b
            .len(),
        81780
    );
    let frame = fixture_1280x720();
    let encoded = encode(&frame, &opts).unwrap();
    assert_eq!(encoded.reconstructed.width, 1280);
    assert_eq!(encoded.reconstructed.height, 720);
    assert!(encoded
        .annex_b
        .windows(4)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x1f]));
    let picture = fixture_1280x720_split();
    let pair = encode_ip_split(&frame, &picture, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(pair.annex_b.len(), 239967);
    assert_eq!(&pair.annex_b[..encoded.annex_b.len()], &encoded.annex_b[..]);
    let mae = (mean_absolute_error(&frame, &pair.intra)
        + mean_absolute_error(&picture, &pair.predicted))
        / 2.0;
    assert!(mae < 3.5, "hd mae {mae}");
    assert_eq!(
        encode(
            &frame,
            &EncodeOptions {
                picture: Picture::Predicted,
                ..opts
            }
        )
        .unwrap_err(),
        Error::PredictedFrame
    );
}

#[test]
fn hd_split_sequence_keeps_the_pair() {
    let opts = EncodeOptions::default();
    let frame = fixture_1280x720();
    let encoded = encode(&frame, &opts).unwrap();
    let picture = fixture_1280x720_split();
    let pair = encode_ip_split(&frame, &picture, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(pair.annex_b.len(), 239967);
    let one = encode_sequence_split(
        &frame,
        std::slice::from_ref(&picture),
        &opts,
        SHIFTED_MV,
        SPLIT_RIGHT_MV,
    )
    .unwrap();
    assert_eq!(one.annex_b, pair.annex_b);
    let (intra, predicted) = fixture_1280x720_split_four();
    assert_eq!(intra, frame);
    assert_eq!(predicted.len(), 3);
    assert_eq!(predicted[0], picture);
    let sequence =
        encode_sequence_split(&intra, &predicted, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(sequence.annex_b.len(), 245120);
    assert_eq!(sequence.reconstructed.len(), 4);
    assert_eq!(
        &sequence.annex_b[..encoded.annex_b.len()],
        &encoded.annex_b[..]
    );
    assert_ne!(&sequence.annex_b[..239967], &pair.annex_b[..]);
    let (vga, vga_p) = fixture_640x480_split_four();
    assert_eq!(
        encode_sequence_split(&vga, &vga_p, &opts, SHIFTED_MV, SPLIT_RIGHT_MV)
            .unwrap()
            .annex_b
            .len(),
        81780
    );
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(src, rec);
    }
    let mae = sum / 3.0;
    let i_mae = mean_absolute_error(&frame, &sequence.reconstructed[0]);
    let all = (i_mae + sum) / 4.0;
    assert!(all < 3.5, "hd sequence mae {all} p {mae}");
}

#[test]
fn hd_eight_keeps_the_four_picture_row() {
    let opts = EncodeOptions::default();
    let (four_intra, four_p) = fixture_1280x720_split_four();
    let four =
        encode_sequence_split(&four_intra, &four_p, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(four.annex_b.len(), 245120);
    let (intra, predicted) = fixture_1280x720_split_eight();
    assert_eq!(intra, four_intra);
    assert_eq!(predicted.len(), 7);
    assert_eq!(&predicted[..3], four_p.as_slice());
    let sequence =
        encode_sequence_split(&intra, &predicted, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(sequence.reconstructed.len(), 8);
    assert_eq!(sequence.annex_b.len(), 254390);
    assert_ne!(&sequence.annex_b[..245120], &four.annex_b[..]);
    let iframe = encode(&intra, &opts).unwrap();
    assert_eq!(
        &sequence.annex_b[..iframe.annex_b.len()],
        &iframe.annex_b[..]
    );
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(src, rec);
    }
    let i_mae = mean_absolute_error(&intra, &sequence.reconstructed[0]);
    let all = (i_mae + sum) / 8.0;
    assert!(all <= 3.30, "hd eight mae {all}");
}

#[test]
fn hd_sixteen_keeps_the_eight_picture_row() {
    let opts = EncodeOptions::default();
    let (eight_intra, eight_p) = fixture_1280x720_split_eight();
    let eight =
        encode_sequence_split(&eight_intra, &eight_p, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(eight.annex_b.len(), 254390);
    let (intra, predicted) = fixture_1280x720_split_sixteen();
    assert_eq!(intra, eight_intra);
    assert_eq!(predicted.len(), 15);
    assert_eq!(&predicted[..7], eight_p.as_slice());
    let sequence =
        encode_sequence_split(&intra, &predicted, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(sequence.reconstructed.len(), 16);
    assert_eq!(sequence.annex_b.len(), 272518);
    assert_ne!(&sequence.annex_b[..254390], &eight.annex_b[..]);
    let iframe = encode(&intra, &opts).unwrap();
    assert_eq!(
        &sequence.annex_b[..iframe.annex_b.len()],
        &iframe.annex_b[..]
    );
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(src, rec);
    }
    let i_mae = mean_absolute_error(&intra, &sequence.reconstructed[0]);
    let all = (i_mae + sum) / 16.0;
    assert!(all <= 3.30, "hd sixteen mae {all}");
}

#[test]
fn hd_thirtytwo_keeps_the_sixteen_picture_row() {
    let opts = EncodeOptions::default();
    let (sixteen_intra, sixteen_p) = fixture_1280x720_split_sixteen();
    let sixteen = encode_sequence_split(
        &sixteen_intra,
        &sixteen_p,
        &opts,
        SHIFTED_MV,
        SPLIT_RIGHT_MV,
    )
    .unwrap();
    assert_eq!(sixteen.annex_b.len(), 272518);
    let (intra, predicted) = fixture_1280x720_split_thirtytwo();
    assert_eq!(intra, sixteen_intra);
    assert_eq!(predicted.len(), 31);
    assert_eq!(&predicted[..15], sixteen_p.as_slice());
    let sequence =
        encode_sequence_split(&intra, &predicted, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap();
    assert_eq!(sequence.reconstructed.len(), 32);
    assert_eq!(sequence.annex_b.len(), 308774);
    assert_ne!(
        &sequence.annex_b[..sixteen.annex_b.len()],
        &sixteen.annex_b[..]
    );
    let iframe = encode(&intra, &opts).unwrap();
    assert_ne!(
        &sequence.annex_b[..iframe.annex_b.len()],
        &iframe.annex_b[..]
    );
    let mut sum = 0.0;
    for (src, rec) in predicted.iter().zip(sequence.reconstructed.iter().skip(1)) {
        sum += mean_absolute_error(src, rec);
    }
    let i_mae = mean_absolute_error(&intra, &sequence.reconstructed[0]);
    let all = (i_mae + sum) / 32.0;
    assert!(all <= 3.30, "hd thirty-two mae {all}");
    let extra: Vec<_> = std::iter::repeat_with(|| intra.clone()).take(32).collect();
    assert_eq!(
        encode_sequence_split(&intra, &extra, &opts, SHIFTED_MV, SPLIT_RIGHT_MV).unwrap_err(),
        Error::Count
    );
}

#[test]
fn bad_size_and_qp() {
    let opts = EncodeOptions {
        qp: 52,
        ..EncodeOptions::default()
    };
    let frame = fixture_32x32();
    assert_eq!(encode(&frame, &opts).unwrap_err(), Error::Qp);
    assert!(Yuv420::new(30, 32, vec![0; 30 * 32], vec![0; 15 * 16], vec![0; 15 * 16]).is_err());
}
