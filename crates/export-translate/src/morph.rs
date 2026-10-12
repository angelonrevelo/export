//! English inflection into the frequent lexicon.
//! Exact closed-lexicon hits are resolved by the caller before this runs, so the
//! fixture grammar stays on the forms it already spells out.

use crate::frequent::{find_lemma, Lem};
use crate::kind::*;
use crate::lexicon::{find_en, Eg, Parad};

#[derive(Clone, Copy)]
pub(crate) struct FTok {
    pub en: &'static str,
    pub es: &'static str,
    pub kind: u8,
    pub stem: u8,
    pub samepl: bool,
    pub pl: &'static str,
    pub ten: u8,
    pub num: u8,
    pub s3: bool,
    pub neg: bool,
}

pub(crate) enum Hit {
    F(FTok),
    Cv(&'static Parad, u8),
    Be { were: bool },
}

struct Irr {
    surf: &'static str,
    lem: &'static str,
    ten: u8,
    s3: bool,
    neg: bool,
    were: bool,
}

pub(crate) fn lookup(key: &str) -> Option<Hit> {
    if let Some(row) = find_irr(key) {
        if let Some(hit) = realize(row.lem, row.ten, NUM_SG, row.s3, row.neg, row.were) {
            return Some(hit);
        }
    }
    if let Some(hit) = exact(key) {
        return Some(hit);
    }
    inflect(key)
}

fn exact(key: &str) -> Option<Hit> {
    let lem = find_lemma(key)?;
    let ten = if is_verb(lem.kind) {
        TEN_PRES
    } else {
        TEN_NONE
    };
    Some(Hit::F(ftok(lem, ten, NUM_SG, false, false)))
}

fn realize(lem: &str, ten: u8, num: u8, s3: bool, neg: bool, were: bool) -> Option<Hit> {
    if lem.is_empty() || lem == "be" {
        return Some(Hit::Be { were });
    }
    if let Some(e) = find_en(lem) {
        if let Eg::Verb(p, _) = e.g {
            return Some(Hit::Cv(p, ten));
        }
    }
    let row = find_lemma(lem)?;
    Some(Hit::F(ftok(row, ten, num, s3, neg)))
}

fn ftok(lem: &Lem, ten: u8, num: u8, s3: bool, neg: bool) -> FTok {
    FTok {
        en: lem.en,
        es: lem.es,
        kind: lem.kind,
        stem: lem.stem,
        samepl: lem.samepl,
        pl: lem.pl,
        ten,
        num,
        s3,
        neg,
    }
}

fn inflect(key: &str) -> Option<Hit> {
    if let Some(hit) = strip_ied(key) {
        return Some(hit);
    }
    if let Some(hit) = strip_ed(key) {
        return Some(hit);
    }
    if let Some(hit) = strip_ing(key) {
        return Some(hit);
    }
    if let Some(hit) = strip_ies(key) {
        return Some(hit);
    }
    strip_s(key)
}

fn strip_ied(key: &str) -> Option<Hit> {
    let stem = key.strip_suffix("ied")?;
    if stem.len() < 2 {
        return None;
    }
    let mut y = String::with_capacity(stem.len() + 1);
    y.push_str(stem);
    y.push('y');
    verb_hit(&y, TEN_PRET, false)
}

fn strip_ed(key: &str) -> Option<Hit> {
    let stem = key.strip_suffix("ed")?;
    if stem.len() < 2 {
        return None;
    }
    if let Some(hit) = verb_hit(stem, TEN_PRET, false) {
        return Some(hit);
    }
    if let Some(hit) = undouble(stem, TEN_PRET) {
        return Some(hit);
    }
    let mut e = String::with_capacity(stem.len() + 1);
    e.push_str(stem);
    e.push('e');
    verb_hit(&e, TEN_PRET, false)
}

fn strip_ing(key: &str) -> Option<Hit> {
    let stem = key.strip_suffix("ing")?;
    if stem.len() < 2 {
        return None;
    }
    if let Some(hit) = verb_hit(stem, TEN_GER, false) {
        return Some(hit);
    }
    if let Some(hit) = undouble(stem, TEN_GER) {
        return Some(hit);
    }
    if stem.ends_with('y') && stem.len() > 2 {
        let mut lie = String::with_capacity(stem.len() + 1);
        lie.push_str(&stem[..stem.len() - 1]);
        lie.push_str("ie");
        if let Some(hit) = verb_hit(&lie, TEN_GER, false) {
            return Some(hit);
        }
    }
    let mut e = String::with_capacity(stem.len() + 1);
    e.push_str(stem);
    e.push('e');
    verb_hit(&e, TEN_GER, false)
}

fn strip_ies(key: &str) -> Option<Hit> {
    let stem = key.strip_suffix("ies")?;
    if stem.len() < 2 {
        return None;
    }
    let mut y = String::with_capacity(stem.len() + 1);
    y.push_str(stem);
    y.push('y');
    if let Some(hit) = verb_hit(&y, TEN_PRES, true) {
        return Some(hit);
    }
    noun_hit(&y)
}

fn strip_s(key: &str) -> Option<Hit> {
    if key.ends_with("ss") {
        return None;
    }
    let mut stems: [&str; 2] = ["", ""];
    let mut n = 0;
    let mut owned = String::new();
    if let Some(stem) = key.strip_suffix("es") {
        if stem.len() >= 2 {
            stems[n] = stem;
            n += 1;
        }
    }
    if let Some(stem) = key.strip_suffix('s') {
        if stem.len() >= 2 && (n == 0 || stems[0] != stem) {
            stems[n] = stem;
            n += 1;
        }
    }
    let mut i = 0;
    while i < n {
        if let Some(hit) = verb_hit(stems[i], TEN_PRES, true) {
            return Some(hit);
        }
        i += 1;
    }
    i = 0;
    while i < n {
        if let Some(hit) = noun_hit(stems[i]) {
            return Some(hit);
        }
        i += 1;
    }
    // notes → note, where "not" was tried first via -es.
    if key.ends_with("es") {
        if let Some(stem) = key.strip_suffix('s') {
            owned.push_str(stem);
            if let Some(hit) = noun_hit(&owned) {
                return Some(hit);
            }
        }
    }
    None
}

fn undouble(stem: &str, ten: u8) -> Option<Hit> {
    let mut chars = stem.chars();
    let last = chars.next_back()?;
    let prev = chars.next_back()?;
    if last != prev || !last.is_ascii_alphabetic() {
        return None;
    }
    let bare = &stem[..stem.len() - last.len_utf8()];
    verb_hit(bare, ten, false)
}

fn verb_hit(stem: &str, ten: u8, s3: bool) -> Option<Hit> {
    if let Some(e) = find_en(stem) {
        if let Eg::Verb(p, _) = e.g {
            return Some(Hit::Cv(p, ten));
        }
    }
    let lem = find_lemma(stem)?;
    if !is_verb(lem.kind) && lem.kind != AU {
        return None;
    }
    Some(Hit::F(ftok(lem, ten, NUM_SG, s3, false)))
}

fn noun_hit(stem: &str) -> Option<Hit> {
    if let Some(e) = find_en(stem) {
        if let Eg::Noun { .. } = e.g {
            // Closed plurals are exact entries. A stripped plural of a closed
            // singular still has to stay on that noun, not the frequent table.
            return None;
        }
    }
    let lem = find_lemma(stem)?;
    if !is_noun(lem.kind) {
        return None;
    }
    Some(Hit::F(ftok(lem, TEN_NONE, NUM_PL, false, false)))
}

fn find_irr(key: &str) -> Option<&'static Irr> {
    let mut lo = 0usize;
    let mut hi = IRR.len();
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        if IRR[mid].surf < key {
            lo = mid + 1;
        } else {
            hi = mid;
        }
    }
    if lo < IRR.len() && IRR[lo].surf == key {
        Some(&IRR[lo])
    } else {
        None
    }
}

