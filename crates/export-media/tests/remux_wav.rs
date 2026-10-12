use export_media::{
    build_movie, decode_wav, export, export_reader, hash, sniff, Dedup, Error, Job, MediaKind,
    MovieSpec, Output, TrackSpec,
};
use std::io::Cursor;

fn av_movie(faststart: bool, interleave: bool) -> Vec<u8> {
    let video: Vec<Vec<u8>> = (0..4u8)
        .map(|i| vec![0x10 + i, 0x20, 0x30, i.wrapping_mul(3)])
        .collect();
    let audio: Vec<Vec<u8>> = (0..8u16)
        .map(|i| i16::to_le_bytes((i as i16) * 1000).to_vec())
        .collect();
    build_movie(&MovieSpec {
        brand: *b"isom",
        timescale: 1000,
        faststart,
        interleave,
        tracks: vec![
            TrackSpec {
                handler: *b"vide",
                codec: *b"raw ",
                timescale: 30,
                width: 16,
                height: 16,
                channels: 0,
                bits: 0,
                sample_rate: 0,
                deltas: vec![1; video.len()],
                payloads: video,
            },
            TrackSpec {
                handler: *b"soun",
                codec: *b"sowt",
                timescale: 8,
                width: 0,
                height: 0,
                channels: 1,
                bits: 16,
                sample_rate: 8,
                deltas: vec![1; audio.len()],
                payloads: audio,
            },
        ],
    })
    .unwrap()
}

fn payloads(bytes: &[u8], audio: bool) -> Vec<Vec<u8>> {
    let movie = export_media::parse(bytes).unwrap();
    let handler: [u8; 4] = if audio { *b"soun" } else { *b"vide" };
    let track = movie.tracks.iter().find(|t| t.handler == handler).unwrap();
    export_media::elementary(track, bytes)
        .unwrap()
        .into_iter()
        .map(|s| s.to_vec())
        .collect()
}

#[test]
fn remux_faststart_keeps_interleaved_samples() {
    let src = av_movie(false, true);
    assert!(
        export_media::parse(&src).unwrap().moov_pos > export_media::parse(&src).unwrap().mdat_pos
    );
    let video = payloads(&src, false);
    let audio = payloads(&src, true);
    let mut dedup = Dedup::new();
    let out = export(
        &src,
        Job::Remux {
            faststart: true,
            audio_only: false,
            video_only: false,
        },
        &mut dedup,
    )
    .unwrap();
    let Output::Fresh { digest, kind, data } = out else {
        panic!("expected a first ingest");
    };
    assert_eq!(digest, hash(&src));
    assert!(matches!(kind, MediaKind::Mp4 { brand } if &brand == b"isom"));
    let parsed = export_media::parse(&data).unwrap();
    assert!(parsed.moov_pos < parsed.mdat_pos);
    assert_eq!(payloads(&data, false), video);
    assert_eq!(payloads(&data, true), audio);
    assert_eq!(parsed.tracks[1].codec, *b"sowt");
    assert_eq!(parsed.tracks[0].codec, *b"raw ");

    let again = export(
        &src,
        Job::Wav {
            sample_rate: Some(16000),
            channels: Some(1),
        },
        &mut dedup,
    )
    .unwrap();
    assert!(matches!(again, Output::Duplicate { .. }));
    assert_eq!(dedup.len(), 1);
}

