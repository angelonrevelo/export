use export_avcbox::{
    box_iframe, Error, Options, FOUR_320X240, IFRAME_160X160, IFRAME_320X240, IFRAME_32X32,
    PAIR_320X240, SEQUENCE_160X160,
};

fn opts() -> Options {
    Options::default()
}

#[test]
fn fixture_is_one_avc1_sample() {
    assert_eq!(IFRAME_32X32.len(), 272);
    assert!(IFRAME_32X32
        .windows(7)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x0a, 0xda, 0x25, 0x90]));
    let boxed = box_iframe(IFRAME_32X32, &opts()).unwrap();
    assert_eq!(boxed.width, 32);
    assert_eq!(boxed.height, 32);
    assert_eq!(boxed.mp4.len(), 888);
    assert!(boxed.mp4.starts_with(b"\x00\x00\x00\x20ftyp"));
    let kinds = top_level(&boxed.mp4);
    assert_eq!(kinds, vec!["ftyp", "moov", "mdat"]);
    assert_eq!(count(&boxed.mp4, b"avcC"), 1);
    assert_eq!(count(&boxed.mp4, b"avc1"), 2);
    let stsz = find(&boxed.mp4, b"stsz");
    let sample_count = u32::from_be_bytes(boxed.mp4[stsz + 12..stsz + 16].try_into().unwrap());
    let sample_size = u32::from_be_bytes(boxed.mp4[stsz + 16..stsz + 20].try_into().unwrap());
    assert_eq!(sample_count, 1);
    assert_eq!(sample_size, 247);
    assert!(boxed.mp4.ends_with(&IFRAME_32X32[29..]));
    let again = box_iframe(IFRAME_32X32, &opts()).unwrap();
    assert_eq!(again.mp4, boxed.mp4);
}

#[test]
fn larger_iframe_is_one_avc1_sample() {
    assert_eq!(IFRAME_160X160.len(), 6432);
    assert!(IFRAME_160X160
        .windows(4)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x0b]));
    let boxed = box_iframe(IFRAME_160X160, &opts()).unwrap();
    assert_eq!(boxed.width, 160);
    assert_eq!(boxed.height, 160);
    assert_eq!(boxed.mp4.len(), 7048);
    assert!(boxed.mp4.starts_with(b"\x00\x00\x00\x20ftyp"));
    let kinds = top_level(&boxed.mp4);
    assert_eq!(kinds, vec!["ftyp", "moov", "mdat"]);
    assert_eq!(count(&boxed.mp4, b"avcC"), 1);
    let stsz = find(&boxed.mp4, b"stsz");
    let sample_count = u32::from_be_bytes(boxed.mp4[stsz + 12..stsz + 16].try_into().unwrap());
    let sample_size = u32::from_be_bytes(boxed.mp4[stsz + 16..stsz + 20].try_into().unwrap());
    assert_eq!(sample_count, 1);
    assert_eq!(sample_size, 6406);
    assert!(boxed.mp4.windows(4).any(|w| w == [0x67, 0x42, 0xc0, 0x0b]));
    assert_eq!(
        box_iframe(&IFRAME_160X160[..IFRAME_160X160.len() - 1], &opts()).unwrap_err(),
        Error::Truncated
    );
    let again = box_iframe(IFRAME_160X160, &opts()).unwrap();
    assert_eq!(again.mp4, boxed.mp4);
    let small = box_iframe(IFRAME_32X32, &opts()).unwrap();
    assert_eq!(small.mp4.len(), 888);
}

