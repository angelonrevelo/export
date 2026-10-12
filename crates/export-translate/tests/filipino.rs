//! Ordinary English ↔ Filipino sentences. Not the closed Spanish fixtures.

use export_translate::{translate, translate_counts, Error, PAIR_LIMIT};

#[test]
fn ordinary_sentences() {
    assert_eq!(
        translate("I talked to my friend.", "en", "tl").unwrap(),
        "Nakipag-usap ako sa kaibigan ko."
    );
    assert_eq!(
        translate("The train was late.", "en", "tl").unwrap(),
        "Huli ang tren."
    );
    assert_eq!(
        translate("We missed it.", "en", "tl").unwrap(),
        "Nakaligtaan natin ito."
    );
    assert_eq!(
        translate("We can send the notes tomorrow.", "en", "tl").unwrap(),
        "Maaari nating ipadala ang mga tala bukas."
    );
    assert_eq!(
        translate("We didn't miss the train.", "en", "tl").unwrap(),
        "Hindi nakaligtaan natin ang tren."
    );
    assert_eq!(
        translate("The children are playing outside.", "en", "tl").unwrap(),
        "Naglalaro ang mga bata sa labas."
    );
    assert_eq!(
        translate("She wants water.", "en", "tl").unwrap(),
        "Gusto niya ng tubig."
    );
    assert_eq!(
        translate("The house is new.", "en", "tl").unwrap(),
        "Bago ang bahay."
    );
}

#[test]
fn paragraph_is_filipino() {
    let src = "Yesterday the team finished the report about the meeting. I will call my friend tomorrow and send the notes. She said the train was late, but we can meet at the office before Friday.";
    let out = translate(src, "en", "tl").unwrap();
    assert_ne!(out, src);
    assert!(out.contains("tinapos"), "{out}");
    assert!(out.contains("koponan"), "{out}");
    assert!(out.contains("ulat"), "{out}");
    assert!(out.contains("pulong"), "{out}");
    assert!(out.contains("Tatawag"), "{out}");
    assert!(out.contains("kaibigan"), "{out}");
    assert!(out.contains("ipadala"), "{out}");
    assert!(out.contains("tala"), "{out}");
    assert!(out.contains("Sinabi"), "{out}");
    assert!(out.contains("huli"), "{out}");
    assert!(out.contains("tren"), "{out}");
    assert!(out.contains("magkita"), "{out}");
    assert!(out.contains("opisina"), "{out}");
    assert!(out.contains("Biyernes"), "{out}");
    let back = translate(&out, "tl", "en").unwrap();
    assert_ne!(back, out);
    assert!(
        back.to_lowercase().contains("report") || back.to_lowercase().contains("train"),
        "{back}"
    );
}

#[test]
fn copies_unknown_only_after_content() {
    let (out, counts) = translate_counts("I talked to xyzzy.", "en", "tl").unwrap();
    assert!(out.contains("Nakipag-usap"), "{out}");
    assert!(out.contains("xyzzy"), "{out}");
    assert!(counts.content_words >= 1);
    assert!(counts.unknown_copied >= 1);
    assert_ne!(out, "I talked to xyzzy.");

    for src in ["The.", "the and of", "Xyzzy plugh"] {
        let err = translate(src, "en", "tl").unwrap_err();
        assert!(matches!(err, Error::NothingTranslated { .. }), "{err}");
        assert!(err.to_string().contains("not echoed"), "{err}");
        assert!(!err.to_string().contains(src));
    }
}

#[test]
fn other_codes_still_fail() {
    assert!(PAIR_LIMIT.contains("en → es"));
    assert!(PAIR_LIMIT.contains("not NLLB"));
    assert!(PAIR_LIMIT.contains("tl"));
    let err = translate("The house is new.", "fil", "tl").unwrap_err();
    assert!(matches!(err, Error::Unsupported { .. }));
    assert!(err.to_string().contains("en ↔ es"));
    assert!(export_translate::tl_lemma_count() > 200);
    assert!(export_translate::lexicon_static_bytes() < 3_000_000);
}
