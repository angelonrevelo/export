//! English ↔ Filipino clause walk.
//!
//! A content word has to translate before an unknown token is copied. Function
//! words, pronouns, determiners, and auxiliaries do not count. Hyphens are glued
//! only here; the shared scanner still treats `-` as punctuation.

use crate::esconj;
use crate::fold::fold_into;
use crate::kind::{TEN_GER, TEN_PRES, TEN_PRET};
use crate::morph;
use crate::scan::{tokenize, Raw};
use crate::tlfreq::{
    find_lemma, find_surf, Lem, ASP_CONT, ASP_IMPF, ASP_INF, ASP_PERF, K_ADJ, K_ADV, K_AU, K_AUX,
    K_CONJ, K_COP, K_CW, K_DET, K_FN, K_IRR, K_MAG, K_N, K_PL, K_POSS, K_PREP, K_PRO, K_PSEUDO,
    K_UM, SUB_A, SUB_DO, SUB_NOT, SUB_THAT, SUB_WILL,
};
use crate::transfer::Meta;

pub(crate) fn en_to_tl(text: &str) -> Result<String, &'static str> {
    en_to_tl_meta(text).map(|(s, _)| s)
}

pub(crate) fn tl_to_en(text: &str) -> Result<String, &'static str> {
    tl_to_en_meta(text).map(|(s, _)| s)
}

pub(crate) fn en_to_tl_meta(text: &str) -> Result<(String, Meta), &'static str> {
    let toks = analyze_en(text);
    let (content, unknown) = tally(&toks);
    if content == 0 {
        return Err("none");
    }
    Ok((render_sentence(&toks), Meta { content, unknown }))
}

pub(crate) fn tl_to_en_meta(text: &str) -> Result<(String, Meta), &'static str> {
    let items = tl_items(text);
    let mut content = 0u32;
    let mut unknown = 0u32;
    let mut out = String::new();
    let mut plural_next = false;
    let mut i = 0;
    while i < items.len() {
        match &items[i] {
            Item::Comma => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(',');
                }
                i += 1;
                continue;
            }
            Item::Word(w) => {
                let mut key = String::new();
                fold_into(w, &mut key);
                if key == "sa" && i + 1 < items.len() {
                    if let Item::Word(next) = &items[i + 1] {
                        let mut nkey = String::new();
                        fold_into(next, &mut nkey);
                        if nkey == "palagay" {
                            push_word(&mut out, "think");
                            content += 1;
                            i += 2;
                            plural_next = false;
                            continue;
                        }
                    }
                }
                if key == "tungkol" {
                    push_word(&mut out, "about");
                    if i + 1 < items.len() {
                        if let Item::Word(next) = &items[i + 1] {
                            let mut nkey = String::new();
                            fold_into(next, &mut nkey);
                            if nkey == "sa" {
                                i += 2;
                                plural_next = false;
                                continue;
                            }
                        }
                    }
                    i += 1;
                    plural_next = false;
                    continue;
                }
                if key == "palagay" {
                    push_word(&mut out, "think");
                    content += 1;
                    i += 1;
                    plural_next = false;
                    continue;
                }
                if let Some(surf) = find_flex(&key) {
                    if surf.kind == K_PL || surf.en.is_empty() {
                        plural_next = true;
                        i += 1;
                        continue;
                    }
                    if is_content_kind(surf.kind) {
                        content += 1;
                    }
                    let word = en_word(surf.en, surf.kind, surf.asp, plural_next);
                    plural_next = false;
                    if surf.asp == ASP_CONT {
                        push_word(&mut out, "will");
                    }
                    push_word(&mut out, &word);
                } else {
                    unknown += 1;
                    push_word(&mut out, w);
                    plural_next = false;
                }
                i += 1;
            }
        }
    }
    if content == 0 {
        return Err("none");
    }
    Ok((out, Meta { content, unknown }))
}

fn en_word(en: &str, kind: u8, asp: u8, plural: bool) -> String {
    if kind == K_N && plural {
        return en_plural(en);
    }
    match asp {
        ASP_PERF => esconj::en_past(en),
        ASP_IMPF => esconj::en_ger(en),
        _ => en.to_string(),
    }
}

fn en_plural(lem: &str) -> String {
    match lem {
        "child" => "children".to_string(),
        "person" => "people".to_string(),
        "man" => "men".to_string(),
        "woman" => "women".to_string(),
        _ => esconj::en_s3(lem),
    }
}

fn find_flex(key: &str) -> Option<&'static crate::tlfreq::Surf> {
    if let Some(s) = find_surf(key) {
        return Some(s);
    }
    if let Some(stem) = key.strip_suffix("ng") {
        if stem.len() >= 2 {
            if let Some(s) = find_surf(stem) {
                return Some(s);
            }
        }
    }
    if let Some(stem) = key.strip_suffix('g') {
        if stem.len() >= 2 && stem.ends_with('n') {
            if let Some(s) = find_surf(stem) {
                return Some(s);
            }
        }
    }
    None
}