#[test]
fn wider_iframe_is_one_avc1_sample() {
    assert_eq!(IFRAME_320X240.len(), 19337);
    assert!(IFRAME_320X240
        .windows(4)
        .any(|w| w == [0x67, 0x42, 0xc0, 0x0b]));
    let boxed = box_iframe(IFRAME_320X240, &opts()).unwrap();
    assert_eq!(boxed.width, 320);
    assert_eq!(boxed.height, 240);
    assert_eq!(boxed.samples, 1);
    assert_eq!(boxed.mp4.len(), 19953);
    assert!(boxed.mp4.starts_with(b"\x00\x00\x00\x20ftyp"));
    assert_eq!(top_level(&boxed.mp4), vec!["ftyp", "moov", "mdat"]);
    assert_eq!(count(&boxed.mp4, b"avcC"), 1);
    let stsz = find(&boxed.mp4, b"stsz");
    let sample_count = u32::from_be_bytes(boxed.mp4[stsz + 12..stsz + 16].try_into().unwrap());
    let sample_size = u32::from_be_bytes(boxed.mp4[stsz + 16..stsz + 20].try_into().unwrap());
    assert_eq!(sample_count, 1);
    assert_eq!(sample_size, 19311);
    assert_eq!(
        box_iframe(&IFRAME_320X240[..IFRAME_320X240.len() - 1], &opts()).unwrap_err(),
        Error::Truncated
    );
    let again = box_iframe(IFRAME_320X240, &opts()).unwrap();
    assert_eq!(again.mp4, boxed.mp4);
    assert_eq!(box_iframe(IFRAME_32X32, &opts()).unwrap().mp4.len(), 888);
    assert_eq!(box_iframe(IFRAME_160X160, &opts()).unwrap().mp4.len(), 7048);
    assert_eq!(
        box_iframe(SEQUENCE_160X160, &opts()).unwrap().mp4.len(),
        7819
    );
}

#[test]
fn pair_is_two_320_by_240_samples() {
    assert_eq!(PAIR_320X240.len(), 29090);
    assert!(PAIR_320X240.starts_with(IFRAME_320X240));
    let boxed = box_iframe(PAIR_320X240, &opts()).unwrap();
    assert_eq!(boxed.width, 320);
    assert_eq!(boxed.height, 240);
    assert_eq!(boxed.samples, 2);
    assert_eq!(boxed.mp4.len(), 29724);
    assert!(boxed.mp4.starts_with(b"\x00\x00\x00\x20ftyp"));
    assert_eq!(top_level(&boxed.mp4), vec!["ftyp", "moov", "mdat"]);
    let stsz = find(&boxed.mp4, b"stsz");
    let sample_count = u32::from_be_bytes(boxed.mp4[stsz + 12..stsz + 16].try_into().unwrap());
    assert_eq!(sample_count, boxed.samples);
    assert_eq!(
        box_iframe(&PAIR_320X240[..PAIR_320X240.len() - 1], &opts()).unwrap_err(),
        Error::Truncated
    );
    let prefix = box_iframe(&PAIR_320X240[..IFRAME_320X240.len()], &opts()).unwrap();
    assert_eq!(prefix.mp4.len(), 19953);
    assert_eq!(prefix.samples, 1);
    assert_eq!(
        box_iframe(SEQUENCE_160X160, &opts()).unwrap().mp4.len(),
        7819
    );
    let again = box_iframe(PAIR_320X240, &opts()).unwrap();
    assert_eq!(again.mp4, boxed.mp4);
}

#[test]
fn four_pictures_stay_on_the_320_level() {
    assert_eq!(FOUR_320X240.len(), 29559);
    assert!(FOUR_320X240.starts_with(IFRAME_320X240));
    let boxed = box_iframe(FOUR_320X240, &opts()).unwrap();
    assert_eq!(boxed.width, 320);
    assert_eq!(boxed.height, 240);
    assert_eq!(boxed.samples, 4);
    assert_eq!(boxed.mp4.len(), 30189);
    assert!(boxed.mp4.starts_with(b"\x00\x00\x00\x20ftyp"));
    assert_eq!(top_level(&boxed.mp4), vec!["ftyp", "moov", "mdat"]);
    let stsz = find(&boxed.mp4, b"stsz");
    let sample_count = u32::from_be_bytes(boxed.mp4[stsz + 12..stsz + 16].try_into().unwrap());
    assert_eq!(sample_count, boxed.samples);
    assert_eq!(
        box_iframe(&FOUR_320X240[..FOUR_320X240.len() - 1], &opts()).unwrap_err(),
        Error::Truncated
    );
    let prefix = box_iframe(&FOUR_320X240[..IFRAME_320X240.len()], &opts()).unwrap();
    assert_eq!(prefix.mp4.len(), 19953);
    assert_eq!(prefix.samples, 1);
    assert_eq!(box_iframe(PAIR_320X240, &opts()).unwrap().mp4.len(), 29724);
    let again = box_iframe(FOUR_320X240, &opts()).unwrap();
    assert_eq!(again.mp4, boxed.mp4);
}

