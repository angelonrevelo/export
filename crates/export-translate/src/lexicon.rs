//! Closed lexicon. Keys are lowercase and sorted; lookup is a binary search we own.
//! Multi-word entries are a handful of fixed phrases, matched longest-first before
//! the unigram table. Nothing here is a full sentence.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Gen {
    M,
    F,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Num {
    Sg,
    Pl,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Per {
    I,
    You,
    He,
    She,
    We,
    They,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DetK {
    The,
    A,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Cop {
    Ser,
    Estar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VForm {
    Base,
    S3,
    Ger,
    /// Spanish surface that is both the tú imperative and the él/ella present.
    ImpOrS3,
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Parad {
    pub inf: &'static str,
    pub s1: &'static str,
    pub s2: &'static str,
    pub s3: &'static str,
    pub p1: &'static str,
    pub p3: &'static str,
    pub ger: &'static str,
    pub imp: &'static str,
    pub en: &'static str,
    pub en_s3: &'static str,
    pub en_ger: &'static str,
}

pub(crate) static OPEN: Parad = Parad {
    inf: "abrir",
    s1: "abro",
    s2: "abres",
    s3: "abre",
    p1: "abrimos",
    p3: "abren",
    ger: "abriendo",
    imp: "abre",
    en: "open",
    en_s3: "opens",
    en_ger: "opening",
};

pub(crate) static SAVE: Parad = Parad {
    inf: "guardar",
    s1: "guardo",
    s2: "guardas",
    s3: "guarda",
    p1: "guardamos",
    p3: "guardan",
    ger: "guardando",
    imp: "guarda",
    en: "save",
    en_s3: "saves",
    en_ger: "saving",
};

pub(crate) static PLAY: Parad = Parad {
    inf: "jugar",
    s1: "juego",
    s2: "juegas",
    s3: "juega",
    p1: "jugamos",
    p3: "juegan",
    ger: "jugando",
    imp: "juega",
    en: "play",
    en_s3: "plays",
    en_ger: "playing",
};

pub(crate) static WANT: Parad = Parad {
    inf: "querer",
    s1: "quiero",
    s2: "quieres",
    s3: "quiere",
    p1: "queremos",
    p3: "quieren",
    ger: "queriendo",
    imp: "quiere",
    en: "want",
    en_s3: "wants",
    en_ger: "wanting",
};

#[derive(Clone, Copy, Debug)]
pub(crate) enum Eg {
    Conj(&'static str),
    Prep(&'static str),
    Adv(&'static str),
    Pron(&'static str, Per),
    Det(DetK),
    Interj(&'static str),
    Please,
    Not,
    Be,
    Noun {
        sg: &'static str,
        pl: &'static str,
        g: Gen,
        n: Num,
    },
    Adj {
        m: &'static str,
        f: &'static str,
        mp: &'static str,
        fp: &'static str,
        cop: Cop,
    },
    Verb(&'static Parad, VForm),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EnEnt {
    pub key: &'static str,
    pub g: Eg,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Sg {
    Conj(&'static str),
    Prep(&'static str),
    Adv(&'static str),
    Pron(&'static str, Per),
    Det(DetK),
    Interj(&'static str),
    Phrase(&'static str),
    Not,
    Be { per: Per },
    Noun { en: &'static str, n: Num },
    Adj { en: &'static str },
    Verb(&'static Parad, VForm),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct EsEnt {
    pub key: &'static str,
    pub g: Sg,
}

pub(crate) struct Mwe {
    pub w: &'static [&'static str],
    pub out: &'static str,
}

pub(crate) const MWE_EN: &[Mwe] = &[
    Mwe {
        w: &["good", "morning"],
        out: "buenos días",
    },
    Mwe {
        w: &["how", "are", "you"],
        out: "cómo estás",
    },
    Mwe {
        w: &["thank", "you"],
        out: "gracias",
    },
];

pub(crate) const MWE_ES: &[Mwe] = &[
    Mwe {
        w: &["buenos", "días"],
        out: "good morning",
    },
    Mwe {
        w: &["cómo", "estás"],
        out: "how are you",
    },
    Mwe {
        w: &["por", "favor"],
        out: "please",
    },
];

pub(crate) const EN: &[EnEnt] = &[
    EnEnt {
        key: "a",
        g: Eg::Det(DetK::A),
    },
    EnEnt {
        key: "am",
        g: Eg::Be,
    },
    EnEnt {
        key: "an",
        g: Eg::Det(DetK::A),
    },
    EnEnt {
        key: "and",
        g: Eg::Conj("y"),
    },
    EnEnt {
        key: "are",
        g: Eg::Be,
    },
    EnEnt {
        key: "be",
        g: Eg::Be,
    },
    EnEnt {
        key: "big",
        g: Eg::Adj {
            m: "grande",
            f: "grande",
            mp: "grandes",
            fp: "grandes",
            cop: Cop::Ser,
        },
    },
    EnEnt {
        key: "but",
        g: Eg::Conj("pero"),
    },
    EnEnt {
        key: "child",
        g: Eg::Noun {
            sg: "niño",
            pl: "niños",
            g: Gen::M,
            n: Num::Sg,
        },
    },
    EnEnt {
        key: "children",
        g: Eg::Noun {
            sg: "niño",
            pl: "niños",
            g: Gen::M,
            n: Num::Pl,
        },
    },
    EnEnt {
        key: "document",
        g: Eg::Noun {
            sg: "documento",
            pl: "documentos",
            g: Gen::M,
            n: Num::Sg,
        },
    },
    EnEnt {
        key: "documents",
        g: Eg::Noun {
            sg: "documento",
            pl: "documentos",
            g: Gen::M,
            n: Num::Pl,
        },
    },
    EnEnt {
        key: "file",
        g: Eg::Noun {
            sg: "archivo",
            pl: "archivos",
            g: Gen::M,
            n: Num::Sg,
        },
    },
    EnEnt {
        key: "files",
        g: Eg::Noun {
            sg: "archivo",
            pl: "archivos",
            g: Gen::M,
            n: Num::Pl,
        },
    },
    EnEnt {
        key: "for",
        g: Eg::Prep("para"),
    },
    EnEnt {
        key: "he",
        g: Eg::Pron("él", Per::He),
    },
    EnEnt {
        key: "hello",
        g: Eg::Interj("hola"),
    },
    EnEnt {
        key: "house",
        g: Eg::Noun {
            sg: "casa",
            pl: "casas",
            g: Gen::F,
            n: Num::Sg,
        },
    },
    EnEnt {
        key: "houses",
        g: Eg::Noun {
            sg: "casa",
            pl: "casas",
            g: Gen::F,
            n: Num::Pl,
        },
    },
    EnEnt {
        key: "how",
        g: Eg::Adv("cómo"),
    },
    EnEnt {
        key: "i",
        g: Eg::Pron("yo", Per::I),
    },
    EnEnt {
        key: "in",
        g: Eg::Prep("en"),
    },
    EnEnt {
        key: "is",
        g: Eg::Be,
    },
    EnEnt {
        key: "new",
        g: Eg::Adj {
            m: "nuevo",
            f: "nueva",
            mp: "nuevos",
            fp: "nuevas",
            cop: Cop::Ser,
        },
    },
    EnEnt {
        key: "nice",
        g: Eg::Adj {
            m: "agradable",
            f: "agradable",
            mp: "agradables",
            fp: "agradables",
            cop: Cop::Estar,
        },
    },
    EnEnt {
        key: "not",
        g: Eg::Not,
    },
    EnEnt {
        key: "of",
        g: Eg::Prep("de"),
    },
    EnEnt {
        key: "open",
        g: Eg::Verb(&OPEN, VForm::Base),
    },
    EnEnt {
        key: "opening",
        g: Eg::Verb(&OPEN, VForm::Ger),
    },
    EnEnt {
        key: "opens",
        g: Eg::Verb(&OPEN, VForm::S3),
    },
    EnEnt {
        key: "or",
        g: Eg::Conj("o"),
    },
    EnEnt {
        key: "outside",
        g: Eg::Adv("afuera"),
    },
    EnEnt {
        key: "play",
        g: Eg::Verb(&PLAY, VForm::Base),
    },
    EnEnt {
        key: "playing",
        g: Eg::Verb(&PLAY, VForm::Ger),
    },
    EnEnt {
        key: "plays",
        g: Eg::Verb(&PLAY, VForm::S3),
    },
    EnEnt {
        key: "please",
        g: Eg::Please,
    },
    EnEnt {
        key: "save",
        g: Eg::Verb(&SAVE, VForm::Base),
    },
    EnEnt {
        key: "saves",
        g: Eg::Verb(&SAVE, VForm::S3),
    },
    EnEnt {
        key: "saving",
        g: Eg::Verb(&SAVE, VForm::Ger),
    },
    EnEnt {
        key: "she",
        g: Eg::Pron("ella", Per::She),
    },
    EnEnt {
        key: "small",
        g: Eg::Adj {
            m: "pequeño",
            f: "pequeña",
            mp: "pequeños",
            fp: "pequeñas",
            cop: Cop::Ser,
        },
    },
    EnEnt {
        key: "the",
        g: Eg::Det(DetK::The),
    },
    EnEnt {
        key: "they",
        g: Eg::Pron("ellos", Per::They),
    },
    EnEnt {
        key: "to",
        g: Eg::Prep("a"),
    },
    EnEnt {
        key: "today",
        g: Eg::Adv("hoy"),
    },
    EnEnt {
        key: "want",
        g: Eg::Verb(&WANT, VForm::Base),
    },
    EnEnt {
        key: "wanting",
        g: Eg::Verb(&WANT, VForm::Ger),
    },
    EnEnt {
        key: "wants",
        g: Eg::Verb(&WANT, VForm::S3),
    },
    EnEnt {
        key: "water",
        g: Eg::Noun {
            sg: "agua",
            pl: "aguas",
            g: Gen::F,
            n: Num::Sg,
        },
    },
    EnEnt {
        key: "we",
        g: Eg::Pron("nosotros", Per::We),
    },
    EnEnt {
        key: "weather",
        g: Eg::Noun {
            sg: "clima",
            pl: "climas",
            g: Gen::M,
            n: Num::Sg,
        },
    },
    EnEnt {
        key: "with",
        g: Eg::Prep("con"),
    },
    EnEnt {
        key: "you",
        g: Eg::Pron("tú", Per::You),
    },
];

pub(crate) const ES: &[EsEnt] = &[
    EsEnt {
        key: "a",
        g: Sg::Prep("to"),
    },
    EsEnt {
        key: "abre",
        g: Sg::Verb(&OPEN, VForm::ImpOrS3),
    },
    EsEnt {
        key: "abren",
        g: Sg::Verb(&OPEN, VForm::Base),
    },
    EsEnt {
        key: "abres",
        g: Sg::Verb(&OPEN, VForm::Base),
    },
    EsEnt {
        key: "abriendo",
        g: Sg::Verb(&OPEN, VForm::Ger),
    },
    EsEnt {
        key: "abrimos",
        g: Sg::Verb(&OPEN, VForm::Base),
    },
    EsEnt {
        key: "abrir",
        g: Sg::Verb(&OPEN, VForm::Base),
    },
    EsEnt {
        key: "abro",
        g: Sg::Verb(&OPEN, VForm::Base),
    },
    EsEnt {
        key: "afuera",
        g: Sg::Adv("outside"),
    },
    EsEnt {
        key: "agradable",
        g: Sg::Adj { en: "nice" },
    },
    EsEnt {
        key: "agradables",
        g: Sg::Adj { en: "nice" },
    },
    EsEnt {
        key: "agua",
        g: Sg::Noun {
            en: "water",
            n: Num::Sg,
        },
    },
    EsEnt {
        key: "aguas",
        g: Sg::Noun {
            en: "water",
            n: Num::Sg,
        },
    },
    EsEnt {
        key: "archivo",
        g: Sg::Noun {
            en: "file",
            n: Num::Sg,
        },
    },
    EsEnt {
        key: "archivos",
        g: Sg::Noun {
            en: "files",
            n: Num::Pl,
        },
    },
    EsEnt {
        key: "casa",
        g: Sg::Noun {
            en: "house",
            n: Num::Sg,
        },
    },
    EsEnt {
        key: "casas",
        g: Sg::Noun {
            en: "houses",
            n: Num::Pl,
        },
    },
    EsEnt {
        key: "clima",
        g: Sg::Noun {
            en: "weather",
            n: Num::Sg,
        },
    },
    EsEnt {
        key: "climas",
        g: Sg::Noun {
            en: "weather",
            n: Num::Sg,
        },
    },
    EsEnt {
        key: "con",
        g: Sg::Prep("with"),
    },
    EsEnt {
        key: "cómo",
        g: Sg::Adv("how"),
    },
    EsEnt {
        key: "de",
        g: Sg::Prep("of"),
    },
    EsEnt {
        key: "documento",
        g: Sg::Noun {
            en: "document",
            n: Num::Sg,
        },
    },
    EsEnt {
        key: "documentos",
        g: Sg::Noun {
            en: "documents",
            n: Num::Pl,
        },
    },
    EsEnt {
        key: "el",
        g: Sg::Det(DetK::The),
    },
    EsEnt {
        key: "ella",
        g: Sg::Pron("she", Per::She),
    },
    EsEnt {
        key: "ellos",
        g: Sg::Pron("they", Per::They),
    },
    EsEnt {
        key: "en",
        g: Sg::Prep("in"),
    },
    EsEnt {
        key: "eres",
        g: Sg::Be { per: Per::You },
    },
    EsEnt {
        key: "es",
        g: Sg::Be { per: Per::He },
    },
    EsEnt {
        key: "estamos",
        g: Sg::Be { per: Per::We },
    },
    EsEnt {
        key: "estoy",
        g: Sg::Be { per: Per::I },
    },
    EsEnt {
        key: "está",
        g: Sg::Be { per: Per::He },
    },
    EsEnt {
        key: "están",
        g: Sg::Be { per: Per::They },
    },
    EsEnt {
        key: "estás",
        g: Sg::Be { per: Per::You },
    },
    EsEnt {
        key: "gracias",
        g: Sg::Phrase("thank you"),
    },
    EsEnt {
        key: "grande",
        g: Sg::Adj { en: "big" },
    },
    EsEnt {
        key: "grandes",
        g: Sg::Adj { en: "big" },
    },
    EsEnt {
        key: "guarda",
        g: Sg::Verb(&SAVE, VForm::ImpOrS3),
    },
    EsEnt {
        key: "guardamos",
        g: Sg::Verb(&SAVE, VForm::Base),
    },
    EsEnt {
        key: "guardan",
        g: Sg::Verb(&SAVE, VForm::Base),
    },
    EsEnt {
        key: "guardando",
        g: Sg::Verb(&SAVE, VForm::Ger),
    },
    EsEnt {
        key: "guardar",
        g: Sg::Verb(&SAVE, VForm::Base),
    },
    EsEnt {
        key: "guardas",
        g: Sg::Verb(&SAVE, VForm::Base),
    },
    EsEnt {
        key: "guardo",
        g: Sg::Verb(&SAVE, VForm::Base),
    },
    EsEnt {
        key: "hola",
        g: Sg::Interj("hello"),
    },
    EsEnt {
        key: "hoy",
        g: Sg::Adv("today"),
    },
    EsEnt {
        key: "juega",
        g: Sg::Verb(&PLAY, VForm::ImpOrS3),
    },
    EsEnt {
        key: "juegan",
        g: Sg::Verb(&PLAY, VForm::Base),
    },
    EsEnt {
        key: "juegas",
        g: Sg::Verb(&PLAY, VForm::Base),
    },
    EsEnt {
        key: "juego",
        g: Sg::Verb(&PLAY, VForm::Base),
    },
    EsEnt {
        key: "jugamos",
        g: Sg::Verb(&PLAY, VForm::Base),
    },
    EsEnt {
        key: "jugando",
        g: Sg::Verb(&PLAY, VForm::Ger),
    },
    EsEnt {
        key: "jugar",
        g: Sg::Verb(&PLAY, VForm::Base),
    },
    EsEnt {
        key: "la",
        g: Sg::Det(DetK::The),
    },
    EsEnt {
        key: "las",
        g: Sg::Det(DetK::The),
    },
    EsEnt {
        key: "los",
        g: Sg::Det(DetK::The),
    },
    EsEnt {
        key: "niño",
        g: Sg::Noun {
            en: "child",
            n: Num::Sg,
        },
    },
    EsEnt {
        key: "niños",
        g: Sg::Noun {
            en: "children",
            n: Num::Pl,
        },
    },
    EsEnt {
        key: "no",
        g: Sg::Not,
    },
    EsEnt {
        key: "nosotros",
        g: Sg::Pron("we", Per::We),
    },
    EsEnt {
        key: "nueva",
        g: Sg::Adj { en: "new" },
    },
    EsEnt {
        key: "nuevas",
        g: Sg::Adj { en: "new" },
    },
    EsEnt {
        key: "nuevo",
        g: Sg::Adj { en: "new" },
    },
    EsEnt {
        key: "nuevos",
        g: Sg::Adj { en: "new" },
    },
    EsEnt {
        key: "o",
        g: Sg::Conj("or"),
    },
    EsEnt {
        key: "para",
        g: Sg::Prep("for"),
    },
    EsEnt {
        key: "pequeña",
        g: Sg::Adj { en: "small" },
    },
    EsEnt {
        key: "pequeñas",
        g: Sg::Adj { en: "small" },
    },
    EsEnt {
        key: "pequeño",
        g: Sg::Adj { en: "small" },
    },
    EsEnt {
        key: "pequeños",
        g: Sg::Adj { en: "small" },
    },
    EsEnt {
        key: "pero",
        g: Sg::Conj("but"),
    },
    EsEnt {
        key: "queremos",
        g: Sg::Verb(&WANT, VForm::Base),
    },
    EsEnt {
        key: "querer",
        g: Sg::Verb(&WANT, VForm::Base),
    },
    EsEnt {
        key: "queriendo",
        g: Sg::Verb(&WANT, VForm::Ger),
    },
    EsEnt {
        key: "quiere",
        g: Sg::Verb(&WANT, VForm::ImpOrS3),
    },
    EsEnt {
        key: "quieren",
        g: Sg::Verb(&WANT, VForm::Base),
    },
    EsEnt {
        key: "quieres",
        g: Sg::Verb(&WANT, VForm::Base),
    },
    EsEnt {
        key: "quiero",
        g: Sg::Verb(&WANT, VForm::Base),
    },
    EsEnt {
        key: "somos",
        g: Sg::Be { per: Per::We },
    },
    EsEnt {
        key: "son",
        g: Sg::Be { per: Per::They },
    },
    EsEnt {
        key: "soy",
        g: Sg::Be { per: Per::I },
    },
    EsEnt {
        key: "tú",
        g: Sg::Pron("you", Per::You),
    },
    EsEnt {
        key: "un",
        g: Sg::Det(DetK::A),
    },
    EsEnt {
        key: "una",
        g: Sg::Det(DetK::A),
    },
    EsEnt {
        key: "unas",
        g: Sg::Det(DetK::A),
    },
    EsEnt {
        key: "unos",
        g: Sg::Det(DetK::A),
    },
    EsEnt {
        key: "y",
        g: Sg::Conj("and"),
    },
    EsEnt {
        key: "yo",
        g: Sg::Pron("i", Per::I),
    },
    EsEnt {
        key: "él",
        g: Sg::Pron("he", Per::He),
    },
];

#[inline]
pub(crate) fn find_en(key: &str) -> Option<&'static EnEnt> {
    debug_assert!(table_sorted_en());
    find(EN, key, |e| e.key)
}

#[inline]
pub(crate) fn find_es(key: &str) -> Option<&'static EsEnt> {
    debug_assert!(table_sorted_es());
    find(ES, key, |e| e.key)
}

fn find<'a, T>(table: &'a [T], key: &str, k: fn(&T) -> &str) -> Option<&'a T> {
    let mut lo = 0usize;
    let mut hi = table.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if k(&table[mid]) < key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo < table.len() && k(&table[lo]) == key {
        Some(&table[lo])
    } else {
        None
    }
}

fn table_sorted_en() -> bool {
    let mut i = 1;
    while i < EN.len() {
        if EN[i - 1].key >= EN[i].key {
            return false;
        }
        i += 1;
    }
    true
}

fn table_sorted_es() -> bool {
    let mut i = 1;
    while i < ES.len() {
        if ES[i - 1].key >= ES[i].key {
            return false;
        }
        i += 1;
    }
    true
}

/// Bytes of the static tables plus the strings they point at.
/// Shared literals are counted once per reference, before the linker merges them.
pub fn lexicon_static_bytes() -> usize {
    let mut n = core::mem::size_of_val(EN)
        + core::mem::size_of_val(ES)
        + core::mem::size_of_val(MWE_EN)
        + core::mem::size_of_val(MWE_ES)
        + core::mem::size_of_val(&OPEN)
        + core::mem::size_of_val(&SAVE)
        + core::mem::size_of_val(&PLAY)
        + core::mem::size_of_val(&WANT);
    let mut i = 0;
    while i < EN.len() {
        n += EN[i].key.len() + eg_bytes(&EN[i].g);
        i += 1;
    }
    i = 0;
    while i < ES.len() {
        n += ES[i].key.len() + sg_bytes(&ES[i].g);
        i += 1;
    }
    i = 0;
    while i < MWE_EN.len() {
        n += mwe_bytes(&MWE_EN[i]);
        i += 1;
    }
    i = 0;
    while i < MWE_ES.len() {
        n += mwe_bytes(&MWE_ES[i]);
        i += 1;
    }
    n += parad_str(&OPEN) + parad_str(&SAVE) + parad_str(&PLAY) + parad_str(&WANT);
    n
}

fn mwe_bytes(m: &Mwe) -> usize {
    let mut n = m.out.len() + core::mem::size_of_val(m.w);
    let mut i = 0;
    while i < m.w.len() {
        n += m.w[i].len();
        i += 1;
    }
    n
}

fn parad_str(p: &Parad) -> usize {
    p.inf.len()
        + p.s1.len()
        + p.s2.len()
        + p.s3.len()
        + p.p1.len()
        + p.p3.len()
        + p.ger.len()
        + p.imp.len()
        + p.en.len()
        + p.en_s3.len()
        + p.en_ger.len()
}

fn eg_bytes(g: &Eg) -> usize {
    match *g {
        Eg::Conj(s) | Eg::Prep(s) | Eg::Adv(s) | Eg::Interj(s) | Eg::Pron(s, _) => s.len(),
        Eg::Noun { sg, pl, .. } => sg.len() + pl.len(),
        Eg::Adj { m, f, mp, fp, .. } => m.len() + f.len() + mp.len() + fp.len(),
        Eg::Det(_) | Eg::Please | Eg::Not | Eg::Be | Eg::Verb(_, _) => 0,
    }
}

fn sg_bytes(g: &Sg) -> usize {
    match *g {
        Sg::Conj(s) | Sg::Prep(s) | Sg::Adv(s) | Sg::Interj(s) | Sg::Phrase(s) | Sg::Pron(s, _) => {
            s.len()
        }
        Sg::Noun { en, .. } | Sg::Adj { en, .. } => en.len(),
        Sg::Det(_) | Sg::Not | Sg::Be { .. } | Sg::Verb(_, _) => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tables_are_sorted_unique() {
        let mut i = 1;
        while i < EN.len() {
            assert!(
                EN[i - 1].key < EN[i].key,
                "{} >= {}",
                EN[i - 1].key,
                EN[i].key
            );
            i += 1;
        }
        i = 1;
        while i < ES.len() {
            assert!(
                ES[i - 1].key < ES[i].key,
                "{} >= {}",
                ES[i - 1].key,
                ES[i].key
            );
            i += 1;
        }
    }

    #[test]
    fn phrases_are_short() {
        for m in MWE_EN.iter().chain(MWE_ES.iter()) {
            assert!(m.w.len() <= 3, "{}", m.out);
            assert!(!m.out.is_empty());
        }
        for e in EN {
            assert!(!e.key.contains(' '), "{}", e.key);
        }
        for e in ES {
            assert!(!e.key.contains(' '), "{}", e.key);
        }
    }
}