enum Item {
    Word(String),
    Comma,
}

fn tl_items(text: &str) -> Vec<Item> {
    let raw = tokenize(text);
    let mut out = Vec::new();
    let mut i = 0;
    while i < raw.len() {
        match raw[i] {
            Raw::Word(w) => {
                if i + 2 < raw.len() {
                    if let Raw::Punct("-") = raw[i + 1] {
                        if let Raw::Word(w2) = raw[i + 2] {
                            let mut s = String::new();
                            s.push_str(w);
                            s.push('-');
                            s.push_str(w2);
                            out.push(Item::Word(s));
                            i += 3;
                            continue;
                        }
                    }
                }
                out.push(Item::Word(w.to_string()));
                i += 1;
            }
            Raw::Punct(",") => {
                out.push(Item::Comma);
                i += 1;
            }
            _ => i += 1,
        }
    }
    out
}

struct Hit {
    lem: &'static Lem,
    ten: u8,
    neg: bool,
    plural: bool,
}

struct Tok<'a> {
    lem: Option<&'static Lem>,
    ten: u8,
    neg: bool,
    plural: bool,
    raw: &'a str,
    comma: bool,
}

fn analyze_en(text: &str) -> Vec<Tok<'_>> {
    let raws = tokenize(text);
    let mut key = String::new();
    let mut out = Vec::new();
    for raw in raws {
        match raw {
            Raw::Punct(",") => out.push(Tok {
                lem: None,
                ten: 0,
                neg: false,
                plural: false,
                raw: ",",
                comma: true,
            }),
            Raw::Punct(_) => {}
            Raw::Word(w) => {
                fold_into(w, &mut key);
                if let Some(hit) = lookup(&key) {
                    out.push(Tok {
                        lem: Some(hit.lem),
                        ten: hit.ten,
                        neg: hit.neg,
                        plural: hit.plural,
                        raw: w,
                        comma: false,
                    });
                } else {
                    out.push(Tok {
                        lem: None,
                        ten: 0,
                        neg: false,
                        plural: false,
                        raw: w,
                        comma: false,
                    });
                }
            }
        }
    }
    out
}

fn lookup(key: &str) -> Option<Hit> {
    if let Some(irr) = morph::irregular(key) {
        if let Some(lem) = find_lemma(irr.lem) {
            return Some(Hit {
                lem,
                ten: irr.ten,
                neg: irr.neg,
                plural: false,
            });
        }
    }
    if let Some(lem) = find_lemma(key) {
        let ten = if inflectable(lem.kind) { TEN_PRES } else { 0 };
        return Some(Hit {
            lem,
            ten,
            neg: false,
            plural: lem.plural,
        });
    }
    inflect(key)
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
    verb_at(&y, TEN_PRET)
}

fn strip_ed(key: &str) -> Option<Hit> {
    let stem = key.strip_suffix("ed")?;
    if stem.len() < 2 {
        return None;
    }
    if let Some(hit) = verb_at(stem, TEN_PRET) {
        return Some(hit);
    }
    if let Some(hit) = undouble(stem, TEN_PRET) {
        return Some(hit);
    }
    let mut e = String::with_capacity(stem.len() + 1);
    e.push_str(stem);
    e.push('e');
    verb_at(&e, TEN_PRET)
}

fn strip_ing(key: &str) -> Option<Hit> {
    let stem = key.strip_suffix("ing")?;
    if stem.len() < 2 {
        return None;
    }
    if let Some(hit) = verb_at(stem, TEN_GER) {
        return Some(hit);
    }
    if let Some(hit) = undouble(stem, TEN_GER) {
        return Some(hit);
    }
    if stem.ends_with('y') && stem.len() > 1 {
        let mut lie = String::with_capacity(stem.len() + 1);
        lie.push_str(&stem[..stem.len() - 1]);
        lie.push_str("ie");
        if let Some(hit) = verb_at(&lie, TEN_GER) {
            return Some(hit);
        }
    }
    let mut e = String::with_capacity(stem.len() + 1);
    e.push_str(stem);
    e.push('e');
    verb_at(&e, TEN_GER)
}

fn strip_ies(key: &str) -> Option<Hit> {
    let stem = key.strip_suffix("ies")?;
    if stem.len() < 2 {
        return None;
    }
    let mut y = String::with_capacity(stem.len() + 1);
    y.push_str(stem);
    y.push('y');
    if let Some(hit) = verb_at(&y, TEN_PRES) {
        return Some(hit);
    }
    noun_pl(&y)
}