#[test]
fn sequence_is_160_by_160_and_keeps_the_single_frames() {
    assert_eq!(SEQUENCE_160X160.len(), 7197);
    assert!(SEQUENCE_160X160.starts_with(IFRAME_160X160));
    let boxed = box_iframe(SEQUENCE_160X160, &opts()).unwrap();
    assert_eq!(boxed.width, 160);
    assert_eq!(boxed.height, 160);
    assert_eq!(boxed.samples, 8);
    assert_eq!(boxed.mp4.len(), 7819);
    assert!(boxed.mp4.starts_with(b"\x00\x00\x00\x20ftyp"));
    assert_eq!(top_level(&boxed.mp4), vec!["ftyp", "moov", "mdat"]);
    assert_eq!(count(&boxed.mp4, b"avcC"), 1);
    let stsz = find(&boxed.mp4, b"stsz");
    let default_size = u32::from_be_bytes(boxed.mp4[stsz + 8..stsz + 12].try_into().unwrap());
    let sample_count = u32::from_be_bytes(boxed.mp4[stsz + 12..stsz + 16].try_into().unwrap());
    assert_eq!(default_size, 0);
    assert_eq!(sample_count, boxed.samples);
    assert!(sample_count > 1);
    let mut sum = 0u32;
    for i in 0..sample_count {
        let at = stsz + 16 + (i as usize) * 4;
        sum += u32::from_be_bytes(boxed.mp4[at..at + 4].try_into().unwrap());
    }
    let mdat = find(&boxed.mp4, b"mdat");
    let mdat_size = u32::from_be_bytes(boxed.mp4[mdat - 4..mdat].try_into().unwrap());
    assert_eq!(sum, mdat_size - 8);
    assert_eq!(
        box_iframe(&SEQUENCE_160X160[..SEQUENCE_160X160.len() - 1], &opts()).unwrap_err(),
        Error::Truncated
    );
    let prefix = box_iframe(&SEQUENCE_160X160[..IFRAME_160X160.len()], &opts()).unwrap();
    assert_eq!(prefix.mp4.len(), 7048);
    assert_eq!(prefix.samples, 1);
    let small = box_iframe(IFRAME_32X32, &opts()).unwrap();
    assert_eq!(small.mp4.len(), 888);
    assert_eq!(small.samples, 1);
    let again = box_iframe(SEQUENCE_160X160, &opts()).unwrap();
    assert_eq!(again.mp4, boxed.mp4);
}

#[test]
fn truncated_annex_b_is_an_error() {
    assert_eq!(box_iframe(&[], &opts()).unwrap_err(), Error::Truncated);
    assert_eq!(
        box_iframe(&[0, 0, 0], &opts()).unwrap_err(),
        Error::Truncated
    );
    for n in 0..IFRAME_32X32.len() {
        assert!(
            box_iframe(&IFRAME_32X32[..n], &opts()).is_err(),
            "prefix {n} boxed"
        );
    }
    assert_eq!(
        box_iframe(&IFRAME_32X32[..IFRAME_32X32.len() - 1], &opts()).unwrap_err(),
        Error::Truncated
    );
    assert_eq!(
        Error::Truncated.to_string(),
        "the Annex B buffer is truncated"
    );
}

#[test]
fn threading_is_off_and_a_p_slice_is_refused() {
    let threaded = Options { threads: 4 };
    assert_eq!(
        box_iframe(IFRAME_32X32, &threaded).unwrap_err(),
        Error::Threading
    );
    assert_eq!(Error::Threading.to_string(), "threading is off");
    let mut predicted = IFRAME_32X32.to_vec();
    predicted[29] = 0x41;
    assert_eq!(box_iframe(&predicted, &opts()).unwrap_err(), Error::Frame);
}

fn top_level(data: &[u8]) -> Vec<&str> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 8 <= data.len() {
        let size = u32::from_be_bytes(data[i..i + 4].try_into().unwrap()) as usize;
        assert!(size >= 8 && i + size <= data.len());
        out.push(std::str::from_utf8(&data[i + 4..i + 8]).unwrap());
        i += size;
    }
    assert_eq!(i, data.len());
    out
}

fn find(data: &[u8], kind: &[u8; 4]) -> usize {
    data.windows(4).position(|w| w == kind).unwrap()
}

fn count(data: &[u8], kind: &[u8; 4]) -> usize {
    data.windows(4).filter(|w| *w == kind).count()
}
