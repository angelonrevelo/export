//! Left-to-right transfer for any clause that contains a frequent-lexicon token.
//! Closed entries in the same clause are rendered here too, so a mixed sentence
//! is not half-parsed by the fixture grammar.

use crate::esconj::{self, agree_adj, conj_into, copula, noun_pl, Pers as Ep, Ten};
use crate::kind::*;
use crate::lexicon::*;
use crate::tok::{TokE, TokS};

struct Agr {
    per: Per,
    gen: Gen,
    num: Num,
    subj: bool,
}

pub(crate) fn clause_en(toks: &[TokE<'_>]) -> String {
    let mut out = String::new();
    let mut buf = String::new();
    let mut i = 0;
    let mut agr = Agr {
        per: Per::He,
        gen: Gen::M,
        num: Num::Sg,
        subj: false,
    };
    while i < toks.len() {
        if is_end_en(&toks[i]) {
            i += 1;
            continue;
        }
        if matches!(toks[i], TokE::G(Eg::Please)) {
            push_es(&mut out, "por favor");
            i += 1;
            continue;
        }
        if is_there(&toks[i]) && i + 1 < toks.len() && is_be(&toks[i + 1]) {
            let past = matches!(toks[i + 1], TokE::BePast { .. });
            push_es(&mut out, if past { "había" } else { "hay" });
            i += 2;
            continue;
        }
        if let Some(per) = pronoun_en(&toks[i]) {
            if ends_prep(&out) && per == Per::You {
                push_es(&mut out, "ti");
            } else {
                push_es(&mut out, pron_es(per));
                agr.per = per;
                agr.subj = true;
                let (g, n) = pron_agr(per);
                agr.gen = g;
                agr.num = n;
            }
            i += 1;
            continue;
        }
        if let Some((np, n)) = take_np(toks, i) {
            let force = np.det.is_none() && np.poss.is_none() && ends_de_or_a(&out);
            render_np(&np, force, &mut out);
            agr.per = if np.plural { Per::They } else { Per::He };
            agr.gen = if np.gen_f { Gen::F } else { Gen::M };
            agr.num = if np.plural { Num::Pl } else { Num::Sg };
            agr.subj = true;
            i = n;
            continue;
        }
        if is_be(&toks[i]) {
            i = emit_be(toks, i, &agr, &mut out, &mut buf);
            continue;
        }
        if is_modal(&toks[i]) {
            i = emit_modal(toks, i, &agr, &mut out, &mut buf);
            continue;
        }
        if is_do(&toks[i]) {
            i = emit_do(toks, i, &agr, &mut out, &mut buf);
            continue;
        }
        if is_verb(&toks[i]) {
            i = emit_lex_verb(toks, i, &agr, &mut out, &mut buf);
            continue;
        }
        emit_other_en(&toks[i], &agr, &mut out, &mut buf);
        i += 1;
    }
    out
}

pub(crate) fn clause_es(toks: &[TokS<'_>]) -> String {
    let mut out = String::new();
    let mut i = 0;
    let mut held: Option<&str> = None;
    while i < toks.len() {
        if is_end_es(&toks[i]) {
            i += 1;
            continue;
        }
        if let Some(word) = clitic_en(&toks[i], toks.get(i + 1)) {
            held = Some(word);
            i += 1;
            continue;
        }
        if let TokS::G(Sg::Pron(en, per)) = toks[i] {
            push_word(&mut out, en);
            let _ = per;
            i += 1;
            continue;
        }
        if let Some(en) = poss_en(&toks[i]) {
            if let Some((noun, adjs, n)) = take_np_es(toks, i + 1) {
                push_word(&mut out, en);
                let mut k = 0;
                while k < adjs.len() {
                    push_word(&mut out, adjs[k]);
                    k += 1;
                }
                push_word(&mut out, &noun);
                i = n;
                continue;
            }
        }
        if is_det_es(&toks[i]) {
            if let Some((noun, adjs, n)) = take_np_es(toks, i + 1) {
                let art = det_en(&toks[i]);
                push_word(&mut out, art);
                let mut k = 0;
                while k < adjs.len() {
                    push_word(&mut out, adjs[k]);
                    k += 1;
                }
                push_word(&mut out, &noun);
                i = n;
                continue;
            }
        }
        if is_verb_es(&toks[i]) {
            i = emit_verb_es(toks, i, held, &mut out);
            held = None;
            continue;
        }
        if let Some(word) = gloss_es(&toks[i]) {
            push_word(&mut out, word);
        }
        i += 1;
    }
    if let Some(word) = held {
        push_word(&mut out, word);
    }
    out
}

struct Np {
    det: Option<DetK>,
    demo: Option<&'static str>,
    poss: Option<&'static str>,
    head: String,
    gen_f: bool,
    plural: bool,
    adjs: Vec<String>,
}

fn emit_be(toks: &[TokE<'_>], i: usize, agr: &Agr, out: &mut String, buf: &mut String) -> usize {
    let past = matches!(toks[i], TokE::BePast { .. });
    let were = matches!(toks[i], TokE::BePast { were: true });
    let per = if agr.subj {
        agr.per
    } else if were {
        Per::They
    } else {
        Per::He
    };
    let mut j = i + 1;
    let mut neg = false;
    if j < toks.len() && is_not(&toks[j]) {
        neg = true;
        j += 1;
    }
    if j < toks.len() && is_gerund(&toks[j]) {
        if neg {
            push_es(out, "no");
        }
        push_es(out, copula(true, past, ep(per)));
        gerund_into(&toks[j], buf);
        push_es(out, buf);
        return j + 1;
    }
    if j < toks.len() && is_adj(&toks[j]) {
        let estar = adj_estar(&toks[j]);
        if neg {
            push_es(out, "no");
        }
        push_es(out, copula(estar, past, ep(per)));
        let word = adj_es(&toks[j], agr.gen == Gen::F, agr.num == Num::Pl);
        push_es(out, &word);
        return j + 1;
    }
    if let Some((np, n)) = take_np(toks, j) {
        if neg {
            push_es(out, "no");
        }
        push_es(out, copula(false, past, ep(per)));
        render_np(&np, false, out);
        return n;
    }
    if neg {
        push_es(out, "no");
    }
    push_es(out, copula(true, past, ep(per)));
    j
}

fn emit_modal(toks: &[TokE<'_>], i: usize, agr: &Agr, out: &mut String, buf: &mut String) -> usize {
    let en = en_key(&toks[i]).unwrap_or("");
    let past = ten_of(&toks[i]) == TEN_PRET;
    let mut neg = neg_of(&toks[i]);
    let mut j = i + 1;
    if j < toks.len() && is_not(&toks[j]) {
        neg = true;
        j += 1;
    }
    if j < toks.len() && is_to(&toks[j]) && j + 1 < toks.len() && is_verb(&toks[j + 1]) {
        j += 1;
    }
    if j < toks.len() && is_verb(&toks[j]) {
        let mut n = j + 1;
        let clitic = take_clitic(toks, n);
        if let Some((_, k)) = clitic {
            n = k;
        }
        if neg {
            push_es(out, "no");
        }
        if let Some((c, _)) = clitic {
            push_es(out, c);
        }
        emit_aux(en, past, agr.per, out, buf);
        infinitive_into(&toks[j], buf);
        push_es(out, buf);
        return n;
    }
    if neg {
        push_es(out, "no");
    }
    emit_aux(en, past, agr.per, out, buf);
    j
}

fn emit_aux(en: &str, past: bool, per: Per, out: &mut String, buf: &mut String) {
    let p = ep(per);
    match en {
        "will" => {
            let ten = if past { Ten::Imperf } else { Ten::Pres };
            conj_into(buf, "ir", STEM_NONE, ten, p);
            push_es(out, buf);
            push_es(out, "a");
        }
        "should" => push_es(out, deberia(p)),
        "must" => {
            conj_into(buf, "tener", STEM_NONE, Ten::Pres, p);
            push_es(out, buf);
            push_es(out, "que");
        }
        _ => {
            let ten = if past { Ten::Imperf } else { Ten::Pres };
            conj_into(buf, "poder", STEM_NONE, ten, p);
            push_es(out, buf);
        }
    }
}

fn deberia(per: Ep) -> &'static str {
    match per {
        Ep::I => "debería",
        Ep::You => "deberías",
        Ep::He => "debería",
        Ep::We => "deberíamos",
        Ep::They => "deberían",
    }
}

fn emit_do(toks: &[TokE<'_>], i: usize, agr: &Agr, out: &mut String, buf: &mut String) -> usize {
    let mut neg = neg_of(&toks[i]);
    let aux_ten = ten_of(&toks[i]);
    let aux_s3 = s3_of(&toks[i]);
    let mut j = i + 1;
    if j < toks.len() && is_not(&toks[j]) {
        neg = true;
        j += 1;
    }
    if j < toks.len() && is_verb(&toks[j]) {
        let mut n = j + 1;
        let clitic = take_clitic(toks, n);
        if let Some((_, k)) = clitic {
            n = k;
        }
        if neg {
            push_es(out, "no");
        }
        if let Some((c, _)) = clitic {
            push_es(out, c);
        }
        let ten = if aux_ten == TEN_PRET {
            TEN_PRET
        } else {
            TEN_PRES
        };
        finite_into(&toks[j], agr, ten, aux_s3, buf);
        push_es(out, buf);
        return n;
    }
    // Main-verb "do": hacer.
    if neg {
        push_es(out, "no");
    }
    let ten = if aux_ten == TEN_PRET {
        Ten::Pret
    } else if !agr.subj && !aux_s3 {
        Ten::Imp
    } else {
        Ten::Pres
    };
    conj_into(buf, "hacer", STEM_NONE, ten, ep(agr.per));
    push_es(out, buf);
    j
}

fn emit_lex_verb(
    toks: &[TokE<'_>],
    i: usize,
    agr: &Agr,
    out: &mut String,
    buf: &mut String,
) -> usize {
    let en = en_key(&toks[i]).unwrap_or("");
    let j = i + 1;
    if matches!(en, "need" | "want" | "have")
        && j + 1 < toks.len()
        && is_to(&toks[j])
        && is_verb(&toks[j + 1])
    {
        let mut n = j + 2;
        let clitic = take_clitic(toks, n);
        if let Some((_, k)) = clitic {
            n = k;
        }
        if neg_of(&toks[i]) {
            push_es(out, "no");
        }
        if let Some((c, _)) = clitic {
            push_es(out, c);
        }
        finite_into(&toks[i], agr, ten_of(&toks[i]), s3_of(&toks[i]), buf);
        push_es(out, buf);
        if en == "have" {
            push_es(out, "que");
        }
        infinitive_into(&toks[j + 1], buf);
        push_es(out, buf);
        maybe_que(toks, n, en, out);
        return n;
    }
    let mut n = j;
    let clitic = if i + 1 < toks.len() && is_to(&toks[i + 1]) {
        None
    } else {
        take_clitic(toks, i + 1)
    };
    if let Some((_, k)) = clitic {
        n = k;
    }
    if neg_of(&toks[i]) {
        push_es(out, "no");
    }
    if let Some((c, _)) = clitic {
        push_es(out, c);
    }
    finite_into(&toks[i], agr, ten_of(&toks[i]), s3_of(&toks[i]), buf);
    push_es(out, buf);
    if matches!(en, "talk" | "speak")
        && n < toks.len()
        && is_to(&toks[n])
        && !verb_after_to(toks, n)
    {
        push_es(out, "con");
        n += 1;
    }
    maybe_que(toks, n, en, out);
    if n < toks.len() && is_en(&toks[n], "that") && take_np(toks, n).is_none() {
        n += 1;
    }
    n
}

fn verb_after_to(toks: &[TokE<'_>], to_at: usize) -> bool {
    to_at + 1 < toks.len() && is_verb(&toks[to_at + 1])
}

fn maybe_que(toks: &[TokE<'_>], i: usize, en: &str, out: &mut String) {
    if !matches!(en, "say" | "tell" | "think" | "know" | "believe") {
        return;
    }
    if i >= toks.len() {
        return;
    }
    if is_en(&toks[i], "that") && take_np(toks, i).is_none() {
        push_es(out, "que");
        return;
    }
    if pronoun_en(&toks[i]).is_some() {
        push_es(out, "que");
    }
}

fn take_np(toks: &[TokE<'_>], i: usize) -> Option<(Np, usize)> {
    if i >= toks.len() {
        return None;
    }
    let mut j = i;
    let mut det = None;
    let mut demo = None;
    let mut poss = None;
    if let Some(d) = det_kind(&toks[j]) {
        det = Some(d);
        j += 1;
    } else if let Some(en) = demo_en(&toks[j]) {
        demo = Some(en);
        j += 1;
    } else if let Some(en) = poss_en_tok(&toks[j]) {
        poss = Some(en);
        j += 1;
    }
    let mut adj_at = Vec::new();
    while j < toks.len() && is_adj(&toks[j]) {
        adj_at.push(j);
        j += 1;
    }
    if j >= toks.len() || !is_noun(&toks[j]) {
        return None;
    }
    let (head, gen_f, plural) = noun_es(&toks[j]);
    let mut adjs = Vec::new();
    let mut k = 0;
    while k < adj_at.len() {
        adjs.push(adj_es(&toks[adj_at[k]], gen_f, plural));
        k += 1;
    }
    Some((
        Np {
            det,
            demo,
            poss,
            head,
            gen_f,
            plural,
            adjs,
        },
        j + 1,
    ))
}

fn render_np(np: &Np, force_the: bool, out: &mut String) {
    if let Some(en) = np.poss {
        push_es(out, poss_es(en, np.gen_f, np.plural));
    } else if let Some(en) = np.demo {
        push_es(out, demo_es(en, np.gen_f, np.plural));
    } else if let Some(d) = np.det {
        let g = if np.gen_f { Gen::F } else { Gen::M };
        let n = if np.plural { Num::Pl } else { Num::Sg };
        push_es(out, det_es(d, g, n, &np.head));
    } else if force_the {
        let g = if np.gen_f { Gen::F } else { Gen::M };
        let n = if np.plural { Num::Pl } else { Num::Sg };
        push_es(out, det_es(DetK::The, g, n, &np.head));
    }
    push_es(out, &np.head);
    let mut k = 0;
    while k < np.adjs.len() {
        push_es(out, &np.adjs[k]);
        k += 1;
    }
}

fn emit_other_en(t: &TokE<'_>, agr: &Agr, out: &mut String, buf: &mut String) {
    if is_end_en(t) {
        return;
    }
    if let Some(en) = en_key(t) {
        if ends_prep(out) {
            if let Some(tonic) = tonic_es(en) {
                push_es(out, tonic);
                return;
            }
        }
        if let Some(word) = fn_es(en, t, agr) {
            push_es(out, word);
            return;
        }
    }
    match t {
        TokE::Mwe(s) => push_es(out, s),
        TokE::Unknown(w) => push_es(out, w),
        TokE::Punct(p) => push_es(out, p),
        TokE::F(f) => {
            if is_noun(t) {
                push_es(out, &noun_es(t).0);
            } else if is_adj(t) {
                let word = adj_es(t, agr.gen == Gen::F, agr.num == Num::Pl);
                push_es(out, &word);
            } else if is_verb(t) {
                finite_into(t, agr, f.ten, f.s3, buf);
                push_es(out, buf);
            } else {
                push_es(out, f.es);
            }
        }
        TokE::G(g) => {
            let w = match *g {
                Eg::Conj(s) | Eg::Prep(s) | Eg::Adv(s) | Eg::Interj(s) | Eg::Pron(s, _) => s,
                Eg::Please => "por favor",
                Eg::Not => "no",
                Eg::Be => "está",
                Eg::Det(DetK::The) => "el",
                Eg::Det(DetK::A) => "un",
                Eg::Noun { sg, pl, n, .. } => {
                    if n == Num::Pl {
                        pl
                    } else {
                        sg
                    }
                }
                Eg::Adj { m, .. } => m,
                Eg::Verb(p, form) => match form {
                    VForm::Ger => p.ger,
                    VForm::S3 | VForm::ImpOrS3 => p.s3,
                    VForm::Base => p.inf,
                },
            };
            push_es(out, w);
        }
        TokE::Cv(p, ten) => {
            finite_closed(p, agr, *ten, false, buf);
            push_es(out, buf);
        }
        TokE::BePast { were } => {
            let per = if agr.subj {
                agr.per
            } else if *were {
                Per::They
            } else {
                Per::He
            };
            push_es(out, copula(true, true, ep(per)));
        }
    }
}

fn fn_es<'a>(en: &str, t: &'a TokE<'a>, _agr: &Agr) -> Option<&'a str> {
    let es = es_field(t)?;
    Some(match en {
        "before" => "antes de",
        "after" => "después de",
        "without" => "sin",
        "according" => "según",
        _ => es,
    })
}

fn finite_into(t: &TokE<'_>, agr: &Agr, ten: u8, s3: bool, buf: &mut String) {
    match t {
        TokE::Cv(p, _) => finite_closed(p, agr, ten, s3, buf),
        TokE::G(Eg::Verb(p, _)) => finite_closed(p, agr, ten, s3, buf),
        TokE::F(f) => {
            let per = finite_per(agr, ten, s3);
            let et = if !agr.subj && ten != TEN_PRET && ten != TEN_GER && !s3 {
                Ten::Imp
            } else if ten == TEN_GER {
                Ten::Ger
            } else if ten == TEN_PRET {
                Ten::Pret
            } else {
                Ten::Pres
            };
            conj_into(buf, f.es, f.stem, et, ep(per));
        }
        _ => buf.clear(),
    }
}

fn finite_closed(p: &Parad, agr: &Agr, ten: u8, s3: bool, buf: &mut String) {
    let per = finite_per(agr, ten, s3);
    let stem = if p.en == "play" { STEM_UE } else { STEM_NONE };
    let et = if !agr.subj && ten != TEN_PRET && ten != TEN_GER && !s3 {
        Ten::Imp
    } else if ten == TEN_GER {
        Ten::Ger
    } else if ten == TEN_PRET {
        Ten::Pret
    } else {
        Ten::Pres
    };
    conj_into(buf, p.inf, stem, et, ep(per));
}

fn finite_per(agr: &Agr, ten: u8, s3: bool) -> Per {
    if agr.subj {
        agr.per
    } else if s3 || ten == TEN_PRET || ten == TEN_GER {
        Per::He
    } else {
        Per::You
    }
}

fn infinitive_into(t: &TokE<'_>, buf: &mut String) {
    match t {
        TokE::F(f) => conj_into(buf, f.es, f.stem, Ten::Inf, Ep::He),
        TokE::G(Eg::Verb(p, _)) | TokE::Cv(p, _) => {
            buf.clear();
            buf.push_str(p.inf);
        }
        _ => buf.clear(),
    }
}

fn gerund_into(t: &TokE<'_>, buf: &mut String) {
    match t {
        TokE::F(f) => conj_into(buf, f.es, f.stem, Ten::Ger, Ep::He),
        TokE::G(Eg::Verb(p, _)) | TokE::Cv(p, _) => {
            let stem = if p.en == "play" { STEM_UE } else { STEM_NONE };
            conj_into(buf, p.inf, stem, Ten::Ger, Ep::He);
        }
        _ => buf.clear(),
    }
}

fn noun_es(t: &TokE<'_>) -> (String, bool, bool) {
    match t {
        TokE::G(Eg::Noun { sg, pl, g, n }) => {
            let plural = *n == Num::Pl;
            let head = if plural {
                (*pl).to_string()
            } else {
                (*sg).to_string()
            };
            (head, *g == Gen::F, plural)
        }
        TokE::F(f) => {
            let plural = f.num == NUM_PL;
            let head = if plural {
                let explicit = if f.pl.is_empty() { None } else { Some(f.pl) };
                noun_pl(f.es, f.samepl, explicit)
            } else {
                f.es.to_string()
            };
            (head, f.kind == NF, plural)
        }
        _ => (String::new(), false, false),
    }
}

fn adj_es(t: &TokE<'_>, gen_f: bool, plural: bool) -> String {
    match t {
        TokE::G(Eg::Adj { m, f, mp, fp, .. }) => match (gen_f, plural) {
            (false, false) => (*m).to_string(),
            (true, false) => (*f).to_string(),
            (false, true) => (*mp).to_string(),
            (true, true) => (*fp).to_string(),
        },
        TokE::F(f) => agree_adj(f.es, f.kind, gen_f, plural),
        _ => String::new(),
    }
}

fn adj_estar(t: &TokE<'_>) -> bool {
    match t {
        TokE::G(Eg::Adj { cop, .. }) => *cop == Cop::Estar,
        TokE::F(f) => f.kind == AE || f.kind == AT,
        _ => false,
    }
}

fn take_clitic(toks: &[TokE<'_>], i: usize) -> Option<(&'static str, usize)> {
    if i >= toks.len() {
        return None;
    }
    if poss_en_tok(&toks[i]).is_some() && take_np(toks, i).is_some() {
        return None;
    }
    let en = en_key(&toks[i])?;
    let es = match en {
        "it" | "him" => "lo",
        "her" => "la",
        "them" => "los",
        "me" => "me",
        "us" => "nos",
        _ => return None,
    };
    Some((es, i + 1))
}

fn tonic_es(en: &str) -> Option<&'static str> {
    Some(match en {
        "me" => "mí",
        "him" => "él",
        "her" => "ella",
        "us" => "nosotros",
        "them" => "ellos",
        "it" => "ello",
        _ => return None,
    })
}

fn poss_es(en: &str, gen_f: bool, plural: bool) -> &'static str {
    match en {
        "my" => {
            if plural {
                "mis"
            } else {
                "mi"
            }
        }
        "your" => {
            if plural {
                "tus"
            } else {
                "tu"
            }
        }
        "our" => match (gen_f, plural) {
            (false, false) => "nuestro",
            (true, false) => "nuestra",
            (false, true) => "nuestros",
            (true, true) => "nuestras",
        },
        _ => {
            if plural {
                "sus"
            } else {
                "su"
            }
        }
    }
}

fn demo_es(en: &str, gen_f: bool, plural: bool) -> &'static str {
    match en {
        "this" | "these" => match (gen_f, plural || en == "these") {
            (false, false) => "este",
            (true, false) => "esta",
            (false, true) => "estos",
            (true, true) => "estas",
        },
        _ => match (gen_f, plural || en == "those") {
            (false, false) => "ese",
            (true, false) => "esa",
            (false, true) => "esos",
            (true, true) => "esas",
        },
    }
}

fn det_es(d: DetK, g: Gen, n: Num, noun: &str) -> &'static str {
    if n == Num::Sg && g == Gen::F && noun == "agua" {
        return if d == DetK::The { "el" } else { "un" };
    }
    match (d, g, n) {
        (DetK::The, Gen::M, Num::Sg) => "el",
        (DetK::The, Gen::F, Num::Sg) => "la",
        (DetK::The, Gen::M, Num::Pl) => "los",
        (DetK::The, Gen::F, Num::Pl) => "las",
        (DetK::A, Gen::M, Num::Sg) => "un",
        (DetK::A, Gen::F, Num::Sg) => "una",
        (DetK::A, Gen::M, Num::Pl) => "unos",
        (DetK::A, Gen::F, Num::Pl) => "unas",
    }
}

fn pron_es(per: Per) -> &'static str {
    match per {
        Per::I => "yo",
        Per::You => "tú",
        Per::He => "él",
        Per::She => "ella",
        Per::We => "nosotros",
        Per::They => "ellos",
    }
}

fn pron_agr(per: Per) -> (Gen, Num) {
    match per {
        Per::She => (Gen::F, Num::Sg),
        Per::We | Per::They => (Gen::M, Num::Pl),
        _ => (Gen::M, Num::Sg),
    }
}

fn ep(per: Per) -> Ep {
    match per {
        Per::I => Ep::I,
        Per::You => Ep::You,
        Per::We => Ep::We,
        Per::They => Ep::They,
        Per::He | Per::She => Ep::He,
    }
}

fn pronoun_en(t: &TokE<'_>) -> Option<Per> {
    match t {
        TokE::G(Eg::Pron(_, per)) => Some(*per),
        _ => None,
    }
}

fn det_kind(t: &TokE<'_>) -> Option<DetK> {
    match t {
        TokE::G(Eg::Det(d)) => Some(*d),
        _ => None,
    }
}

fn demo_en(t: &TokE<'_>) -> Option<&'static str> {
    match en_key(t)? {
        "this" | "that" | "these" | "those" => en_key(t),
        _ => None,
    }
}

fn poss_en_tok(t: &TokE<'_>) -> Option<&'static str> {
    match en_key(t)? {
        "my" | "your" | "his" | "her" | "our" | "their" | "its" => en_key(t),
        _ => None,
    }
}

fn is_noun(t: &TokE<'_>) -> bool {
    match t {
        TokE::G(Eg::Noun { .. }) => true,
        TokE::F(f) => is_noun_kind(f.kind),
        _ => false,
    }
}

fn is_noun_kind(kind: u8) -> bool {
    kind == NM || kind == NF
}

fn is_adj(t: &TokE<'_>) -> bool {
    match t {
        TokE::G(Eg::Adj { .. }) => true,
        TokE::F(f) => is_adj_kind(f.kind),
        _ => false,
    }
}

fn is_adj_kind(kind: u8) -> bool {
    matches!(kind, AO | AE | AI | AT)
}

fn is_verb(t: &TokE<'_>) -> bool {
    match t {
        TokE::G(Eg::Verb(_, _)) | TokE::Cv(_, _) => true,
        TokE::F(f) => is_verb_kind(f.kind),
        _ => false,
    }
}

fn is_verb_kind(kind: u8) -> bool {
    matches!(kind, VAR | VER | VIR | VI)
}

fn is_gerund(t: &TokE<'_>) -> bool {
    match t {
        TokE::G(Eg::Verb(_, VForm::Ger)) => true,
        TokE::F(f) => is_verb_kind(f.kind) && f.ten == TEN_GER,
        TokE::Cv(_, ten) => *ten == TEN_GER,
        _ => false,
    }
}

fn is_be(t: &TokE<'_>) -> bool {
    matches!(t, TokE::G(Eg::Be) | TokE::BePast { .. })
}

fn is_not(t: &TokE<'_>) -> bool {
    matches!(t, TokE::G(Eg::Not))
}

fn is_to(t: &TokE<'_>) -> bool {
    match t {
        TokE::G(Eg::Prep(p)) => *p == "a",
        _ => false,
    }
}

fn is_there(t: &TokE<'_>) -> bool {
    en_key(t) == Some("there")
}

fn is_modal(t: &TokE<'_>) -> bool {
    matches!(en_key(t), Some("can" | "will" | "should" | "must" | "may"))
}

fn is_do(t: &TokE<'_>) -> bool {
    en_key(t) == Some("do")
}

fn is_en(t: &TokE<'_>, en: &str) -> bool {
    en_key(t) == Some(en)
}

fn is_end_en(t: &TokE<'_>) -> bool {
    matches!(
        t,
        TokE::Punct("?")
            | TokE::Punct("!")
            | TokE::Punct(".")
            | TokE::Punct("¿")
            | TokE::Punct("¡")
    )
}

fn en_key(t: &TokE<'_>) -> Option<&'static str> {
    match t {
        TokE::F(f) => Some(f.en),
        TokE::G(Eg::Verb(p, _)) | TokE::Cv(p, _) => Some(p.en),
        _ => None,
    }
}

fn es_field<'a>(t: &'a TokE<'a>) -> Option<&'a str> {
    match t {
        TokE::F(f) => Some(f.es),
        _ => None,
    }
}

fn ten_of(t: &TokE<'_>) -> u8 {
    match t {
        TokE::F(f) => {
            if f.ten == TEN_NONE && is_verb_kind(f.kind) {
                TEN_PRES
            } else {
                f.ten
            }
        }
        TokE::Cv(_, ten) => *ten,
        TokE::G(Eg::Verb(_, VForm::Ger)) => TEN_GER,
        TokE::G(Eg::Verb(_, VForm::S3 | VForm::ImpOrS3 | VForm::Base)) => TEN_PRES,
        TokE::BePast { .. } => TEN_PRET,
        _ => TEN_NONE,
    }
}

fn s3_of(t: &TokE<'_>) -> bool {
    match t {
        TokE::F(f) => f.s3,
        TokE::G(Eg::Verb(_, VForm::S3 | VForm::ImpOrS3)) => true,
        _ => false,
    }
}

fn neg_of(t: &TokE<'_>) -> bool {
    match t {
        TokE::F(f) => f.neg,
        _ => false,
    }
}

fn ends_prep(out: &str) -> bool {
    out.ends_with(" con")
        || out.ends_with(" de")
        || out.ends_with(" a")
        || out.ends_with(" para")
        || out.ends_with(" por")
        || out.ends_with(" en")
        || out.ends_with(" sobre")
        || out.ends_with(" sin")
        || out == "con"
        || out == "de"
        || out == "a"
}

fn ends_de_or_a(out: &str) -> bool {
    out.ends_with(" de") || out.ends_with(" a") || out == "de" || out == "a"
}

fn push_word(out: &mut String, w: &str) {
    if w.is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with(' ') && !out.ends_with('¿') {
        out.push(' ');
    }
    out.push_str(w);
}

fn push_es(out: &mut String, w: &str) {
    if w == "el" {
        if out.ends_with(" a") {
            out.truncate(out.len() - 2);
            push_word(out, "al");
            return;
        }
        if out.ends_with(" de") {
            out.truncate(out.len() - 3);
            push_word(out, "del");
            return;
        }
        if out == "a" {
            out.clear();
            out.push_str("al");
            return;
        }
        if out == "de" {
            out.clear();
            out.push_str("del");
            return;
        }
    }
    push_word(out, w);
}

fn emit_verb_es(toks: &[TokS<'_>], i: usize, held: Option<&str>, out: &mut String) -> usize {
    let j = i + 1;
    if j < toks.len()
        && is_en_word_es(&toks[j], "a")
        && j + 1 < toks.len()
        && is_inf_es(&toks[j + 1])
    {
        let will = ten_es(&toks[i]) != TEN_PRET;
        push_word(out, if will { "will" } else { "went" });
        push_word(out, &en_base(&toks[j + 1]));
        if let Some(c) = held {
            push_word(out, c);
        }
        return j + 2;
    }
    if j < toks.len() && is_inf_es(&toks[j]) {
        let aux = en_base(&toks[i]);
        if aux == "can" || aux == "poder" {
            let word = if ten_es(&toks[i]) == TEN_IMPERF {
                "could"
            } else {
                "can"
            };
            push_word(out, word);
            push_word(out, &en_base(&toks[j]));
        } else if aux == "should" || aux == "deber" {
            push_word(out, "should");
            push_word(out, &en_base(&toks[j]));
        } else {
            push_word(out, &en_finite(&toks[i]));
            push_word(out, "to");
            push_word(out, &en_base(&toks[j]));
        }
        if let Some(c) = held {
            push_word(out, c);
        }
        return j + 1;
    }
    push_word(out, &en_finite(&toks[i]));
    if let Some(c) = held {
        push_word(out, c);
    }
    j
}

fn take_np_es(toks: &[TokS<'_>], i: usize) -> Option<(String, Vec<&'static str>, usize)> {
    if i >= toks.len() || !is_noun_es(&toks[i]) {
        return None;
    }
    let noun = en_noun(&toks[i]);
    let mut j = i + 1;
    let mut adjs = Vec::new();
    while j < toks.len() {
        if let Some(en) = adj_en(&toks[j]) {
            adjs.push(en);
            j += 1;
            continue;
        }
        break;
    }
    Some((noun, adjs, j))
}

fn en_noun(t: &TokS<'_>) -> String {
    match t {
        TokS::G(Sg::Noun { en, n }) => {
            if *n == Num::Pl {
                en_plural(en)
            } else {
                (*en).to_string()
            }
        }
        TokS::S(s) => {
            if s.num == NUM_PL {
                en_plural(s.en)
            } else {
                s.en.to_string()
            }
        }
        _ => String::new(),
    }
}

fn en_plural(en: &str) -> String {
    if en.ends_with("ch")
        || en.ends_with("sh")
        || en.ends_with('s')
        || en.ends_with('x')
        || en.ends_with('z')
    {
        format!("{en}es")
    } else if en.ends_with('y') && en.len() > 1 {
        let pre = &en[..en.len() - 1];
        let prev = pre.chars().last().unwrap_or('a');
        if !matches!(prev, 'a' | 'e' | 'i' | 'o' | 'u') {
            format!("{pre}ies")
        } else {
            format!("{en}s")
        }
    } else {
        format!("{en}s")
    }
}

fn en_base(t: &TokS<'_>) -> String {
    match t {
        TokS::S(s) => s.en.to_string(),
        TokS::G(Sg::Verb(p, _)) => p.en.to_string(),
        _ => String::new(),
    }
}

fn en_finite(t: &TokS<'_>) -> String {
    let base = en_base(t);
    if base.is_empty() {
        return base;
    }
    if matches!(base.as_str(), "can" | "will" | "should" | "must" | "may") {
        return base;
    }
    match ten_es(t) {
        TEN_PRET => esconj::en_past(&base),
        TEN_GER => esconj::en_ger(&base),
        TEN_INF | TEN_IMP => base,
        TEN_IMPERF if base == "be" => {
            if per_es(t) == PER_WE || per_es(t) == PER_THEY || per_es(t) == PER_YOU {
                "were".to_string()
            } else {
                "was".to_string()
            }
        }
        TEN_IMPERF if base == "can" => "could".to_string(),
        TEN_PRES if per_es(t) == PER_HE => esconj::en_s3(&base),
        _ => base,
    }
}

fn gloss_es<'a>(t: &'a TokS<'a>) -> Option<&'a str> {
    match t {
        TokS::Mwe(s) => Some(*s),
        TokS::Unknown(w) => Some(match *w {
            "del" => "of the",
            "al" => "to the",
            _ => w,
        }),
        TokS::Punct(p) => Some(*p),
        TokS::S(s) => Some(s.en),
        TokS::G(g) => Some(match g {
            Sg::Conj(s)
            | Sg::Prep(s)
            | Sg::Adv(s)
            | Sg::Interj(s)
            | Sg::Phrase(s)
            | Sg::Pron(s, _) => s,
            Sg::Not => "not",
            Sg::Det(DetK::The) => "the",
            Sg::Det(DetK::A) => "a",
            Sg::Be { per, .. } => en_be_word(*per),
            Sg::Noun { en, .. } | Sg::Adj { en, .. } => en,
            Sg::Verb(p, _) => p.en,
        }),
    }
}

fn en_be_word(per: Per) -> &'static str {
    match per {
        Per::I => "am",
        Per::You | Per::We | Per::They => "are",
        Per::He | Per::She => "is",
    }
}

fn clitic_en<'a>(t: &TokS<'a>, _next: Option<&TokS<'a>>) -> Option<&'static str> {
    let TokS::S(s) = t else {
        return None;
    };
    if matches!(s.es, "lo" | "la" | "le" | "les" | "me" | "te" | "nos") {
        Some(match s.es {
            "me" => "me",
            "te" => "you",
            "nos" => "us",
            "les" => "them",
            "la" => "her",
            _ => "it",
        })
    } else {
        None
    }
}

fn poss_en<'a>(t: &'a TokS<'a>) -> Option<&'a str> {
    if let TokS::S(s) = t {
        if matches!(
            s.en,
            "my" | "your" | "his" | "her" | "our" | "their" | "its"
        ) {
            return Some(s.en);
        }
    }
    None
}

fn is_det_es(t: &TokS<'_>) -> bool {
    matches!(t, TokS::G(Sg::Det(_)))
}

fn det_en<'a>(t: &'a TokS<'a>) -> &'a str {
    match t {
        TokS::G(Sg::Det(DetK::A)) => "a",
        _ => "the",
    }
}

fn is_noun_es(t: &TokS<'_>) -> bool {
    match t {
        TokS::G(Sg::Noun { .. }) => true,
        TokS::S(s) => s.kind == NM || s.kind == NF,
        _ => false,
    }
}

fn is_verb_es(t: &TokS<'_>) -> bool {
    match t {
        TokS::G(Sg::Verb(_, _)) => true,
        TokS::S(s) => {
            matches!(s.kind, VAR | VER | VIR | VI | AU)
                && matches!(
                    s.ten,
                    TEN_PRES | TEN_PRET | TEN_GER | TEN_INF | TEN_IMP | TEN_IMPERF
                )
        }
        _ => false,
    }
}

fn is_inf_es(t: &TokS<'_>) -> bool {
    match t {
        TokS::S(s) => s.ten == TEN_INF,
        TokS::G(Sg::Verb(_, VForm::Base)) => true,
        _ => false,
    }
}

fn is_en_word_es(t: &TokS<'_>, en: &str) -> bool {
    match t {
        TokS::G(Sg::Prep(p)) => *p == en || (*p == "to" && en == "a"),
        TokS::S(s) => s.en == en || s.es == en,
        TokS::Unknown(w) => *w == en,
        _ => false,
    }
}

fn adj_en(t: &TokS<'_>) -> Option<&'static str> {
    match t {
        TokS::G(Sg::Adj { en, .. }) => Some(*en),
        TokS::S(s) if matches!(s.kind, AO | AE | AI | AT) => Some(s.en),
        _ => None,
    }
}

fn ten_es(t: &TokS<'_>) -> u8 {
    match t {
        TokS::S(s) => s.ten,
        TokS::G(Sg::Verb(_, VForm::Ger)) => TEN_GER,
        _ => TEN_PRES,
    }
}

fn per_es(t: &TokS<'_>) -> u8 {
    match t {
        TokS::S(s) => s.per,
        _ => PER_HE,
    }
}

fn is_end_es(t: &TokS<'_>) -> bool {
    matches!(
        t,
        TokS::Punct("?")
            | TokS::Punct("!")
            | TokS::Punct(".")
            | TokS::Punct("¿")
            | TokS::Punct("¡")
    )
}