fn strip_s(key: &str) -> Option<Hit> {
    if key.ends_with("ss") {
        return None;
    }
    let mut stems: [&str; 2] = ["", ""];
    let mut n = 0;
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
        if let Some(hit) = verb_at(stems[i], TEN_PRES) {
            return Some(hit);
        }
        i += 1;
    }
    i = 0;
    while i < n {
        if let Some(hit) = noun_pl(stems[i]) {
            return Some(hit);
        }
        i += 1;
    }
    None
}

fn undouble(stem: &str, ten: u8) -> Option<Hit> {
    let b = stem.as_bytes();
    if b.len() >= 3 && b[b.len() - 1] == b[b.len() - 2] {
        verb_at(&stem[..stem.len() - 1], ten)
    } else {
        None
    }
}

fn verb_at(stem: &str, ten: u8) -> Option<Hit> {
    let lem = find_lemma(stem)?;
    if !inflectable(lem.kind) {
        return None;
    }
    Some(Hit {
        lem,
        ten,
        neg: false,
        plural: false,
    })
}

fn noun_pl(stem: &str) -> Option<Hit> {
    let lem = find_lemma(stem)?;
    if lem.kind != K_N {
        return None;
    }
    Some(Hit {
        lem,
        ten: 0,
        neg: false,
        plural: true,
    })
}

fn tally(toks: &[Tok<'_>]) -> (u32, u32) {
    let mut content = 0u32;
    let mut unknown = 0u32;
    for t in toks {
        if t.comma {
            continue;
        }
        if let Some(lem) = t.lem {
            if is_content_kind(lem.kind) {
                content += 1;
            }
        } else if has_letter(t.raw) {
            unknown += 1;
        }
    }
    (content, unknown)
}

fn is_content_kind(kind: u8) -> bool {
    matches!(
        kind,
        K_N | K_MAG | K_UM | K_IRR | K_PSEUDO | K_ADJ | K_ADV | K_CW
    )
}

fn inflectable(kind: u8) -> bool {
    matches!(kind, K_MAG | K_UM | K_IRR | K_PSEUDO | K_CW)
}

fn has_letter(s: &str) -> bool {
    for ch in s.chars() {
        if ch.is_alphabetic() {
            return true;
        }
    }
    false
}

#[derive(Clone, Copy)]
enum Case {
    Ang,
    Ng,
    Bare,
}

struct Np<'a> {
    pron: Option<&'a Tok<'a>>,
    det: Option<&'a Tok<'a>>,
    poss: Option<&'a Tok<'a>>,
    adjs: Vec<&'a Tok<'a>>,
    head: Option<&'a Tok<'a>>,
    unknown: Option<&'a str>,
    plural: bool,
}

impl Np<'_> {
    fn empty() -> Self {
        Np {
            pron: None,
            det: None,
            poss: None,
            adjs: Vec::new(),
            head: None,
            unknown: None,
            plural: false,
        }
    }
}

struct Pp<'a> {
    prep: &'a Tok<'a>,
    np: Np<'a>,
}

struct Clause<'a> {
    front: Vec<&'a Tok<'a>>,
    subj: Np<'a>,
    will: bool,
    neg: bool,
    do_pret: bool,
    existential: bool,
    pseudo: Option<&'a Tok<'a>>,
    modal: Option<&'a Tok<'a>>,
    cop: bool,
    think: bool,
    verb: Option<&'a Tok<'a>>,
    pred_adj: Option<&'a Tok<'a>>,
    pred_np: Np<'a>,
    obj: Np<'a>,
    pps: Vec<Pp<'a>>,
    advs: Vec<&'a Tok<'a>>,
    comp: &'a [Tok<'a>],
    loose: Vec<&'a Tok<'a>>,
}

