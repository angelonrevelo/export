//! Tokens shared by the clause grammar and the frequent-word walk.

use crate::frequent::Surf;
use crate::kind::is_content;
use crate::lexicon::{Eg, Parad, Sg};
use crate::morph::FTok;

#[derive(Clone, Copy)]
pub(crate) enum TokE<'a> {
    Mwe(&'static str),
    Punct(&'a str),
    Unknown(&'a str),
    G(Eg),
    F(FTok),
    Cv(&'static Parad, u8),
    BePast { were: bool },
}

#[derive(Clone, Copy)]
pub(crate) enum TokS<'a> {
    Mwe(&'static str),
    Punct(&'a str),
    Unknown(&'a str),
    G(Sg),
    S(&'static Surf),
}

pub(crate) fn content_en(t: &TokE<'_>) -> bool {
    match t {
        TokE::Mwe(_) => true,
        TokE::Cv(_, _) => true,
        TokE::F(f) => is_content(f.kind),
        TokE::G(g) => match g {
            Eg::Noun { .. } | Eg::Verb(_, _) | Eg::Adj { .. } | Eg::Adv(_) | Eg::Interj(_) => true,
            Eg::Conj(_)
            | Eg::Prep(_)
            | Eg::Pron(_, _)
            | Eg::Det(_)
            | Eg::Please
            | Eg::Not
            | Eg::Be => false,
        },
        TokE::Unknown(_) | TokE::Punct(_) | TokE::BePast { .. } => false,
    }
}

pub(crate) fn content_es(t: &TokS<'_>) -> bool {
    match t {
        TokS::Mwe(_) => true,
        TokS::S(s) => is_content(s.kind),
        TokS::G(g) => match g {
            Sg::Noun { .. }
            | Sg::Verb(_, _)
            | Sg::Adj { .. }
            | Sg::Adv(_)
            | Sg::Interj(_)
            | Sg::Phrase(_) => true,
            Sg::Conj(_) | Sg::Prep(_) | Sg::Pron(_, _) | Sg::Det(_) | Sg::Not | Sg::Be { .. } => {
                false
            }
        },
        TokS::Unknown(_) | TokS::Punct(_) => false,
    }
}

pub(crate) fn needs_lex_en(toks: &[TokE<'_>]) -> bool {
    let mut i = 0;
    while i < toks.len() {
        if matches!(toks[i], TokE::F(_) | TokE::Cv(_, _) | TokE::BePast { .. }) {
            return true;
        }
        i += 1;
    }
    false
}

pub(crate) fn needs_lex_es(toks: &[TokS<'_>]) -> bool {
    let mut i = 0;
    while i < toks.len() {
        if matches!(toks[i], TokS::S(_)) {
            return true;
        }
        i += 1;
    }
    false
}