#[test]
fn audio_only_remux_and_wav_resample() {
    let src = av_movie(false, true);
    let audio = payloads(&src, true);
    let mut dedup = Dedup::new();
    let out = export(
        &src,
        Job::Remux {
            faststart: true,
            audio_only: true,
            video_only: false,
        },
        &mut dedup,
    )
    .unwrap();
    let Output::Fresh { data, .. } = out else {
        panic!("fresh")
    };
    let parsed = export_media::parse(&data).unwrap();
    assert_eq!(parsed.tracks.len(), 1);
    assert_eq!(parsed.tracks[0].handler, *b"soun");
    assert_eq!(payloads(&data, true), audio);

    let mut dedup = Dedup::new();
    let wav = export(
        &src,
        Job::Wav {
            sample_rate: None,
            channels: None,
        },
        &mut dedup,
    )
    .unwrap();
    let Output::Fresh { data, kind, .. } = wav else {
        panic!("fresh")
    };
    assert_eq!(kind, MediaKind::Mp4 { brand: *b"isom" });
    let decoded = decode_wav(&data).unwrap();
    assert_eq!(decoded.spec.channels, 1);
    assert_eq!(decoded.spec.sample_rate, 8);
    assert_eq!(decoded.pcm, audio.concat());

    let mut dedup = Dedup::new();
    let half = export(
        &src,
        Job::Wav {
            sample_rate: Some(4),
            channels: Some(1),
        },
        &mut dedup,
    )
    .unwrap();
    let Output::Fresh { data, .. } = half else {
        panic!("fresh")
    };
    let decoded = decode_wav(&data).unwrap();
    assert_eq!(decoded.spec.sample_rate, 4);
    assert_eq!(decoded.pcm.len(), audio.concat().len() / 2);
}

#[test]
fn wav_bytes_are_not_an_mp4_whatever_the_name_would_be() {
    let pcm: Vec<i16> = (0..32).map(|i| i * 100).collect();
    let mut raw = Vec::with_capacity(pcm.len() * 2);
    for s in &pcm {
        raw.extend_from_slice(&s.to_le_bytes());
    }
    let bytes = export_media::encode_wav(
        export_media::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits: 16,
        },
        &raw,
    )
    .unwrap();
    assert_eq!(sniff(&bytes), MediaKind::Wav);
    let mut dedup = Dedup::new();
    let err = export(
        &bytes,
        Job::Remux {
            faststart: true,
            audio_only: false,
            video_only: false,
        },
        &mut dedup,
    )
    .unwrap_err();
    assert!(matches!(
        err,
        Error::Kind {
            found: MediaKind::Wav,
            ..
        }
    ));
    assert!(dedup.is_empty());

    let out = export(
        &bytes,
        Job::Wav {
            sample_rate: Some(8000),
            channels: Some(1),
        },
        &mut dedup,
    )
    .unwrap();
    let Output::Fresh { data, .. } = out else {
        panic!("fresh")
    };
    let decoded = decode_wav(&data).unwrap();
    assert_eq!(decoded.spec.sample_rate, 8000);
    assert_eq!(decoded.pcm.len(), (pcm.len() / 2) * 2);
}

#[test]
fn corrupt_and_fragmented_do_not_enter_the_dedup_set() {
    let mut src = av_movie(true, false);
    src.extend_from_slice(&8u32.to_be_bytes());
    src.extend_from_slice(b"moof");
    let mut dedup = Dedup::new();
    let err = export(
        &src,
        Job::Remux {
            faststart: true,
            audio_only: false,
            video_only: false,
        },
        &mut dedup,
    )
    .unwrap_err();
    assert!(matches!(err, Error::Unsupported(_)));
    assert!(dedup.is_empty());
}

#[test]
fn reader_stops_at_the_cap_before_a_codec() {
    let src = av_movie(false, false);
    let mut dedup = Dedup::new();
    let err = export_reader(
        &mut Cursor::new(src),
        32,
        Job::Remux {
            faststart: true,
            audio_only: false,
            video_only: false,
        },
        &mut dedup,
    )
    .unwrap_err();
    assert!(matches!(err, Error::Limit { cap: 32, .. }));
    assert!(dedup.is_empty());
}

#[test]
fn elementary_audio_is_the_pcm_bytes() {
    let src = av_movie(false, true);
    let audio = payloads(&src, true);
    let mut dedup = Dedup::new();
    let out = export(&src, Job::Elementary { audio: true }, &mut dedup).unwrap();
    let Output::Fresh { data, .. } = out else {
        panic!("fresh")
    };
    assert_eq!(data, audio.concat());
}
