use export_translate::{lexicon_static_bytes, translate, translate_counts, Error};

const PARAGRAPH: &str = "Yesterday I talked to my friend about the meeting. She said we need to finish the report before Friday, but the train was late and we missed it. I think we can send the notes tomorrow.";

#[test]
fn ordinary_sentences() {
    let cases = [
        ("I talked to my friend.", "Yo hablé con mi amigo."),
        (
            "She said we need to finish the report.",
            "Ella dijo que nosotros necesitamos terminar el informe.",
        ),
        (
            "We can send the notes tomorrow.",
            "Nosotros podemos enviar las notas mañana.",
        ),
        ("The train was late.", "El tren estaba tarde."),
        ("We missed it.", "Nosotros lo perdimos."),
        ("The meeting is long.", "La reunión es larga."),
        ("He opened the file.", "Él abrió el archivo."),
        (
            "The children are talking outside.",
            "Los niños están hablando afuera.",
        ),
        ("We didn't miss the train.", "Nosotros no perdimos el tren."),
        ("There is a problem.", "Hay un problema."),
    ];
    let mut i = 0;
    while i < cases.len() {
        let (en, es) = cases[i];
        let got = translate(en, "en", "es").unwrap_or_else(|e| panic!("{en}: {e}"));
        assert_eq!(got, es, "{en}");
        i += 1;
    }
}

#[test]
fn function_words_do_not_echo() {
    for src in ["The.", "the and of"] {
        let err = translate(src, "en", "es").unwrap_err();
        assert!(
            matches!(err, Error::NothingTranslated { .. }),
            "{src}: {err}"
        );
        assert!(err.to_string().contains("not echoed"), "{err}");
    }
}

#[test]
fn unknown_is_copied_and_counted() {
    let (out, counts) = translate_counts("I talked to xyzzy.", "en", "es").unwrap();
    assert!(out.contains("hablé"), "{out}");
    assert!(out.contains("xyzzy"), "{out}");
    assert_ne!(out, "I talked to xyzzy.");
    assert!(counts.content_words >= 1, "{counts:?}");
    assert!(counts.unknown_copied >= 1, "{counts:?}");
}

#[test]
fn paragraph_is_not_a_fixture_echo() {
    let out = translate(PARAGRAPH, "en", "es").unwrap();
    assert_ne!(out, PARAGRAPH);
    for word in [
        "hablé", "amigo", "reunión", "dijo", "informe", "tren", "notas", "mañana",
    ] {
        assert!(out.contains(word), "{word} missing in {out}");
    }
    let back = translate(&out, "es", "en").unwrap();
    assert_ne!(back, out);
    assert!(lexicon_static_bytes() < 3_000_000);
    assert!(export_translate::frequent_lemma_count() > 400);
}