fn render_sentence(toks: &[Tok<'_>]) -> String {
    let segs = segments(toks);
    let mut out = String::new();
    for (conj, comma, slice) in segs {
        if slice.is_empty() {
            continue;
        }
        if !out.is_empty() {
            if comma {
                out.push_str(", ");
            } else {
                out.push(' ');
            }
            if let Some(c) = conj {
                if let Some(lem) = c.lem {
                    push_word(&mut out, lem.tl);
                    out.push(' ');
                }
            }
        }
        push_word(&mut out, &render_clause(slice));
    }
    out
}

fn segments<'a>(toks: &'a [Tok<'a>]) -> Vec<(Option<&'a Tok<'a>>, bool, &'a [Tok<'a>])> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    let mut conj_for_next: Option<&Tok<'_>> = None;
    let mut comma_for_next = false;
    while i < toks.len() {
        if is_conj(&toks[i]) {
            let comma = i > 0 && toks[i - 1].comma;
            let end = if comma { i - 1 } else { i };
            out.push((conj_for_next, comma_for_next, trim_edges(&toks[start..end])));
            conj_for_next = Some(&toks[i]);
            comma_for_next = comma;
            start = i + 1;
        }
        i += 1;
    }
    out.push((conj_for_next, comma_for_next, trim_edges(&toks[start..])));
    out
}

fn trim_edges<'a>(toks: &'a [Tok<'a>]) -> &'a [Tok<'a>] {
    let mut a = 0;
    let mut b = toks.len();
    while a < b && toks[a].comma {
        a += 1;
    }
    while b > a && toks[b - 1].comma {
        b -= 1;
    }
    &toks[a..b]
}

fn render_clause(toks: &[Tok<'_>]) -> String {
    if toks.is_empty() {
        return String::new();
    }
    let c = parse_clause(toks);
    if c.existential {
        return render_exist(&c);
    }
    if c.think {
        return render_think(&c);
    }
    if c.verb.and_then(|v| v.lem).map(|l| l.comp).unwrap_or(false) {
        return render_comp_verb(&c);
    }
    if let Some(p) = c.pseudo {
        if let Some(v) = c.verb {
            return render_modal(&c, lem_tl(p), v);
        }
        return render_pseudo_np(&c);
    }
    if let Some(m) = c.modal {
        if let Some(v) = c.verb {
            return render_modal(&c, lem_tl(m), v);
        }
    }
    if c.cop && c.verb.is_none() {
        return render_cop(&c);
    }
    if c.verb.is_some() {
        return render_finite(&c);
    }
    render_loose_clause(&c)
}

fn parse_clause<'a>(toks: &'a [Tok<'a>]) -> Clause<'a> {
    let n = toks.len();
    let mut i = 0;
    let mut c = Clause {
        front: Vec::new(),
        subj: Np::empty(),
        will: false,
        neg: false,
        do_pret: false,
        existential: false,
        pseudo: None,
        modal: None,
        cop: false,
        think: false,
        verb: None,
        pred_adj: None,
        pred_np: Np::empty(),
        obj: Np::empty(),
        pps: Vec::new(),
        advs: Vec::new(),
        comp: &[],
        loose: Vec::new(),
    };
    while i < n {
        if toks[i].comma || is_skippable_fn(&toks[i]) {
            i += 1;
            continue;
        }
        if is_adv(&toks[i]) {
            if en_is(&toks[i], "there") && i + 1 < n && is_cop(&toks[i + 1]) {
                c.existential = true;
                i += 1;
                continue;
            }
            c.front.push(&toks[i]);
            i += 1;
            continue;
        }
        break;
    }
    if i < n && looks_like_np(toks, i) && !is_main_verb(&toks[i]) {
        let (np, j) = parse_np(toks, i);
        if j > i {
            c.subj = np;
            i = j;
        }
    }
    while i < n {
        let t = &toks[i];
        if t.comma || is_skippable_fn(t) {
            i += 1;
            continue;
        }
        if is_adv(t) {
            c.advs.push(t);
            i += 1;
            continue;
        }
        if is_will(t) {
            c.will = true;
            c.neg |= t.neg;
            i += 1;
            continue;
        }
        if is_do(t) {
            c.do_pret = t.ten == TEN_PRET;
            c.neg |= t.neg;
            i += 1;
            continue;
        }
        if is_not(t) {
            c.neg = true;
            i += 1;
            continue;
        }
        if is_au(t) {
            c.modal = Some(t);
            c.neg |= t.neg;
            i += 1;
            continue;
        }
        if is_need_want(t) {
            c.pseudo = Some(t);
            c.neg |= t.neg;
            i += 1;
            continue;
        }
        if is_inf_to(t) && (c.modal.is_some() || c.pseudo.is_some()) {
            i += 1;
            continue;
        }
        if is_cop(t) {
            c.cop = true;
            i += 1;
            continue;
        }
        break;
    }
    if c.cop && i < n && toks[i].ten == TEN_GER && is_main_verb(&toks[i]) {
        c.cop = false;
        c.verb = Some(&toks[i]);
        i += 1;
    }
    if c.verb.is_none() && i < n && is_think(&toks[i]) {
        c.think = true;
        i += 1;
        c.comp = skip_that(&toks[i..]);
        return c;
    }
    if c.verb.is_none() && i < n && is_main_verb(&toks[i]) {
        c.verb = Some(&toks[i]);
        let comp = c.verb.and_then(|v| v.lem).map(|l| l.comp).unwrap_or(false);
        i += 1;
        if comp {
            c.comp = skip_that(&toks[i..]);
            return c;
        }
    }
    if c.cop && c.verb.is_none() {
        if i < n && is_adj(&toks[i]) {
            c.pred_adj = Some(&toks[i]);
            i += 1;
        } else if i < n && looks_like_np(toks, i) {
            let (np, j) = parse_np(toks, i);
            c.pred_np = np;
            i = j;
        }
    }
    while i < n {
        if toks[i].comma || is_skippable_fn(&toks[i]) {
            i += 1;
            continue;
        }
        if is_prep(&toks[i]) {
            let prep = &toks[i];
            i += 1;
            if i < n && looks_like_np(toks, i) {
                let (np, j) = parse_np(toks, i);
                c.pps.push(Pp { prep, np });
                i = j;
            } else {
                c.pps.push(Pp {
                    prep,
                    np: Np::empty(),
                });
            }
            continue;
        }
        if is_adv(&toks[i]) {
            c.advs.push(&toks[i]);
            i += 1;
            continue;
        }
        if np_empty(&c.obj) && looks_like_np(toks, i) {
            let (np, j) = parse_np(toks, i);
            if j > i {
                c.obj = np;
                i = j;
                continue;
            }
        }
        if toks[i].lem.is_none() || is_content_tok(&toks[i]) {
            c.loose.push(&toks[i]);
        }
        i += 1;
    }
    c
}

fn skip_that<'a>(toks: &'a [Tok<'a>]) -> &'a [Tok<'a>] {
    if !toks.is_empty() && is_that(&toks[0]) {
        &toks[1..]
    } else {
        toks
    }
}

fn parse_np<'a>(toks: &'a [Tok<'a>], mut i: usize) -> (Np<'a>, usize) {
    let mut np = Np::empty();
    if i >= toks.len() {
        return (np, i);
    }
    if is_poss(&toks[i]) || her_poss(toks, i) {
        np.poss = Some(&toks[i]);
        i += 1;
    } else if is_det(&toks[i]) {
        np.det = Some(&toks[i]);
        np.plural = toks[i].plural;
        i += 1;
    } else if is_pro(&toks[i]) {
        np.pron = Some(&toks[i]);
        return (np, i + 1);
    } else if toks[i].lem.is_none() && !toks[i].comma {
        np.unknown = Some(toks[i].raw);
        return (np, i + 1);
    }
    while i < toks.len() && is_adj(&toks[i]) {
        np.adjs.push(&toks[i]);
        i += 1;
    }
    if i < toks.len() && is_noun(&toks[i]) {
        np.plural = np.plural || toks[i].plural;
        np.head = Some(&toks[i]);
        i += 1;
    } else if i < toks.len() && toks[i].lem.is_none() && !toks[i].comma {
        np.unknown = Some(toks[i].raw);
        i += 1;
    }
    (np, i)
}

fn her_poss(toks: &[Tok<'_>], i: usize) -> bool {
    if !en_is(&toks[i], "her") || i + 1 >= toks.len() {
        return false;
    }
    is_noun(&toks[i + 1]) || is_adj(&toks[i + 1]) || toks[i + 1].lem.is_none()
}

fn looks_like_np(toks: &[Tok<'_>], i: usize) -> bool {
    let t = &toks[i];
    if t.comma {
        return false;
    }
    if t.lem.is_none() {
        return has_letter(t.raw);
    }
    let kind = t.lem.unwrap().kind;
    if matches!(kind, K_PRO | K_POSS | K_DET | K_N) {
        return true;
    }
    if kind == K_ADJ {
        let mut j = i + 1;
        while j < toks.len() && is_adj(&toks[j]) {
            j += 1;
        }
        return j < toks.len() && (is_noun(&toks[j]) || toks[j].lem.is_none());
    }
    false
}

fn render_exist(c: &Clause<'_>) -> String {
    let mut s = String::new();
    push_toks(&mut s, &c.front);
    if c.neg {
        push_word(&mut s, "hindi");
    }
    push_word(&mut s, "may");
    if let Some(adj) = c.pred_adj {
        push_word(&mut s, lem_tl(adj));
    } else if !np_empty(&c.pred_np) {
        push_word(&mut s, &render_np(&c.pred_np, Case::Bare, true));
    } else if !np_empty(&c.subj) {
        push_word(&mut s, &render_np(&c.subj, Case::Bare, true));
    }
    render_pps(&mut s, &c.pps);
    push_toks(&mut s, &c.advs);
    push_loose(&mut s, &c.loose);
    s
}

fn render_think(c: &Clause<'_>) -> String {
    let mut s = String::new();
    push_toks(&mut s, &c.front);
    if c.neg {
        push_word(&mut s, "hindi");
    }
    push_word(&mut s, "sa palagay");
    push_actor_gen(&mut s, &c.subj);
    if !c.comp.is_empty() {
        push_word(&mut s, "na");
        push_word(&mut s, &render_clause(c.comp));
    }
    render_pps(&mut s, &c.pps);
    push_toks(&mut s, &c.advs);
    push_loose(&mut s, &c.loose);
    s
}

fn render_comp_verb(c: &Clause<'_>) -> String {
    let Some(verb) = c.verb else {
        return String::new();
    };
    let mut s = String::new();
    push_toks(&mut s, &c.front);
    if c.neg {
        push_word(&mut s, "hindi");
    }
    let asp = aspect(verb, c.will, c.do_pret, false, !np_empty(&c.subj));
    push_word(&mut s, surface_for(verb, asp));
    push_actor(&mut s, verb, &c.subj);
    if !c.comp.is_empty() {
        push_word(&mut s, "na");
        push_word(&mut s, &render_clause(c.comp));
    }
    render_pps(&mut s, &c.pps);
    push_toks(&mut s, &c.advs);
    push_loose(&mut s, &c.loose);
    s
}

fn render_modal(c: &Clause<'_>, word: &str, verb: &Tok<'_>) -> String {
    let focus_ng = verb.lem.map(|l| l.focus_ng).unwrap_or(false);
    let mut s = String::new();
    push_toks(&mut s, &c.front);
    if c.neg {
        push_word(&mut s, "hindi");
    }
    if let Some(p) = c.subj.pron {
        push_word(&mut s, word);
        let base = if focus_ng { lem_gen(p) } else { lem_ang(p) };
        push_word(&mut s, &linker(base));
    } else {
        push_word(&mut s, &linker(word));
    }
    push_word(&mut s, surface_for(verb, ASP_INF));
    if c.subj.pron.is_none() && !np_empty(&c.subj) {
        let case = if focus_ng { Case::Ng } else { Case::Ang };
        push_word(&mut s, &render_np(&c.subj, case, false));
    }
    if !np_empty(&c.obj) {
        push_word(&mut s, &render_obj(verb, &c.obj));
    }
    render_pps(&mut s, &c.pps);
    push_toks(&mut s, &c.advs);
    push_loose(&mut s, &c.loose);
    s
}

fn render_pseudo_np(c: &Clause<'_>) -> String {
    let Some(p) = c.pseudo else {
        return String::new();
    };
    let mut s = String::new();
    push_toks(&mut s, &c.front);
    if c.neg {
        push_word(&mut s, "hindi");
    }
    push_word(&mut s, lem_tl(p));
    push_actor_gen(&mut s, &c.subj);
    if !np_empty(&c.obj) {
        push_word(&mut s, &render_np(&c.obj, Case::Ng, false));
    }
    render_pps(&mut s, &c.pps);
    push_toks(&mut s, &c.advs);
    push_loose(&mut s, &c.loose);
    s
}

fn render_cop(c: &Clause<'_>) -> String {
    let mut s = String::new();
    push_toks(&mut s, &c.front);
    if c.neg {
        push_word(&mut s, "hindi");
    }
    if let Some(adj) = c.pred_adj {
        push_word(&mut s, lem_tl(adj));
    } else if !np_empty(&c.pred_np) {
        push_word(&mut s, &render_np(&c.pred_np, Case::Bare, true));
    }
    if let Some(p) = c.subj.pron {
        push_word(&mut s, lem_ang(p));
    } else if !np_empty(&c.subj) {
        push_word(&mut s, &render_np(&c.subj, Case::Ang, false));
    }
    render_pps(&mut s, &c.pps);
    push_toks(&mut s, &c.advs);
    push_loose(&mut s, &c.loose);
    s
}

fn render_finite(c: &Clause<'_>) -> String {
    let Some(verb) = c.verb else {
        return String::new();
    };
    let mut s = String::new();
    push_toks(&mut s, &c.front);
    if c.neg {
        push_word(&mut s, "hindi");
    }
    let asp = aspect(verb, c.will, c.do_pret, false, !np_empty(&c.subj));
    push_word(&mut s, surface_for(verb, asp));
    push_actor(&mut s, verb, &c.subj);
    if !np_empty(&c.obj) {
        push_word(&mut s, &render_obj(verb, &c.obj));
    }
    render_pps(&mut s, &c.pps);
    push_toks(&mut s, &c.advs);
    push_loose(&mut s, &c.loose);
    s
}

fn render_loose_clause(c: &Clause<'_>) -> String {
    let mut s = String::new();
    push_toks(&mut s, &c.front);
    if c.neg {
        push_word(&mut s, "hindi");
    }
    if let Some(m) = c.modal {
        push_word(&mut s, lem_tl(m));
    }
    if let Some(p) = c.subj.pron {
        push_word(&mut s, lem_ang(p));
    } else if !np_empty(&c.subj) {
        push_word(&mut s, &render_np(&c.subj, Case::Ang, false));
    }
    if !np_empty(&c.obj) {
        push_word(&mut s, &render_np(&c.obj, Case::Ang, false));
    }
    render_pps(&mut s, &c.pps);
    push_toks(&mut s, &c.advs);
    push_loose(&mut s, &c.loose);
    s
}

fn aspect(verb: &Tok<'_>, will: bool, do_pret: bool, modal: bool, have_subj: bool) -> u8 {
    if will {
        return ASP_CONT;
    }
    if modal {
        return ASP_INF;
    }
    if verb.ten == TEN_PRET || do_pret {
        return ASP_PERF;
    }
    if verb.ten == TEN_GER {
        return ASP_IMPF;
    }
    if !have_subj {
        return ASP_INF;
    }
    ASP_IMPF
}

fn surface_for(verb: &Tok<'_>, asp: u8) -> &'static str {
    let Some(lem) = verb.lem else {
        return "";
    };
    let pick = match asp {
        ASP_INF => lem.inf,
        ASP_PERF => lem.perf,
        ASP_IMPF => lem.impf,
        ASP_CONT => lem.cont,
        _ => "",
    };
    if !pick.is_empty() {
        pick
    } else if !lem.tl.is_empty() {
        lem.tl
    } else {
        lem.en
    }
}

fn render_obj(verb: &Tok<'_>, obj: &Np<'_>) -> String {
    let lem = verb.lem;
    let focus_ng = lem.map(|l| l.focus_ng).unwrap_or(false);
    let saobj = lem.map(|l| l.saobj).unwrap_or(false);
    if saobj && !focus_ng {
        let mut s = String::from("sa");
        push_word(&mut s, &render_np(obj, Case::Bare, true));
        return s;
    }
    if focus_ng {
        render_np(obj, Case::Ang, false)
    } else {
        render_np(obj, Case::Ng, false)
    }
}

fn push_actor(out: &mut String, verb: &Tok<'_>, subj: &Np<'_>) {
    let focus_ng = verb.lem.map(|l| l.focus_ng).unwrap_or(false);
    if let Some(p) = subj.pron {
        let form = if focus_ng { lem_gen(p) } else { lem_ang(p) };
        push_word(out, form);
    } else if !np_empty(subj) {
        let case = if focus_ng { Case::Ng } else { Case::Ang };
        push_word(out, &render_np(subj, case, false));
    }
}

fn push_actor_gen(out: &mut String, subj: &Np<'_>) {
    if let Some(p) = subj.pron {
        push_word(out, lem_gen(p));
    } else if !np_empty(subj) {
        push_word(out, &render_np(subj, Case::Ng, false));
    }
}

fn render_np(np: &Np<'_>, case: Case, drop_marker: bool) -> String {
    if let Some(p) = np.pron {
        return match case {
            Case::Ng => lem_gen(p).to_string(),
            Case::Ang | Case::Bare => lem_ang(p).to_string(),
        };
    }
    let mut s = String::new();
    let a = det_a(np);
    if !drop_marker {
        match case {
            Case::Ang => push_word(&mut s, if a { "isang" } else { "ang" }),
            Case::Ng => {
                push_word(&mut s, "ng");
                if a {
                    push_word(&mut s, "isang");
                }
            }
            Case::Bare => {
                if a {
                    push_word(&mut s, "isang");
                }
            }
        }
    } else if a {
        push_word(&mut s, "isang");
    }
    if np.plural {
        push_word(&mut s, "mga");
    }
    let head = np_head(np);
    let mut ai = 0;
    while ai < np.adjs.len() {
        let raw = lem_tl(np.adjs[ai]);
        if ai + 1 < np.adjs.len() || !head.is_empty() {
            push_word(&mut s, &link_adj(raw));
        } else {
            push_word(&mut s, raw);
        }
        ai += 1;
    }
    if !head.is_empty() {
        push_word(&mut s, head);
    }
    if let Some(poss) = np.poss {
        let g = lem_gen(poss);
        if !g.is_empty() {
            push_word(&mut s, g);
        }
    }
    s
}

fn np_head<'a>(np: &'a Np<'_>) -> &'a str {
    if let Some(h) = np.head {
        return lem_tl(h);
    }
    np.unknown.unwrap_or("")
}

fn det_a(np: &Np<'_>) -> bool {
    np.det
        .and_then(|d| d.lem)
        .map(|l| l.sub == SUB_A)
        .unwrap_or(false)
}

fn link_adj(adj: &str) -> String {
    let mut s = String::from(adj);
    if adj.ends_with('n') {
        s.push('g');
    } else if ends_vowel(adj) {
        s.push_str("ng");
    } else {
        s.push_str(" na");
    }
    s
}

fn linker(base: &str) -> String {
    if base.is_empty() {
        return String::new();
    }
    let mut s = String::with_capacity(base.len() + 2);
    s.push_str(base);
    if base.ends_with('n') {
        s.push('g');
    } else {
        s.push_str("ng");
    }
    s
}

fn ends_vowel(s: &str) -> bool {
    match s.chars().last() {
        Some(ch) => matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u'),
        None => false,
    }
}

fn render_pps(out: &mut String, pps: &[Pp<'_>]) {
    for pp in pps {
        if let Some(lem) = pp.prep.lem {
            push_word(out, lem.tl);
        }
        if !np_empty(&pp.np) {
            let drop = pp.prep.lem.map(|l| l.drop_det).unwrap_or(false);
            let case = if drop { Case::Bare } else { Case::Ang };
            push_word(out, &render_np(&pp.np, case, drop));
        }
    }
}

fn push_toks(out: &mut String, toks: &[&Tok<'_>]) {
    for t in toks {
        if let Some(lem) = t.lem {
            if !lem.tl.is_empty() {
                push_word(out, lem.tl);
            }
        }
    }
}

fn push_loose(out: &mut String, loose: &[&Tok<'_>]) {
    for t in loose {
        if let Some(lem) = t.lem {
            if is_content_kind(lem.kind) {
                let w = if lem.tl.is_empty() { t.raw } else { lem.tl };
                push_word(out, w);
            }
        } else {
            push_word(out, t.raw);
        }
    }
}

fn push_word(out: &mut String, w: &str) {
    if w.is_empty() {
        return;
    }
    if !out.is_empty() && !out.ends_with(' ') {
        out.push(' ');
    }
    out.push_str(w);
}

fn np_empty(np: &Np<'_>) -> bool {
    np.pron.is_none() && np.head.is_none() && np.unknown.is_none()
}

fn lem_tl(t: &Tok<'_>) -> &'static str {
    t.lem.map(|l| l.tl).unwrap_or("")
}

fn lem_ang(t: &Tok<'_>) -> &'static str {
    t.lem.map(|l| l.ang).unwrap_or("")
}

fn lem_gen(t: &Tok<'_>) -> &'static str {
    t.lem.map(|l| l.gen).unwrap_or("")
}

fn en_is(t: &Tok<'_>, en: &str) -> bool {
    t.lem.map(|l| l.en == en).unwrap_or(false)
}

fn kind_of(t: &Tok<'_>) -> u8 {
    t.lem.map(|l| l.kind).unwrap_or(0)
}

fn sub_of(t: &Tok<'_>) -> u8 {
    t.lem.map(|l| l.sub).unwrap_or(0)
}

fn is_adv(t: &Tok<'_>) -> bool {
    kind_of(t) == K_ADV
}

fn is_adj(t: &Tok<'_>) -> bool {
    kind_of(t) == K_ADJ
}

fn is_noun(t: &Tok<'_>) -> bool {
    kind_of(t) == K_N
}

fn is_pro(t: &Tok<'_>) -> bool {
    kind_of(t) == K_PRO
}

fn is_poss(t: &Tok<'_>) -> bool {
    kind_of(t) == K_POSS
}

fn is_det(t: &Tok<'_>) -> bool {
    kind_of(t) == K_DET
}

fn is_prep(t: &Tok<'_>) -> bool {
    kind_of(t) == K_PREP
}

fn is_conj(t: &Tok<'_>) -> bool {
    kind_of(t) == K_CONJ
}

fn is_cop(t: &Tok<'_>) -> bool {
    kind_of(t) == K_COP
}

fn is_au(t: &Tok<'_>) -> bool {
    kind_of(t) == K_AU
}

fn is_will(t: &Tok<'_>) -> bool {
    kind_of(t) == K_AUX && sub_of(t) == SUB_WILL
}

fn is_do(t: &Tok<'_>) -> bool {
    kind_of(t) == K_AUX && sub_of(t) == SUB_DO
}

fn is_not(t: &Tok<'_>) -> bool {
    sub_of(t) == SUB_NOT || en_is(t, "not")
}

fn is_that(t: &Tok<'_>) -> bool {
    sub_of(t) == SUB_THAT || en_is(t, "that")
}

fn is_inf_to(t: &Tok<'_>) -> bool {
    en_is(t, "to")
}

fn is_think(t: &Tok<'_>) -> bool {
    kind_of(t) == K_CW && en_is(t, "think")
}

fn is_need_want(t: &Tok<'_>) -> bool {
    kind_of(t) == K_PSEUDO && (en_is(t, "need") || en_is(t, "want") || en_is(t, "like"))
}

fn is_main_verb(t: &Tok<'_>) -> bool {
    matches!(kind_of(t), K_MAG | K_UM | K_IRR | K_PSEUDO) && !is_need_want(t)
}

fn is_content_tok(t: &Tok<'_>) -> bool {
    t.lem.map(|l| is_content_kind(l.kind)).unwrap_or(false)
}

fn is_skippable_fn(t: &Tok<'_>) -> bool {
    kind_of(t) == K_FN && !is_not(t) && !is_that(t)
}