const IRR: &[Irr] = &[
    Irr {
        surf: "ate",
        lem: "eat",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "been",
        lem: "be",
        ten: TEN_GER,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "bought",
        lem: "buy",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "brought",
        lem: "bring",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "came",
        lem: "come",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "can't",
        lem: "can",
        ten: TEN_PRES,
        s3: false,
        neg: true,
        were: false,
    },
    Irr {
        surf: "cannot",
        lem: "can",
        ten: TEN_PRES,
        s3: false,
        neg: true,
        were: false,
    },
    Irr {
        surf: "could",
        lem: "can",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "couldn't",
        lem: "can",
        ten: TEN_PRET,
        s3: false,
        neg: true,
        were: false,
    },
    Irr {
        surf: "did",
        lem: "do",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "didn't",
        lem: "do",
        ten: TEN_PRET,
        s3: false,
        neg: true,
        were: false,
    },
    Irr {
        surf: "does",
        lem: "do",
        ten: TEN_PRES,
        s3: true,
        neg: false,
        were: false,
    },
    Irr {
        surf: "doesn't",
        lem: "do",
        ten: TEN_PRES,
        s3: true,
        neg: true,
        were: false,
    },
    Irr {
        surf: "don't",
        lem: "do",
        ten: TEN_PRES,
        s3: false,
        neg: true,
        were: false,
    },
    Irr {
        surf: "done",
        lem: "do",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "drank",
        lem: "drink",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "drove",
        lem: "drive",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "fell",
        lem: "fall",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "felt",
        lem: "feel",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "forgot",
        lem: "forget",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "found",
        lem: "find",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "gave",
        lem: "give",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "given",
        lem: "give",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "gone",
        lem: "go",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "got",
        lem: "get",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "had",
        lem: "have",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "has",
        lem: "have",
        ten: TEN_PRES,
        s3: true,
        neg: false,
        were: false,
    },
    Irr {
        surf: "hasn't",
        lem: "have",
        ten: TEN_PRES,
        s3: true,
        neg: true,
        were: false,
    },
    Irr {
        surf: "haven't",
        lem: "have",
        ten: TEN_PRES,
        s3: false,
        neg: true,
        were: false,
    },
    Irr {
        surf: "heard",
        lem: "hear",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "held",
        lem: "hold",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "kept",
        lem: "keep",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "knew",
        lem: "know",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "known",
        lem: "know",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "led",
        lem: "lead",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "left",
        lem: "leave",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "lost",
        lem: "lose",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "made",
        lem: "make",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "meant",
        lem: "mean",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "met",
        lem: "meet",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "paid",
        lem: "pay",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "ran",
        lem: "run",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "said",
        lem: "say",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "saw",
        lem: "see",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "seen",
        lem: "see",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "sent",
        lem: "send",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "sold",
        lem: "sell",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "spent",
        lem: "spend",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "spoke",
        lem: "speak",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "stood",
        lem: "stand",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "taken",
        lem: "take",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "taught",
        lem: "teach",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "thought",
        lem: "think",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "told",
        lem: "tell",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "took",
        lem: "take",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "understood",
        lem: "understand",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "was",
        lem: "be",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "went",
        lem: "go",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "were",
        lem: "be",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: true,
    },
    Irr {
        surf: "woke",
        lem: "wake",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "won",
        lem: "win",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "won't",
        lem: "will",
        ten: TEN_PRES,
        s3: false,
        neg: true,
        were: false,
    },
    Irr {
        surf: "wore",
        lem: "wear",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "would",
        lem: "will",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "wouldn't",
        lem: "will",
        ten: TEN_PRET,
        s3: false,
        neg: true,
        were: false,
    },
    Irr {
        surf: "written",
        lem: "write",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
    Irr {
        surf: "wrote",
        lem: "write",
        ten: TEN_PRET,
        s3: false,
        neg: false,
        were: false,
    },
];

#[cfg(test)]
mod tests {
    use super::IRR;

    #[test]
    fn irregulars_are_sorted() {
        let mut i = 1;
        while i < IRR.len() {
            assert!(
                IRR[i - 1].surf < IRR[i].surf,
                "{} >= {}",
                IRR[i - 1].surf,
                IRR[i].surf
            );
            i += 1;
        }
    }
}
