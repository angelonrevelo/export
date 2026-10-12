use export_h264::{
    encode, encode_ip, encode_ip_motion, fixture_32x32, fixture_32x32_predicted,
    fixture_32x32_shifted, mean_absolute_error, EncodeOptions, Error, Motion, Pass, Picture,
    Yuv420, SHIFTED_MV,
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
fn bad_size_and_qp() {
    let opts = EncodeOptions {
        qp: 52,
        ..EncodeOptions::default()
    };
    let frame = fixture_32x32();
    assert_eq!(encode(&frame, &opts).unwrap_err(), Error::Qp);
    assert!(Yuv420::new(30, 32, vec![0; 30 * 32], vec![0; 15 * 16], vec![0; 15 * 16]).is_err());
}
