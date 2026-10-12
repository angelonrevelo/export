//! Shallow transfer. One subject noun phrase, one object, a copula or one finite verb,
//! optional `estar`+gerund, tú-imperative, and trailing adjuncts.
//!
//! Conjunctions split clauses. A verb is not copied onto the next conjunct.
//! Unknown tokens are copied only when something else in the sentence translated.

use crate::fold::fold_into;
use crate::lexicon::*;
use crate::scan::{tokenize, Raw};

#[derive(Clone, Copy)]
enum TokE<'a> {
    Mwe(&'static str),
    Punct(&'a str),
    Unknown(&'a str),
    G(Eg),
}

#[derive(Clone, Copy)]
enum TokS<'a> {
    Mwe(&'static str),
    Punct(&'a str),
    Unknown(&'a str),
    G(Sg),
}

struct Np {
    pron: Option<&'static str>,
    det: Option<DetK>,
    head: &'static str,
    gen: Gen,
    num: Num,
    per: Per,
    adjs: [&'static str; 4],
    adj_n: u8,
}

pub(crate) fn en_to_es(text: &str) -> Result<String, &'static str> {
    let toks = analyze_en(text);
    if !any_en(&toks) {
        return Err("nothing");
    }
    let mut out = String::new();
    render_en(&toks, &mut out);
    Ok(out)
}

pub(crate) fn es_to_en(text: &str) -> Result<String, &'static str> {
    let toks = analyze_es(text);
    if !any_es(&toks) {
        return Err("nothing");
    }
    let mut out = String::new();
    render_es(&toks, &mut out);
    Ok(out)
}

fn any_en(toks: &[TokE<'_>]) -> bool {
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            TokE::Mwe(_) | TokE::G(_) => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn any_es(toks: &[TokS<'_>]) -> bool {
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            TokS::Mwe(_) | TokS::G(_) => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

struct Item<'a> {
    raw: Raw<'a>,
    k0: u32,
    k1: u32,
}

fn items_of<'a>(text: &'a str, buf: &mut String) -> Vec<Item<'a>> {
    let raws = tokenize(text);
    let mut items = Vec::with_capacity(raws.len());
    let mut scratch = String::new();
    for raw in raws {
        let start = buf.len() as u32;
        if let Raw::Word(w) = raw {
            fold_into(w, &mut scratch);
            buf.push_str(&scratch);
        }
        items.push(Item {
            raw,
            k0: start,
            k1: buf.len() as u32,
        });
    }
    items
}

fn mwe_end(
    items: &[Item<'_>],
    buf: &str,
    i: usize,
    table: &[Mwe],
) -> Option<(usize, &'static str)> {
    let mut best: Option<(usize, &'static str)> = None;
    let mut t = 0;
    while t < table.len() {
        let m = &table[t];
        let mut j = i;
        let mut k = 0;
        let mut ok = true;
        while k < m.w.len() {
            if j >= items.len() {
                ok = false;
                break;
            }
            let Raw::Word(_) = items[j].raw else {
                ok = false;
                break;
            };
            let key = &buf[items[j].k0 as usize..items[j].k1 as usize];
            if key != m.w[k] {
                ok = false;
                break;
            }
            j += 1;
            k += 1;
        }
        if ok {
            let n = m.w.len();
            if !matches!(best, Some((bn, _)) if bn >= n) {
                best = Some((j, m.out));
            }
        }
        t += 1;
    }
    best
}

fn analyze_en(text: &str) -> Vec<TokE<'_>> {
    let mut buf = String::new();
    let items = items_of(text, &mut buf);
    let mut out = Vec::with_capacity(items.len());
    let mut i = 0;
    while i < items.len() {
        if let Some((end, phrase)) = mwe_end(&items, &buf, i, MWE_EN) {
            out.push(TokE::Mwe(phrase));
            i = end;
            continue;
        }
        match items[i].raw {
            Raw::Punct(p) => out.push(TokE::Punct(p)),
            Raw::Word(w) => {
                let key = &buf[items[i].k0 as usize..items[i].k1 as usize];
                match find_en(key) {
                    Some(e) => out.push(TokE::G(e.g)),
                    None => out.push(TokE::Unknown(w)),
                }
            }
        }
        i += 1;
    }
    out
}

fn analyze_es(text: &str) -> Vec<TokS<'_>> {
    let mut buf = String::new();
    let items = items_of(text, &mut buf);
    let mut out = Vec::with_capacity(items.len());
    let mut i = 0;
    while i < items.len() {
        if let Some((end, phrase)) = mwe_end(&items, &buf, i, MWE_ES) {
            out.push(TokS::Mwe(phrase));
            i = end;
            continue;
        }
        match items[i].raw {
            Raw::Punct(p) => out.push(TokS::Punct(p)),
            Raw::Word(w) => {
                let key = &buf[items[i].k0 as usize..items[i].k1 as usize];
                match find_es(key) {
                    Some(e) => out.push(TokS::G(e.g)),
                    None => out.push(TokS::Unknown(w)),
                }
            }
        }
        i += 1;
    }
    out
}

fn render_en(toks: &[TokE<'_>], out: &mut String) {
    let mut start = 0;
    let mut i = 0;
    while i <= toks.len() {
        let boundary = if i == toks.len() {
            true
        } else {
            matches!(toks[i], TokE::Punct(",")) || conj_en(&toks[i]).is_some()
        };
        if boundary {
            let q = clause_q_en(&toks[start..i]);
            let body = clause_en(&toks[start..i]);
            push_clause(out, &body, q, true);
            if i == toks.len() {
                break;
            }
            match toks[i] {
                TokE::Punct(",") => out.push(','),
                _ => {
                    if let Some(c) = conj_en(&toks[i]) {
                        push_word(out, c);
                    }
                }
            }
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
}

fn render_es(toks: &[TokS<'_>], out: &mut String) {
    let mut start = 0;
    let mut i = 0;
    while i <= toks.len() {
        let boundary = if i == toks.len() {
            true
        } else {
            matches!(toks[i], TokS::Punct(",")) || conj_es(&toks[i]).is_some()
        };
        if boundary {
            let q = clause_q_es(&toks[start..i]);
            let body = clause_es(&toks[start..i]);
            // English keeps the question mark the clause already saw; no ¿.
            push_clause(out, &body, q, false);
            if i == toks.len() {
                break;
            }
            match toks[i] {
                TokS::Punct(",") => out.push(','),
                _ => {
                    if let Some(c) = conj_es(&toks[i]) {
                        push_word(out, c);
                    }
                }
            }
            i += 1;
            start = i;
            continue;
        }
        i += 1;
    }
}

fn push_clause(out: &mut String, body: &str, q: bool, spanish_q: bool) {
    let body = body.trim();
    if body.is_empty() && !q {
        return;
    }
    if spanish_q && q {
        if !out.is_empty() && !out.ends_with(' ') {
            out.push(' ');
        }
        out.push('¿');
        out.push_str(body);
        if !out.ends_with('?') {
            out.push('?');
        }
        return;
    }
    if !body.is_empty() {
        push_word(out, body);
    }
    if q && !spanish_q && !out.ends_with('?') {
        out.push('?');
    }
}

fn clause_q_en(toks: &[TokE<'_>]) -> bool {
    let mut i = 0;
    while i < toks.len() {
        if matches!(toks[i], TokE::Punct("?")) {
            return true;
        }
        i += 1;
    }
    false
}

fn clause_q_es(toks: &[TokS<'_>]) -> bool {
    let mut i = 0;
    while i < toks.len() {
        if matches!(toks[i], TokS::Punct("?")) {
            return true;
        }
        i += 1;
    }
    false
}

fn conj_en(t: &TokE<'_>) -> Option<&'static str> {
    match t {
        TokE::G(Eg::Conj(s)) => Some(*s),
        _ => None,
    }
}

fn conj_es(t: &TokS<'_>) -> Option<&'static str> {
    match t {
        TokS::G(Sg::Conj(s)) => Some(*s),
        _ => None,
    }
}

fn clause_en(toks: &[TokE<'_>]) -> String {
    if !verbal_en(toks) {
        return linear_en(toks);
    }
    let mut i = 0;
    let mut please = false;
    if i < toks.len() && matches!(toks[i], TokE::G(Eg::Please)) {
        please = true;
        i += 1;
    }
    let mut subject = None;
    if let Some((np, n)) = take_np_en(toks, i) {
        subject = Some(np);
        i = n;
    }
    let mut neg = false;
    let mut prog: Option<&Parad> = None;
    let mut pred: Option<(Cop, &'static str)> = None;
    let mut lex: Option<(&Parad, VForm)> = None;
    let mut be = false;
    let mut object = None;
    if i < toks.len() && matches!(toks[i], TokE::G(Eg::Be)) {
        be = true;
        i += 1;
        if i < toks.len() && matches!(toks[i], TokE::G(Eg::Not)) {
            neg = true;
            i += 1;
        }
        if i < toks.len() && is_ger_en(&toks[i]) {
            if let TokE::G(Eg::Verb(p, _)) = toks[i] {
                prog = Some(p);
            }
            i += 1;
        } else if let Some((np, n)) = take_np_en(toks, i) {
            object = Some(np);
            i = n;
        } else if i < toks.len() {
            if let Some(adj) = pred_adj_en(&toks[i], subject.as_ref()) {
                pred = Some(adj);
                i += 1;
            }
        }
    } else if i < toks.len() {
        if let TokE::G(Eg::Verb(p, form)) = toks[i] {
            lex = Some((p, form));
            i += 1;
            if i < toks.len() && matches!(toks[i], TokE::G(Eg::Not)) {
                neg = true;
                i += 1;
            }
            if let Some((np, n)) = take_np_en(toks, i) {
                object = Some(np);
                i = n;
            }
        }
    }
    let mut out = String::new();
    if please {
        push_word(&mut out, "por favor");
    }
    let agr = subject
        .as_ref()
        .map(agr_of)
        .unwrap_or(Agr { per: Per::You });
    let imperative = subject.is_none() && matches!(lex, Some((_, VForm::Base)));
    if let Some(ref np) = subject {
        if !imperative {
            render_np_es(np, &mut out);
        }
    }
    if neg && (be || lex.is_some() || prog.is_some()) {
        push_word(&mut out, "no");
    }
    if let Some(p) = prog {
        push_word(&mut out, estar(agr.per));
        push_word(&mut out, p.ger);
    } else if let Some((cop, form)) = pred {
        let word = if cop == Cop::Estar {
            estar(agr.per)
        } else {
            ser(agr.per)
        };
        push_word(&mut out, word);
        push_word(&mut out, form);
    } else if be && object.is_some() {
        push_word(&mut out, ser(agr.per));
    } else if let Some((p, form)) = lex {
        let word = if imperative {
            p.imp
        } else if form == VForm::Ger {
            p.ger
        } else {
            conj_parad(p, agr.per)
        };
        push_word(&mut out, word);
    }
    if let Some(ref np) = object {
        render_np_es(np, &mut out);
    }
    while i < toks.len() {
        if matches!(
            toks[i],
            TokE::Punct("?") | TokE::Punct("!") | TokE::Punct(".")
        ) {
            i += 1;
            continue;
        }
        if let TokE::G(Eg::Prep(p)) = toks[i] {
            push_word(&mut out, p);
            i += 1;
            if let Some((np, n)) = take_np_en(toks, i) {
                render_np_es(&np, &mut out);
                i = n;
            }
            continue;
        }
        push_tok_en(&mut out, &toks[i]);
        i += 1;
    }
    out
}

fn clause_es(toks: &[TokS<'_>]) -> String {
    if !verbal_es(toks) {
        return linear_es(toks);
    }
    let mut i = 0;
    let mut please = false;
    if i < toks.len() && matches!(toks[i], TokS::Mwe("please")) {
        please = true;
        i += 1;
    }
    let mut subject = None;
    if let Some((np, n)) = take_np_es(toks, i) {
        subject = Some(np);
        i = n;
    }
    let mut neg = false;
    if i < toks.len() && matches!(toks[i], TokS::G(Sg::Not)) {
        neg = true;
        i += 1;
    }
    let mut prog: Option<&Parad> = None;
    let mut be_per: Option<Per> = None;
    let mut pred: Option<&'static str> = None;
    let mut lex: Option<(&Parad, VForm)> = None;
    let mut saw_be = false;
    let mut object = None;
    if i < toks.len() {
        if let TokS::G(Sg::Be { per, .. }) = toks[i] {
            saw_be = true;
            be_per = Some(per);
            i += 1;
            if i < toks.len() && is_ger_es(&toks[i]) {
                if let TokS::G(Sg::Verb(p, _)) = toks[i] {
                    prog = Some(p);
                }
                i += 1;
            } else if let Some((np, n)) = take_np_es(toks, i) {
                object = Some(np);
                i = n;
            } else if i < toks.len() {
                if let TokS::G(Sg::Adj { en, .. }) = toks[i] {
                    pred = Some(en);
                    i += 1;
                }
            }
        } else if let TokS::G(Sg::Verb(p, form)) = toks[i] {
            lex = Some((p, form));
            i += 1;
            if let Some((np, n)) = take_np_es(toks, i) {
                object = Some(np);
                i = n;
            }
        }
    }
    let imperative = subject.is_none() && matches!(lex, Some((_, VForm::ImpOrS3 | VForm::Base)));
    let mut out = String::new();
    if please {
        push_word(&mut out, "please");
    }
    if let Some(ref np) = subject {
        if !imperative {
            render_np_en(np, &mut out);
        }
    }
    let per = be_per.unwrap_or(Per::He);
    if let Some(p) = prog {
        push_word(&mut out, en_be(per));
        if neg {
            push_word(&mut out, "not");
        }
        push_word(&mut out, p.en_ger);
    } else if let Some(adj) = pred {
        push_word(&mut out, en_be(per));
        if neg {
            push_word(&mut out, "not");
        }
        push_word(&mut out, adj);
    } else if saw_be && object.is_some() {
        push_word(&mut out, en_be(per));
        if neg {
            push_word(&mut out, "not");
        }
    } else if let Some((p, form)) = lex {
        if neg {
            push_word(&mut out, "not");
        }
        push_word(&mut out, en_verb(p, form, subject.is_some()));
    }
    if let Some(ref np) = object {
        render_np_en(np, &mut out);
    }
    while i < toks.len() {
        if matches!(
            toks[i],
            TokS::Punct("?")
                | TokS::Punct("!")
                | TokS::Punct(".")
                | TokS::Punct("¿")
                | TokS::Punct("¡")
        ) {
            i += 1;
            continue;
        }
        if let TokS::G(Sg::Prep(p)) = toks[i] {
            push_word(&mut out, p);
            i += 1;
            if let Some((np, n)) = take_np_es(toks, i) {
                render_np_en(&np, &mut out);
                i = n;
            }
            continue;
        }
        push_tok_es(&mut out, &toks[i]);
        i += 1;
    }
    out
}

struct Agr {
    per: Per,
}

fn agr_of(np: &Np) -> Agr {
    Agr { per: np.per }
}

fn verbal_en(toks: &[TokE<'_>]) -> bool {
    let mut i = 0;
    while i < toks.len() {
        if let TokE::G(Eg::Be | Eg::Please | Eg::Verb(_, _)) = toks[i] {
            return true;
        }
        i += 1;
    }
    false
}

fn verbal_es(toks: &[TokS<'_>]) -> bool {
    let mut i = 0;
    while i < toks.len() {
        match toks[i] {
            TokS::Mwe("please") => return true,
            TokS::G(Sg::Be { .. } | Sg::Verb(_, _)) => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

fn is_ger_en(t: &TokE<'_>) -> bool {
    matches!(t, TokE::G(Eg::Verb(_, VForm::Ger)))
}

fn is_ger_es(t: &TokS<'_>) -> bool {
    matches!(t, TokS::G(Sg::Verb(_, VForm::Ger)))
}

fn is_adj_en(t: &TokE<'_>) -> bool {
    matches!(t, TokE::G(Eg::Adj { .. }))
}

fn take_np_en(toks: &[TokE<'_>], i: usize) -> Option<(Np, usize)> {
    if i >= toks.len() {
        return None;
    }
    if let TokE::G(Eg::Pron(es, per)) = toks[i] {
        let (gen, num) = pron_agr(per);
        return Some((
            Np {
                pron: Some(es),
                det: None,
                head: es,
                gen,
                num,
                per,
                adjs: [""; 4],
                adj_n: 0,
            },
            i + 1,
        ));
    }
    let mut j = i;
    let mut det = None;
    if let TokE::G(Eg::Det(d)) = toks[j] {
        det = Some(d);
        j += 1;
    }
    let mut adjs: [&str; 4] = [""; 4];
    let mut adj_n: u8 = 0;
    let adj_at = j;
    while j < toks.len() && is_adj_en(&toks[j]) {
        j += 1;
    }
    if j >= toks.len() {
        return None;
    }
    let TokE::G(Eg::Noun { sg, pl, g, n }) = toks[j] else {
        return None;
    };
    let head = if n == Num::Pl { pl } else { sg };
    let saved = j;
    j = adj_at;
    while j < saved {
        if let TokE::G(Eg::Adj { m, f, mp, fp, .. }) = toks[j] {
            let form = adj_form(m, f, mp, fp, g, n);
            if (adj_n as usize) < adjs.len() {
                adjs[adj_n as usize] = form;
                adj_n += 1;
            }
        }
        j += 1;
    }
    let per = if n == Num::Pl { Per::They } else { Per::He };
    Some((
        Np {
            pron: None,
            det,
            head,
            gen: g,
            num: n,
            per,
            adjs,
            adj_n,
        },
        saved + 1,
    ))
}

fn take_np_es(toks: &[TokS<'_>], i: usize) -> Option<(Np, usize)> {
    if i >= toks.len() {
        return None;
    }
    if let TokS::G(Sg::Pron(en, per)) = toks[i] {
        let (gen, num) = pron_agr(per);
        return Some((
            Np {
                pron: Some(en),
                det: None,
                head: en,
                gen,
                num,
                per,
                adjs: [""; 4],
                adj_n: 0,
            },
            i + 1,
        ));
    }
    let mut j = i;
    let mut det = None;
    if let TokS::G(Sg::Det(d)) = toks[j] {
        det = Some(d);
        j += 1;
    }
    let TokS::G(Sg::Noun { en, n }) = toks.get(j)? else {
        return None;
    };
    let num = *n;
    let head = *en;
    j += 1;
    let mut adjs: [&str; 4] = [""; 4];
    let mut adj_n: u8 = 0;
    while j < toks.len() {
        if let TokS::G(Sg::Adj { en, .. }) = toks[j] {
            if (adj_n as usize) < adjs.len() {
                adjs[adj_n as usize] = en;
                adj_n += 1;
            }
            j += 1;
            continue;
        }
        break;
    }
    let per = if num == Num::Pl { Per::They } else { Per::He };
    Some((
        Np {
            pron: None,
            det,
            head,
            gen: Gen::M,
            num,
            per,
            adjs,
            adj_n,
        },
        j,
    ))
}

fn pred_adj_en(t: &TokE<'_>, subject: Option<&Np>) -> Option<(Cop, &'static str)> {
    let TokE::G(Eg::Adj { m, f, mp, fp, cop }) = *t else {
        return None;
    };
    let (g, n) = match subject {
        Some(np) => (np.gen, np.num),
        None => (Gen::M, Num::Sg),
    };
    Some((cop, adj_form(m, f, mp, fp, g, n)))
}

fn pron_agr(per: Per) -> (Gen, Num) {
    match per {
        Per::She => (Gen::F, Num::Sg),
        Per::We | Per::They => (Gen::M, Num::Pl),
        _ => (Gen::M, Num::Sg),
    }
}

fn adj_form(
    m: &'static str,
    f: &'static str,
    mp: &'static str,
    fp: &'static str,
    g: Gen,
    n: Num,
) -> &'static str {
    match (g, n) {
        (Gen::M, Num::Sg) => m,
        (Gen::F, Num::Sg) => f,
        (Gen::M, Num::Pl) => mp,
        (Gen::F, Num::Pl) => fp,
    }
}

fn render_np_es(np: &Np, out: &mut String) {
    if let Some(p) = np.pron {
        push_word(out, p);
        return;
    }
    if let Some(d) = np.det {
        push_word(out, det_es(d, np.gen, np.num, np.head));
    }
    push_word(out, np.head);
    let mut k = 0;
    while k < np.adj_n as usize {
        push_word(out, np.adjs[k]);
        k += 1;
    }
}

fn render_np_en(np: &Np, out: &mut String) {
    if let Some(p) = np.pron {
        push_word(out, p);
        return;
    }
    if let Some(d) = np.det {
        push_word(out, if d == DetK::The { "the" } else { "a" });
    }
    let mut k = 0;
    while k < np.adj_n as usize {
        push_word(out, np.adjs[k]);
        k += 1;
    }
    push_word(out, np.head);
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

fn estar(per: Per) -> &'static str {
    match per {
        Per::I => "estoy",
        Per::You => "estás",
        Per::We => "estamos",
        Per::They => "están",
        Per::He | Per::She => "está",
    }
}

fn ser(per: Per) -> &'static str {
    match per {
        Per::I => "soy",
        Per::You => "eres",
        Per::We => "somos",
        Per::They => "son",
        Per::He | Per::She => "es",
    }
}

fn en_be(per: Per) -> &'static str {
    match per {
        Per::I => "am",
        Per::You | Per::We | Per::They => "are",
        Per::He | Per::She => "is",
    }
}

fn conj_parad(p: &Parad, per: Per) -> &'static str {
    match per {
        Per::I => p.s1,
        Per::You => p.s2,
        Per::We => p.p1,
        Per::They => p.p3,
        Per::He | Per::She => p.s3,
    }
}

fn en_verb(p: &Parad, form: VForm, has_subject: bool) -> &'static str {
    match form {
        VForm::Ger => p.en_ger,
        VForm::S3 => p.en_s3,
        VForm::ImpOrS3 => {
            if has_subject {
                p.en_s3
            } else {
                p.en
            }
        }
        VForm::Base => p.en,
    }
}

fn linear_en(toks: &[TokE<'_>]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < toks.len() {
        if !is_end_punct_en(&toks[i]) {
            push_tok_en(&mut out, &toks[i]);
        }
        i += 1;
    }
    out
}

fn linear_es(toks: &[TokS<'_>]) -> String {
    let mut out = String::new();
    let mut i = 0;
    while i < toks.len() {
        if !is_end_punct_es(&toks[i]) {
            push_tok_es(&mut out, &toks[i]);
        }
        i += 1;
    }
    out
}

fn is_end_punct_en(t: &TokE<'_>) -> bool {
    matches!(
        t,
        TokE::Punct("?")
            | TokE::Punct("!")
            | TokE::Punct(".")
            | TokE::Punct("¿")
            | TokE::Punct("¡")
    )
}

fn is_end_punct_es(t: &TokS<'_>) -> bool {
    matches!(
        t,
        TokS::Punct("?")
            | TokS::Punct("!")
            | TokS::Punct(".")
            | TokS::Punct("¿")
            | TokS::Punct("¡")
    )
}

fn push_tok_en(out: &mut String, t: &TokE<'_>) {
    match t {
        TokE::Mwe(s) => push_word(out, s),
        TokE::Unknown(w) => push_word(out, w),
        TokE::Punct(p) => push_word(out, p),
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
            push_word(out, w);
        }
    }
}

fn push_tok_es(out: &mut String, t: &TokS<'_>) {
    match t {
        TokS::Mwe(s) => push_word(out, s),
        TokS::Unknown(w) => push_word(out, w),
        TokS::Punct(p) => push_word(out, p),
        TokS::G(g) => {
            let w = match *g {
                Sg::Conj(s)
                | Sg::Prep(s)
                | Sg::Adv(s)
                | Sg::Interj(s)
                | Sg::Phrase(s)
                | Sg::Pron(s, _) => s,
                Sg::Not => "not",
                Sg::Det(DetK::The) => "the",
                Sg::Det(DetK::A) => "a",
                Sg::Be { per, .. } => en_be(per),
                Sg::Noun { en, .. } | Sg::Adj { en, .. } => en,
                Sg::Verb(p, form) => en_verb(p, form, true),
            };
            push_word(out, w);
        }
    }
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
