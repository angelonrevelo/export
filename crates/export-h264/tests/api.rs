use export_h264::{
    encode, fixture_32x32, mean_absolute_error, EncodeOptions, Error, Pass, Picture, Yuv420,
};

#[test]
fn refuses_p_b_second_pass_and_threads() {
    let frame = fixture_32x32();
    let predicted = EncodeOptions {
        picture: Picture::Predicted,
        ..EncodeOptions::default()
    };
    assert_eq!(encode(&frame, &predicted).unwrap_err(), Error::PredictedFrame);
    let bipred = EncodeOptions {
        picture: Picture::Bipredicted,
        ..EncodeOptions::default()
    };
    assert_eq!(encode(&frame, &bipred).unwrap_err(), Error::BipredictedFrame);
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
        "P-frames are not encoded"
    );
}

#[test]
fn flat_gray_reconstructs_exactly() {
    let frame = Yuv420::flat(32, 32, 128, 128, 128).unwrap();
    let encoded = encode(&frame, &EncodeOptions::default()).unwrap();
    assert!(encoded.annex_b.starts_with(&[0, 0, 0, 1]));
    assert!(encoded.annex_b.windows(5).any(|w| w[0..4] == [0, 0, 0, 1] && w[4] == 0x67));
    assert!(encoded.annex_b.windows(5).any(|w| w[0..4] == [0, 0, 0, 1] && w[4] == 0x68));
    assert!(encoded.annex_b.windows(5).any(|w| w[0..4] == [0, 0, 0, 1] && w[4] == 0x65));
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
fn bad_size_and_qp() {
    let opts = EncodeOptions {
        qp: 52,
        ..EncodeOptions::default()
    };
    let frame = fixture_32x32();
    assert_eq!(encode(&frame, &opts).unwrap_err(), Error::Qp);
    assert!(Yuv420::new(30, 32, vec![0; 30 * 32], vec![0; 15 * 16], vec![0; 15 * 16]).is_err());
}
