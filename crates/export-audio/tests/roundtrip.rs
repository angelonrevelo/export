use export_audio::{export, hash, hex, sniff, Dedup, Error, Job, Kind, Output, DEFAULT_MAX_BYTES};
use std::io::Cursor;

fn tone(n: usize, channels: u16) -> export_audio::Pcm {
    let ch = channels as usize;
    let mut samples = Vec::with_capacity(n * ch);
    for i in 0..n {
        for c in 0..ch {
            let s = ((i * (3 + c) + c * 17) % 400) as i32 - 200;
            let wobble = ((i * i) % 50) as i32;
            samples.push((s + wobble - if c == 0 { 0 } else { 20 }).clamp(-32_768, 32_767));
        }
    }
    export_audio::Pcm {
        sample_rate: 44_100,
        channels,
        bits: 16,
        samples,
    }
}

#[test]
fn generated_fixture_round_trips_exact_samples() {
    let pcm = tone(12_000, 2);
    let flac = export_audio::encode(&pcm).unwrap();
    assert_eq!(sniff(&flac), Kind::Flac);
    let back = export_audio::decode(&flac).unwrap();
    assert_eq!(back.samples, pcm.samples);
    assert_eq!(back.channels, 2);
    assert_eq!(back.sample_rate, 44_100);
    assert_eq!(back.bits, 16);
}

#[test]
fn truncated_stream_and_flipped_crc_error() {
    let pcm = tone(64, 1);
    let flac = export_audio::encode(&pcm).unwrap();
    assert!(flac.len() > 8);

    let mut cut = flac.clone();
    cut.truncate(flac.len() - 1);
    assert!(
        matches!(export_audio::decode(&cut), Err(Error::Truncated { .. })),
        "short read: {:?}",
        export_audio::decode(&cut)
    );

    let mut early = flac.clone();
    early.truncate(4);
    assert!(export_audio::decode(&early).is_err());

    let mut flip = flac.clone();
    let last = flip.len() - 1;
    flip[last] ^= 0xff;
    assert!(
        matches!(export_audio::decode(&flip), Err(Error::Corrupt("crc16"))),
        "crc flip: {:?}",
        export_audio::decode(&flip)
    );
}

#[test]
fn mp3_aac_and_opus_are_unsupported() {
    let mut dedup = Dedup::new();
    let mp3 = b"ID3\x03\x00\x00\x00\x00\x00\x00";
    let err = export(mp3, Job::ToFlac, &mut dedup).unwrap_err();
    assert!(
        matches!(err, Error::Unsupported(msg) if msg.contains("MP3")),
        "{err}"
    );
    assert!(dedup.is_empty(), "a refused file is not a duplicate");

    let aac = [0xff, 0xf1, 0x50, 0x80, 0x00, 0x1f, 0xfc];
    let err = export(&aac, Job::ToWav, &mut dedup).unwrap_err();
    assert!(
        matches!(err, Error::Unsupported(msg) if msg.contains("AAC")),
        "{err}"
    );

    let mut opus = b"OggS".to_vec();
    opus.extend_from_slice(&[0u8; 28]);
    opus.extend_from_slice(b"OpusHead");
    let err = export(&opus, Job::ToFlac, &mut dedup).unwrap_err();
    assert!(
        matches!(err, Error::Unsupported(msg) if msg.contains("Opus")),
        "{err}"
    );
    assert!(dedup.is_empty());
}

#[test]
fn hash_once_then_dedup_then_codec() {
    let pcm = tone(128, 1);
    let flac = export_audio::encode(&pcm).unwrap();
    let digest = hash(&flac);
    let mut dedup = Dedup::new();
    let first = export(&flac, Job::ToWav, &mut dedup).unwrap();
    match first {
        Output::Fresh {
            digest: got,
            kind,
            data,
        } => {
            assert_eq!(got, digest);
            assert_eq!(kind, Kind::Flac);
            assert!(data.starts_with(b"RIFF"));
        }
        Output::Duplicate { .. } => panic!("first insert was treated as a hit"),
    }
    let second = export(&flac, Job::ToWav, &mut dedup).unwrap();
    assert!(matches!(second, Output::Duplicate { digest: d } if d == digest));
    assert_eq!(dedup.len(), 1);

    let broken = {
        let mut b = flac.clone();
        b.truncate(b.len() / 2);
        b
    };
    assert!(export(&broken, Job::ToFlac, &mut dedup).is_err());
    assert_eq!(dedup.len(), 1);
}

#[test]
fn four_thousand_sample_wav_matches_the_plain_pcm_s16le_file() {
    let mut samples = Vec::with_capacity(4096);
    for i in 0..4096i32 {
        let s = ((i % 200) - 100) * 40 + (i * i) % 30;
        samples.push(s.clamp(-32_768, 32_767));
    }
    let pcm = export_audio::Pcm {
        sample_rate: 44_100,
        channels: 1,
        bits: 16,
        samples,
    };
    let flac = export_audio::encode(&pcm).unwrap();
    let Output::Fresh { data, .. } = export(&flac, Job::ToWav, &mut Dedup::new()).unwrap() else {
        panic!("expected a fresh wav");
    };
    assert_eq!(data.len(), 8270);
    assert_eq!(
        &data[36..70],
        b"LIST\x1a\x00\x00\x00INFOISFT\x0e\x00\x00\x00Lavf60.16.100\x00"
    );
    assert_eq!(
        hex(&hash(&data)),
        "d114e84a93552a2325cf2495318614f6d79cd9c8d7a1d5994ebf79c349e7da55"
    );
}

#[test]
fn reader_hashes_in_one_pass_and_honors_the_cap() {
    let pcm = tone(64, 1);
    let flac = export_audio::encode(&pcm).unwrap();
    let mut dedup = Dedup::new();
    let out = export_audio::export_reader(
        &mut Cursor::new(&flac),
        DEFAULT_MAX_BYTES,
        Job::ToFlac,
        &mut dedup,
    )
    .unwrap();
    let Output::Fresh { data, digest, .. } = out else {
        panic!("miss expected");
    };
    assert_eq!(digest, hash(&flac));
    let back = export_audio::decode(&data).unwrap();
    assert_eq!(back.samples, pcm.samples);

    let err = export_audio::export_reader(
        &mut Cursor::new(vec![0u8; 100]),
        32,
        Job::ToFlac,
        &mut Dedup::new(),
    );
    assert!(matches!(err, Err(Error::Limit { .. })));
}
