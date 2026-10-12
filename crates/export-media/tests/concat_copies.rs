use export_media::{audiogram, concat_mp4, elementary, encode_wav, parse, WavSpec, HEIGHT, WIDTH};

fn one_second() -> Vec<u8> {
    let rate = 48_000usize;
    let mut pcm = Vec::with_capacity(rate * 2);
    for i in 0..rate {
        let s = ((i as i16).wrapping_mul(17)).wrapping_add((i as i16) >> 3);
        pcm.extend_from_slice(&s.to_le_bytes());
    }
    encode_wav(
        WavSpec {
            channels: 1,
            sample_rate: rate as u32,
            bits: 16,
        },
        &pcm,
    )
    .expect("wav")
}

#[test]
fn two_copies_of_the_audiogram_are_two_raw_samples() {
    let one = audiogram(&one_second()).expect("audiogram");
    assert_eq!(one.bytes, 327_420);
    assert_eq!(one.video_samples, 1);
    let source = parse(&one.data).expect("source");
    let source_video = source
        .tracks
        .iter()
        .find(|t| t.handler == *b"vide")
        .expect("source video");
    assert_eq!(source_video.samples.len(), 1);
    assert_eq!(source_video.width >> 16, u32::from(WIDTH));
    assert_eq!(source_video.height >> 16, u32::from(HEIGHT));
    let frame = elementary(source_video, &one.data).expect("frame")[0].to_vec();
    assert_eq!(frame.len(), usize::from(WIDTH) * usize::from(HEIGHT) * 3);

    let out = concat_mp4(&one.data, &one.data).expect("concat");
    let movie = parse(&out).expect("concat parse");
    let video = movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"vide")
        .expect("video");
    assert_eq!(video.codec, *b"raw ");
    assert_eq!(video.samples.len(), 2);
    assert_eq!(video.width >> 16, u32::from(WIDTH));
    assert_eq!(video.height >> 16, u32::from(HEIGHT));
    assert_eq!(video.stsd, source_video.stsd);
    let samples = elementary(video, &out).expect("samples");
    assert_eq!(samples.len(), 2);
    assert_eq!(samples[0], frame.as_slice());
    assert_eq!(samples[1], frame.as_slice());
}
