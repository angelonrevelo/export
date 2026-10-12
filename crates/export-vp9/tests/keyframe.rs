use export_vp9::{decode_ivf, decode_keyframe, Error};

const IVF: &[u8] = include_bytes!("../testdata/keyframe.ivf");
const YUV: &[u8] = include_bytes!("../testdata/ffmpeg.yuv");
const IVF16: &[u8] = include_bytes!("../testdata/keyframe-16.ivf");
const YUV16: &[u8] = include_bytes!("../testdata/ffmpeg-16.yuv");
const IVF32: &[u8] = include_bytes!("../testdata/keyframe-32.ivf");
const YUV32: &[u8] = include_bytes!("../testdata/ffmpeg-32.yuv");
const IVF64: &[u8] = include_bytes!("../testdata/keyframe-64.ivf");
const YUV64: &[u8] = include_bytes!("../testdata/ffmpeg-64.yuv");

fn payload() -> &'static [u8] {
    &IVF[44..]
}

#[test]
fn pixels_match_ffmpeg_within_bound() {
    let frame = decode_ivf(IVF).expect("keyframe");
    assert_eq!((frame.width, frame.height), (8, 8));
    let mut got = Vec::new();
    got.extend_from_slice(&frame.y);
    got.extend_from_slice(&frame.u);
    got.extend_from_slice(&frame.v);
    assert_eq!(got.len(), YUV.len());
    let mut max_abs = 0i32;
    for (decoded, ffmpeg) in got.iter().zip(YUV.iter()) {
        let diff = (i32::from(*decoded) - i32::from(*ffmpeg)).abs();
        max_abs = max_abs.max(diff);
    }
    let error_bound = 0;
    assert!(
        max_abs <= error_bound,
        "max abs error {max_abs} exceeds bound {error_bound}"
    );
    assert_eq!(got.as_slice(), YUV);
}

#[test]
fn truncated_ivf_errors() {
    assert_eq!(decode_ivf(&IVF[..10]), Err(Error::Truncated));
    assert_eq!(decode_keyframe(&payload()[..8]), Err(Error::Truncated));
}

#[test]
fn bad_header_errors() {
    let mut bad = IVF.to_vec();
    bad[0] = b'X';
    assert_eq!(decode_ivf(&bad), Err(Error::BadHeader));

    let mut frame = payload().to_vec();
    frame[1] = 0x00;
    assert_eq!(decode_keyframe(&frame), Err(Error::BadHeader));

    let mut marker = payload().to_vec();
    marker[0] = 0x00;
    assert_eq!(decode_keyframe(&marker), Err(Error::BadHeader));
}

#[test]
fn inter_frame_unsupported() {
    let mut frame = payload().to_vec();
    frame[0] |= 0x04;
    assert_eq!(decode_keyframe(&frame), Err(Error::Unsupported));
}

#[test]
fn vp8_unsupported() {
    let mut file = IVF.to_vec();
    file[8..12].copy_from_slice(b"VP80");
    assert_eq!(decode_ivf(&file), Err(Error::Unsupported));
}

#[test]
fn pixels_16_match_ffmpeg_within_bound() {
    let frame = decode_ivf(IVF16).expect("16x16 keyframe");
    assert_eq!((frame.width, frame.height), (16, 16));
    let mut got = Vec::new();
    got.extend_from_slice(&frame.y);
    got.extend_from_slice(&frame.u);
    got.extend_from_slice(&frame.v);
    assert_eq!(got.len(), YUV16.len());
    let mut max_abs = 0i32;
    for (decoded, ffmpeg) in got.iter().zip(YUV16.iter()) {
        let diff = (i32::from(*decoded) - i32::from(*ffmpeg)).abs();
        max_abs = max_abs.max(diff);
    }
    let error_bound = 0;
    assert!(
        max_abs <= error_bound,
        "max abs error {max_abs} exceeds bound {error_bound}"
    );
    assert_eq!(got.as_slice(), YUV16);
}

#[test]
fn pixels_32_match_ffmpeg_within_bound() {
    let frame = decode_ivf(IVF32).expect("32x32 keyframe");
    assert_eq!((frame.width, frame.height), (32, 32));
    let mut got = Vec::new();
    got.extend_from_slice(&frame.y);
    got.extend_from_slice(&frame.u);
    got.extend_from_slice(&frame.v);
    assert_eq!(got.len(), YUV32.len());
    let mut max_abs = 0i32;
    for (decoded, ffmpeg) in got.iter().zip(YUV32.iter()) {
        let diff = (i32::from(*decoded) - i32::from(*ffmpeg)).abs();
        max_abs = max_abs.max(diff);
    }
    let error_bound = 0;
    assert!(
        max_abs <= error_bound,
        "max abs error {max_abs} exceeds bound {error_bound}"
    );
    assert_eq!(got.as_slice(), YUV32);
}

#[test]
fn pixels_64_match_ffmpeg_within_bound() {
    let frame = decode_ivf(IVF64).expect("64x64 keyframe");
    assert_eq!((frame.width, frame.height), (64, 64));
    let mut got = Vec::new();
    got.extend_from_slice(&frame.y);
    got.extend_from_slice(&frame.u);
    got.extend_from_slice(&frame.v);
    assert_eq!(got.len(), YUV64.len());
    let mut max_abs = 0i32;
    for (decoded, ffmpeg) in got.iter().zip(YUV64.iter()) {
        let diff = (i32::from(*decoded) - i32::from(*ffmpeg)).abs();
        max_abs = max_abs.max(diff);
    }
    let error_bound = 0;
    assert!(
        max_abs <= error_bound,
        "max abs error {max_abs} exceeds bound {error_bound}"
    );
    assert_eq!(got.as_slice(), YUV64);
}

#[test]
fn truncated_64_errors() {
    assert_eq!(decode_ivf(&IVF64[..10]), Err(Error::Truncated));
    assert_eq!(decode_keyframe(&IVF64[44..52]), Err(Error::Truncated));
}

#[test]
fn truncated_32_errors() {
    assert_eq!(decode_ivf(&IVF32[..10]), Err(Error::Truncated));
    assert_eq!(decode_keyframe(&IVF32[44..52]), Err(Error::Truncated));
}

#[test]
fn truncated_16_errors() {
    assert_eq!(decode_ivf(&IVF16[..10]), Err(Error::Truncated));
    assert_eq!(decode_keyframe(&IVF16[44..52]), Err(Error::Truncated));
}

#[test]
fn second_frame_unsupported() {
    let mut file = IVF.to_vec();
    file[24] = 2;
    let packet = IVF[32..].to_vec();
    file.extend_from_slice(&packet);
    assert_eq!(decode_ivf(&file), Err(Error::Unsupported));

    let mut extra = IVF.to_vec();
    extra.extend_from_slice(&IVF[32..]);
    assert_eq!(decode_ivf(&extra), Err(Error::Unsupported));
}
