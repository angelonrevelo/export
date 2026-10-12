use export_media::{
    audiogram, audiogram_repeat, decode_wav, elementary, encode_wav, parse, Error, WavSpec, HEIGHT,
    WIDTH,
};

fn tone(rate: u32, channels: u16, bits: u16) -> Vec<u8> {
    let frames = rate as usize;
    let mut pcm = Vec::new();
    for i in 0..frames {
        let loud = (i / (rate as usize / 8).max(1)) % 2 == 0;
        for c in 0..channels {
            if bits == 16 {
                let s = if loud {
                    20000 - (c as i16) * 1000
                } else {
                    -8000
                };
                pcm.extend_from_slice(&s.to_le_bytes());
            } else {
                pcm.push(if loud { 240 } else { 16 });
            }
        }
    }
    encode_wav(
        WavSpec {
            channels,
            sample_rate: rate,
            bits,
        },
        &pcm,
    )
    .expect("wav")
}

#[test]
fn ships_one_gray_frame_and_pcm() {
    let wav = tone(8_000, 1, 16);
    let out = audiogram(&wav).expect("audiogram");
    assert_eq!(out.video_samples, 1);
    assert_eq!(out.bytes, out.data.len() as u64);

    let movie = parse(&out.data).expect("parse");
    assert!(!movie.fragmented);
    let video = movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"vide")
        .expect("video");
    let audio = movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"soun")
        .expect("audio");
    assert_eq!(video.codec, *b"raw ");
    assert_eq!(video.samples.len(), 1);
    assert_eq!(video.samples.deltas, vec![8_000]);
    assert_eq!(video.timescale, 8_000);
    assert_eq!(video.width >> 16, u32::from(WIDTH));
    assert_eq!(video.height >> 16, u32::from(HEIGHT));
    assert_eq!(audio.codec, *b"sowt");
    assert_eq!(audio.samples.len(), 1);
    assert_eq!(audio.samples.deltas, vec![8_000]);
    assert_eq!(audio.channels, 1);
    assert_eq!(audio.bits, 16);

    let frames = elementary(video, &out.data).expect("video bytes");
    assert_eq!(frames.len(), 1);
    assert_eq!(
        frames[0].len(),
        usize::from(WIDTH) * usize::from(HEIGHT) * 3
    );
    let mut saw_bg = false;
    let mut saw_fg = false;
    for px in frames[0].chunks(3) {
        assert_eq!(px[0], px[1]);
        assert_eq!(px[1], px[2]);
        if px[0] == 32 {
            saw_bg = true;
        }
        if px[0] == 220 {
            saw_fg = true;
        }
    }
    assert!(saw_bg && saw_fg);

    let pcm = elementary(audio, &out.data).expect("pcm");
    let src = decode_wav(&wav).expect("src");
    assert_eq!(pcm[0], src.pcm);
}

#[test]
fn repeat_copies_one_second_at_30fps() {
    let wav = tone(8_000, 2, 16);
    let once = audiogram(&wav).expect("one");
    let repeated = audiogram_repeat(&wav, 30, 30).expect("30");
    assert_eq!(once.video_samples, 1);
    assert_eq!(repeated.video_samples, 30);
    assert!(repeated.bytes > once.bytes);

    let movie = parse(&repeated.data).expect("parse");
    let video = movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"vide")
        .expect("video");
    let audio = movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"soun")
        .expect("audio");
    assert_eq!(video.samples.len(), 30);
    assert_eq!(video.timescale, 30);
    assert!(video.samples.deltas.iter().all(|d| *d == 1));
    assert_eq!(audio.samples.len(), 1);
    let samples = elementary(video, &repeated.data).expect("frames");
    for frame in &samples[1..] {
        assert_eq!(*frame, samples[0]);
    }
    let one_movie = parse(&once.data).expect("one parse");
    let one_video = one_movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"vide")
        .expect("one video");
    let one_frame = elementary(one_video, &once.data).expect("one frame");
    assert_eq!(samples[0], one_frame[0]);
}

#[test]
fn eight_bit_expands_to_sowt() {
    let wav = tone(1_000, 1, 8);
    let out = audiogram(&wav).expect("audiogram");
    let movie = parse(&out.data).expect("parse");
    let audio = movie
        .tracks
        .iter()
        .find(|t| t.handler == *b"soun")
        .expect("audio");
    assert_eq!(audio.samples.len(), 1);
    let pcm = elementary(audio, &out.data).expect("pcm");
    assert_eq!(pcm[0].len(), 1_000 * 2);
    assert_eq!(
        movie
            .tracks
            .iter()
            .find(|t| t.handler == *b"vide")
            .unwrap()
            .samples
            .len(),
        1
    );
}

#[test]
fn rejects_non_wav() {
    let err = audiogram(b"\x00\x00\x00\x18ftypisom\x00\x00\x00\x00isomiso2").expect_err("mp4");
    assert!(matches!(
        err,
        Error::Kind {
            job: "audiogram",
            ..
        }
    ));
    let err = audiogram_repeat(b"not-a-wav", 30, 30).expect_err("unknown");
    assert!(matches!(
        err,
        Error::Kind {
            job: "audiogram",
            ..
        }
    ));
}
