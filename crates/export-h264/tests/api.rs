use export_h264::{
    encode, encode_ip, encode_ip_motion, encode_sequence, fixture_160x160, fixture_160x160_eight,
    fixture_160x160_predicted, fixture_160x160_shifted, fixture_160x160_sixteen, fixture_32x32,
    fixture_32x32_eight, fixture_32x32_predicted, fixture_32x32_shifted, mean_absolute_error,
    EncodeOptions, Error, Motion, Pass, Picture, PredictedFrame, Yuv420, SHIFTED_MV,
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
        176,
        176,
        vec![0; 176 * 176],
        vec![0; 88 * 88],
        vec![0; 88 * 88]
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
fn bad_size_and_qp() {
    let opts = EncodeOptions {
        qp: 52,
        ..EncodeOptions::default()
    };
    let frame = fixture_32x32();
    assert_eq!(encode(&frame, &opts).unwrap_err(), Error::Qp);
    assert!(Yuv420::new(30, 32, vec![0; 30 * 32], vec![0; 15 * 16], vec![0; 15 * 16]).is_err());
}
