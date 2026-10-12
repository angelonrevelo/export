use export_translate::{translate, translate_batch, Error, PAIR_LIMIT};

const EN_ES: &[(&str, &str)] = &[
    (
        "The weather is nice today and the children are playing outside.",
        "El clima está agradable hoy y los niños están jugando afuera.",
    ),
    (
        "Please open the file and save the document.",
        "Por favor abre el archivo y guarda el documento.",
    ),
    ("Hello, how are you today?", "Hola, ¿cómo estás hoy?"),
    ("She wants water.", "Ella quiere agua."),
    ("The small house is new.", "La casa pequeña es nueva."),
    ("The children want water.", "Los niños quieren agua."),
    ("The big house is new.", "La casa grande es nueva."),
    ("I want water.", "Yo quiero agua."),
    (
        "We are playing outside.",
        "Nosotros estamos jugando afuera.",
    ),
    ("The house is not new.", "La casa no es nueva."),
    (
        "The children are playing in the house.",
        "Los niños están jugando en la casa.",
    ),
    (
        "The small houses are new.",
        "Las casas pequeñas son nuevas.",
    ),
    ("The child is small.", "El niño es pequeño."),
    ("Open the file.", "Abre el archivo."),
    ("He opens the file.", "Él abre el archivo."),
    ("She saves the document.", "Ella guarda el documento."),
    ("How are you today?", "¿Cómo estás hoy?"),
    ("Thank you.", "Gracias."),
    ("Good morning.", "Buenos días."),
    ("She wants the water.", "Ella quiere el agua."),
    (
        "She wants water. The house is new.",
        "Ella quiere agua. La casa es nueva.",
    ),
];

#[test]
fn en_es_fixtures_and_roundtrip() {
    let mut i = 0;
    while i < EN_ES.len() {
        let (en, es) = EN_ES[i];
        let got = translate(en, "en", "es").unwrap_or_else(|e| panic!("{en}: {e}"));
        assert_eq!(got, es, "en→es: {en}");
        assert_ne!(got, en);
        let back = translate(&got, "es", "en").unwrap_or_else(|e| panic!("{es}: {e}"));
        assert_eq!(back, en, "es→en: {got}");
        i += 1;
    }
}

#[test]
fn batch_matches_single() {
    let texts: Vec<&str> = EN_ES.iter().map(|(en, _)| *en).collect();
    let batch = translate_batch(&texts, "en", "es").unwrap();
    let mut i = 0;
    while i < texts.len() {
        assert_eq!(batch[i], translate(texts[i], "en", "es").unwrap());
        i += 1;
    }
}

#[test]
fn unsupported_pairs_fail_clearly() {
    let cases = [
        ("en", "fr"),
        ("es", "ja"),
        ("es", "tl"),
        ("fil", "en"),
        ("zh", "es"),
        ("tl", "fr"),
    ];
    let mut i = 0;
    while i < cases.len() {
        let (src, tgt) = cases[i];
        let err = translate("The house is new.", src, tgt).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("unsupported"), "{msg}");
        assert!(msg.contains(src), "{msg}");
        assert!(msg.contains("en ↔ es"), "{msg}");
        assert!(matches!(err, Error::Unsupported { .. }));
        i += 1;
    }
    assert_eq!(translate("Kumusta ka", "tl", "tl").unwrap(), "Kumusta ka");
    let err = translate("The house is new.", "fil", "en").unwrap_err();
    assert_eq!(
        err.to_string(),
        "unsupported language pair 'fil' → 'en'; this crate translates en ↔ es only"
    );
}

#[test]
fn does_not_echo_unknown_or_source() {
    let src = "The weather is nice today and the children are playing outside.";
    let out = translate(src, "en", "es").unwrap();
    assert!(!out.to_lowercase().contains("children"));
    assert!(!out.to_lowercase().contains("playing"));
    assert!(out.contains("niños"));
    assert!(out.contains("jugando"));

    let err = translate("Xyzzy plugh", "en", "es").unwrap_err();
    assert!(matches!(err, Error::NothingTranslated { .. }), "{err}");
    assert!(err.to_string().contains("not echoed"));
}

#[test]
fn same_language_is_passthrough() {
    let s = "The house is new.";
    assert_eq!(translate(s, "en", "en").unwrap(), s);
    assert_eq!(translate(s, "es", "es").unwrap(), s);
    assert_eq!(translate("Hola", "spa", "es-MX").unwrap(), "Hola");
}

#[test]
fn aliases_and_empty() {
    assert_eq!(
        translate("She wants water.", "eng", "spa").unwrap(),
        "Ella quiere agua."
    );
    assert_eq!(translate("", "en", "es").unwrap(), "");
    assert_eq!(translate("...", "en", "es").unwrap(), "...");
}

#[test]
fn limit_is_written_down() {
    assert!(PAIR_LIMIT.contains("en → es"));
    assert!(PAIR_LIMIT.contains("not NLLB"));
    assert!(PAIR_LIMIT.contains("tl"));
}
