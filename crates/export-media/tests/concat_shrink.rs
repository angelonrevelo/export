use export_media::{
    build_movie, concat_mp4, decode_wav, encode_wav, parse, shrink_wav, Error, MovieSpec,
    TrackSpec, WavSpec,
};

fn movie(codec: [u8; 4], channels: u16, tag: u8) -> Vec<u8> {
    let video: Vec<Vec<u8>> = (0..3).map(|i| vec![tag, i, 0xab]).collect();
    let audio: Vec<Vec<u8>> = (0..4)
        .map(|i| i16::to_le_bytes(i16::from(tag) * 10 + i as i16).to_vec())
        .collect();
    build_movie(&MovieSpec {
        brand: *b"isom",
        timescale: 1000,
        faststart: false,
        interleave: true,
        tracks: vec![
            TrackSpec {
                handler: *b"vide",
                codec: *b"raw ",
                timescale: 30,
                width: 8,
                height: 8,
                channels: 0,
                bits: 0,
                sample_rate: 0,
                deltas: vec![1; video.len()],
                payloads: video,
            },
            TrackSpec {
                handler: *b"soun",
                codec,
                timescale: 8,
                width: 0,
                height: 0,
                channels,
                bits: 16,
                sample_rate: 8,
                deltas: vec![1; audio.len()],
                payloads: audio,
            },
        ],
    })
    .unwrap()
}

fn track_bytes(file: &[u8], audio: bool) -> Vec<Vec<u8>> {
    let parsed = parse(file).unwrap();
    let handler: [u8; 4] = if audio { *b"soun" } else { *b"vide" };
    let track = parsed.tracks.iter().find(|t| t.handler == handler).unwrap();
    export_media::elementary(track, file)
        .unwrap()
        .into_iter()
        .map(|s| s.to_vec())
        .collect()
}

#[test]
fn concat_appends_samples_without_reencoding() {
    let a = movie(*b"sowt", 1, 1);
    let b = movie(*b"sowt", 1, 2);
    let out = concat_mp4(&a, &b).unwrap();
    let parsed = parse(&out).unwrap();
    assert!(parsed.moov_pos < parsed.mdat_pos);
    let mut video = track_bytes(&a, false);
    video.extend(track_bytes(&b, false));
    let mut audio = track_bytes(&a, true);
    audio.extend(track_bytes(&b, true));
    assert_eq!(track_bytes(&out, false), video);
    assert_eq!(track_bytes(&out, true), audio);
    assert_eq!(parsed.tracks[0].codec, *b"raw ");
    assert_eq!(parsed.tracks[1].codec, *b"sowt");
    assert_eq!(parsed.tracks[0].stsd, parse(&a).unwrap().tracks[0].stsd);
}

#[test]
fn concat_rejects_codec_mismatch_fragment_and_wav() {
    let a = movie(*b"sowt", 1, 1);
    let other = movie(*b"twos", 1, 1);
    assert_eq!(
        concat_mp4(&a, &other).unwrap_err(),
        Error::Unsupported("codec mismatch")
    );
    let rate = movie(*b"sowt", 2, 1);
    assert_eq!(
        concat_mp4(&a, &rate).unwrap_err(),
        Error::Unsupported("sample entry mismatch")
    );
    let mut broken = a.clone();
    broken.extend_from_slice(&8u32.to_be_bytes());
    broken.extend_from_slice(b"moof");
    assert_eq!(
        concat_mp4(&a, &broken).unwrap_err(),
        Error::Unsupported("fragmented mp4 (moof) is refused, not flattened")
    );
    let wav = encode_wav(
        WavSpec {
            channels: 1,
            sample_rate: 8000,
            bits: 16,
        },
        &[0, 0, 0, 0],
    )
    .unwrap();
    assert!(matches!(
        concat_mp4(&wav, &a).unwrap_err(),
        Error::Kind { job: "concat", .. }
    ));
}

#[test]
fn wav_shrink_is_one_pass_and_reports_its_bytes() {
    let frames = 8000usize;
    let mut pcm = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let s = (i as i16).wrapping_mul(3);
        pcm.extend_from_slice(&s.to_le_bytes());
        pcm.extend_from_slice(&(-s).to_le_bytes());
    }
    let wav = encode_wav(
        WavSpec {
            channels: 2,
            sample_rate: 8000,
            bits: 16,
        },
        &pcm,
    )
    .unwrap();
    assert!(wav.len() as u64 > 20_000);

    let kept = shrink_wav(&wav, wav.len() as u64).unwrap();
    assert_eq!(kept.bytes, wav.len() as u64);
    assert_eq!(kept.data, wav);

    let mono = shrink_wav(&wav, 20_000).unwrap();
    assert_eq!(mono.bytes, mono.data.len() as u64);
    assert!(mono.bytes <= 20_000);
    let decoded = decode_wav(&mono.data).unwrap();
    assert_eq!(decoded.spec.channels, 1);
    assert_eq!(decoded.spec.sample_rate, 8000);
    assert_eq!(mono.bytes, 44 + frames as u64 * 2);

    let tight = shrink_wav(&wav, 1_000).unwrap();
    assert_eq!(tight.bytes, tight.data.len() as u64);
    assert!(tight.bytes <= 1_000);
    let decoded = decode_wav(&tight.data).unwrap();
    assert_eq!(decoded.spec.channels, 1);
    assert!(decoded.spec.sample_rate < 8000);
    let max_frames = (1_000 - 44) / 2;
    assert_eq!(decoded.spec.sample_rate, max_frames as u32);

    assert!(matches!(
        shrink_wav(&movie(*b"sowt", 1, 1), 100).unwrap_err(),
        Error::Kind {
            job: "wav-shrink",
            ..
        }
    ));
}
