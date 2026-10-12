//! Spanish inflection used by the runtime and by `build.rs` (via `include!`).
//! One implementation so forward forms and the reverse surface index match.

use crate::kind::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum VCls {
    Ar,
    Er,
    Ir,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Pers {
    I,
    You,
    He,
    We,
    They,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Ten {
    Pres,
    Pret,
    Ger,
    Inf,
    Imp,
    Imperf,
}

impl Pers {
    #[allow(dead_code)]
    pub fn code(self) -> u8 {
        match self {
            Pers::I => PER_I,
            Pers::You => PER_YOU,
            Pers::He => PER_HE,
            Pers::We => PER_WE,
            Pers::They => PER_THEY,
        }
    }

    #[allow(dead_code)]
    pub fn all() -> [Pers; 5] {
        [Pers::I, Pers::You, Pers::He, Pers::We, Pers::They]
    }
}

impl Ten {
    #[allow(dead_code)]
    pub fn code(self) -> u8 {
        match self {
            Ten::Pres => TEN_PRES,
            Ten::Pret => TEN_PRET,
            Ten::Ger => TEN_GER,
            Ten::Inf => TEN_INF,
            Ten::Imp => TEN_IMP,
            Ten::Imperf => TEN_IMPERF,
        }
    }
}

pub fn class_of(inf: &str) -> VCls {
    if inf.ends_with("ar") {
        VCls::Ar
    } else if inf.ends_with("ir") {
        VCls::Ir
    } else {
        VCls::Er
    }
}

pub fn conj_into(out: &mut String, inf: &str, stem_code: u8, ten: Ten, per: Pers) {
    out.clear();
    if let Some(hit) = irregular(inf, ten, per) {
        out.push_str(hit);
        return;
    }
    match ten {
        Ten::Inf => out.push_str(inf),
        Ten::Ger => push_ger(out, inf, stem_code),
        Ten::Imp => push_finite(out, inf, stem_code, Ten::Pres, Pers::He),
        Ten::Imperf => push_imperf(out, inf, per),
        Ten::Pres | Ten::Pret => push_finite(out, inf, stem_code, ten, per),
    }
}

fn push_ger(out: &mut String, inf: &str, stem_code: u8) {
    let (stem, cls) = split_inf(inf);
    if cls == VCls::Ir && stem_code == STEM_UE {
        let swapped = swap_last(stem, 'o', "u");
        out.push_str(&swapped);
        out.push_str("iendo");
        return;
    }
    if cls == VCls::Ir && (stem_code == STEM_EI || stem_code == STEM_IE) {
        let swapped = swap_last(stem, 'e', "i");
        out.push_str(&swapped);
        out.push_str("iendo");
        return;
    }
    out.push_str(stem);
    if cls == VCls::Ar {
        out.push_str("ando");
    } else {
        out.push_str("iendo");
    }
}

fn push_imperf(out: &mut String, inf: &str, per: Pers) {
    let (stem, cls) = split_inf(inf);
    out.push_str(stem);
    if cls == VCls::Ar {
        out.push_str(match per {
            Pers::I | Pers::He => "aba",
            Pers::You => "abas",
            Pers::We => "ábamos",
            Pers::They => "aban",
        });
    } else {
        out.push_str(match per {
            Pers::I | Pers::He => "ía",
            Pers::You => "ías",
            Pers::We => "íamos",
            Pers::They => "ían",
        });
    }
}

fn push_finite(out: &mut String, inf: &str, stem_code: u8, ten: Ten, per: Pers) {
    let (stem, cls) = split_inf(inf);
    let stressed = !matches!(per, Pers::We);
    let mut core = String::new();
    if ten == Ten::Pres && stressed {
        apply_stem(&mut core, stem, stem_code);
    } else {
        core.push_str(stem);
    }
    if ten == Ten::Pres && per == Pers::I && stem_code == STEM_ZCO {
        core = zco_stem(stem);
    }
    if ten == Ten::Pret && per == Pers::I {
        spell_pret_yo(&mut core);
    }
    out.push_str(&core);
    out.push_str(ending(cls, ten, per));
    if ten == Ten::Pres && stressed && (stem_code == STEM_IAR || stem_code == STEM_UAR) {
        accent_last_vowel(out);
    }
}

fn ending(cls: VCls, ten: Ten, per: Pers) -> &'static str {
    match (cls, ten, per) {
        (VCls::Ar, Ten::Pres, Pers::I) => "o",
        (VCls::Ar, Ten::Pres, Pers::You) => "as",
        (VCls::Ar, Ten::Pres, Pers::He) => "a",
        (VCls::Ar, Ten::Pres, Pers::We) => "amos",
        (VCls::Ar, Ten::Pres, Pers::They) => "an",
        (VCls::Ar, Ten::Pret, Pers::I) => "é",
        (VCls::Ar, Ten::Pret, Pers::You) => "aste",
        (VCls::Ar, Ten::Pret, Pers::He) => "ó",
        (VCls::Ar, Ten::Pret, Pers::We) => "amos",
        (VCls::Ar, Ten::Pret, Pers::They) => "aron",
        (_, Ten::Pres, Pers::I) => "o",
        (_, Ten::Pres, Pers::You) => "es",
        (_, Ten::Pres, Pers::He) => "e",
        (VCls::Er, Ten::Pres, Pers::We) => "emos",
        (VCls::Ir, Ten::Pres, Pers::We) => "imos",
        (_, Ten::Pres, Pers::They) => "en",
        (_, Ten::Pret, Pers::I) => "í",
        (_, Ten::Pret, Pers::You) => "iste",
        (_, Ten::Pret, Pers::He) => "ió",
        (_, Ten::Pret, Pers::We) => "imos",
        (_, Ten::Pret, Pers::They) => "ieron",
        _ => "",
    }
}

fn spell_pret_yo(stem: &mut String) {
    if stem.ends_with('c') {
        stem.pop();
        stem.push_str("qu");
    } else if stem.ends_with('g') {
        stem.push('u');
    } else if stem.ends_with('z') {
        stem.pop();
        stem.push('c');
    }
}

fn zco_stem(stem: &str) -> String {
    let mut s = String::from(stem);
    if s.ends_with('c') {
        s.pop();
    }
    s.push_str("zc");
    s
}

fn apply_stem(out: &mut String, stem: &str, code: u8) {
    match code {
        STEM_IE => out.push_str(&swap_last(stem, 'e', "ie")),
        STEM_UE => out.push_str(&swap_last(stem, 'o', "ue")),
        STEM_EI => out.push_str(&swap_last(stem, 'e', "i")),
        _ => out.push_str(stem),
    }
}

fn swap_last(stem: &str, vowel: char, repl: &str) -> String {
    let mut at = None;
    for (i, ch) in stem.char_indices() {
        if ch == vowel {
            at = Some(i);
        }
    }
    let Some(i) = at else {
        return stem.to_string();
    };
    let mut s = String::with_capacity(stem.len() + repl.len());
    s.push_str(&stem[..i]);
    s.push_str(repl);
    s.push_str(&stem[i + vowel.len_utf8()..]);
    s
}

fn accent_last_vowel(s: &mut String) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut at = None;
    let mut ch_at = ' ';
    while i < bytes.len() {
        let ch = s[i..].chars().next().unwrap();
        if is_plain_vowel(ch) {
            at = Some(i);
            ch_at = ch;
        }
        i += ch.len_utf8();
    }
    let Some(i) = at else {
        return;
    };
    let accented = match ch_at {
        'a' => 'á',
        'e' => 'é',
        'i' => 'í',
        'o' => 'ó',
        'u' => 'ú',
        _ => return,
    };
    let rest = s[i + ch_at.len_utf8()..].to_string();
    s.truncate(i);
    s.push(accented);
    s.push_str(&rest);
}

fn is_plain_vowel(ch: char) -> bool {
    matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u')
}

fn split_inf(inf: &str) -> (&str, VCls) {
    let cls = class_of(inf);
    let n = inf.len();
    if n >= 2 {
        (&inf[..n - 2], cls)
    } else {
        (inf, cls)
    }
}

pub fn noun_pl(sg: &str, same: bool, explicit: Option<&str>) -> String {
    if let Some(p) = explicit {
        return p.to_string();
    }
    if same {
        return sg.to_string();
    }
    if sg.ends_with('z') {
        let mut s = sg.to_string();
        s.pop();
        s.push_str("ces");
        return s;
    }
    let accent = last_accent(sg);
    if sg.ends_with('s') && accent {
        let mut s = unaccent(sg);
        s.push_str("es");
        return s;
    }
    if ends_vowel(sg) {
        let mut s = if accent { unaccent(sg) } else { sg.to_string() };
        s.push('s');
        return s;
    }
    let mut s = if accent { unaccent(sg) } else { sg.to_string() };
    s.push_str("es");
    s
}

fn last_accent(s: &str) -> bool {
    let mut saw = false;
    for ch in s.chars() {
        if is_accented_vowel(ch) {
            saw = true;
        } else if is_plain_vowel(ch) {
            saw = false;
        }
    }
    saw
}

fn is_accented_vowel(ch: char) -> bool {
    matches!(ch, 'á' | 'é' | 'í' | 'ó' | 'ú')
}

fn unaccent(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        out.push(match ch {
            'á' => 'a',
            'é' => 'e',
            'í' => 'i',
            'ó' => 'o',
            'ú' => 'u',
            other => other,
        });
    }
    out
}

fn ends_vowel(s: &str) -> bool {
    match s.chars().last() {
        Some(ch) => is_plain_vowel(ch) || is_accented_vowel(ch),
        None => false,
    }
}

/// Masculine singular lemma to the requested agreement.
pub fn agree_adj(ms: &str, kind: u8, gen_f: bool, plural: bool) -> String {
    if kind == AT {
        return ms.to_string();
    }
    if kind == AI || !ms.ends_with('o') {
        if plural {
            noun_pl(ms, false, None)
        } else {
            ms.to_string()
        }
    } else {
        let stem = &ms[..ms.len() - 1];
        let mut s = String::from(stem);
        if gen_f {
            s.push('a');
        } else {
            s.push('o');
        }
        if plural {
            s.push('s');
        }
        s
    }
}

pub fn copula(estar: bool, past: bool, per: Pers) -> &'static str {
    match (estar, past, per) {
        (true, false, Pers::I) => "estoy",
        (true, false, Pers::You) => "estás",
        (true, false, Pers::He) => "está",
        (true, false, Pers::We) => "estamos",
        (true, false, Pers::They) => "están",
        (true, true, Pers::I) => "estaba",
        (true, true, Pers::You) => "estabas",
        (true, true, Pers::He) => "estaba",
        (true, true, Pers::We) => "estábamos",
        (true, true, Pers::They) => "estaban",
        (false, false, Pers::I) => "soy",
        (false, false, Pers::You) => "eres",
        (false, false, Pers::He) => "es",
        (false, false, Pers::We) => "somos",
        (false, false, Pers::They) => "son",
        (false, true, Pers::I) => "era",
        (false, true, Pers::You) => "eras",
        (false, true, Pers::He) => "era",
        (false, true, Pers::We) => "éramos",
        (false, true, Pers::They) => "eran",
    }
}

fn irregular(inf: &str, ten: Ten, per: Pers) -> Option<&'static str> {
    let i = per_index(per);
    match inf {
        "decir" => decir(ten, i),
        "hacer" => hacer(ten, i),
        "ir" => ir(ten, i),
        "poder" => poder(ten, i),
        "tener" => tener_like("ten", ten, i),
        "mantener" => tener_like("manten", ten, i),
        "obtener" => tener_like("obten", ten, i),
        "venir" => venir(ten, i),
        "poner" => poner_like("pon", ten, i),
        "salir" => salir(ten, i),
        "ver" => ver(ten, i),
        "dar" => dar(ten, i),
        "saber" => saber(ten, i),
        "traer" => traer(ten, i),
        "querer" => querer(ten, i),
        "oír" => oir(ten, i),
        "leer" => leer(ten, i),
        "caer" => caer(ten, i),
        "construir" => uy_verb("constru", ten, i),
        "conducir" => ducir("condu", ten, i),
        "producir" => ducir("produ", ten, i),
        "traducir" => ducir("tradu", ten, i),
        _ => None,
    }
}

fn per_index(per: Pers) -> usize {
    match per {
        Pers::I => 0,
        Pers::You => 1,
        Pers::He => 2,
        Pers::We => 3,
        Pers::They => 4,
    }
}

fn pick(
    ten: Ten,
    i: usize,
    pres: [&'static str; 5],
    pret: [&'static str; 5],
    ger: &'static str,
    imp: &'static str,
) -> Option<&'static str> {
    Some(match ten {
        Ten::Pres => pres[i],
        Ten::Pret => pret[i],
        Ten::Ger => ger,
        Ten::Imp => imp,
        Ten::Inf | Ten::Imperf => return None,
    })
}

fn decir(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["digo", "dices", "dice", "decimos", "dicen"],
        ["dije", "dijiste", "dijo", "dijimos", "dijeron"],
        "diciendo",
        "di",
    )
}

fn hacer(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["hago", "haces", "hace", "hacemos", "hacen"],
        ["hice", "hiciste", "hizo", "hicimos", "hicieron"],
        "haciendo",
        "haz",
    )
}

fn ir(ten: Ten, i: usize) -> Option<&'static str> {
    match ten {
        Ten::Imperf => Some(["iba", "ibas", "iba", "íbamos", "iban"][i]),
        _ => pick(
            ten,
            i,
            ["voy", "vas", "va", "vamos", "van"],
            ["fui", "fuiste", "fue", "fuimos", "fueron"],
            "yendo",
            "ve",
        ),
    }
}

fn poder(ten: Ten, i: usize) -> Option<&'static str> {
    match ten {
        Ten::Imperf => Some(["podía", "podías", "podía", "podíamos", "podían"][i]),
        _ => pick(
            ten,
            i,
            ["puedo", "puedes", "puede", "podemos", "pueden"],
            ["pude", "pudiste", "pudo", "pudimos", "pudieron"],
            "pudiendo",
            "puede",
        ),
    }
}

fn tener_like(stem: &str, ten: Ten, i: usize) -> Option<&'static str> {
    // Static only for the three stems we ship. Built by matching the stem.
    match stem {
        "ten" => pick(
            ten,
            i,
            ["tengo", "tienes", "tiene", "tenemos", "tienen"],
            ["tuve", "tuviste", "tuvo", "tuvimos", "tuvieron"],
            "teniendo",
            "ten",
        ),
        "manten" => pick(
            ten,
            i,
            [
                "mantengo",
                "mantienes",
                "mantiene",
                "mantenemos",
                "mantienen",
            ],
            [
                "mantuve",
                "mantuviste",
                "mantuvo",
                "mantuvimos",
                "mantuvieron",
            ],
            "manteniendo",
            "mantén",
        ),
        "obten" => pick(
            ten,
            i,
            ["obtengo", "obtienes", "obtiene", "obtenemos", "obtienen"],
            ["obtuve", "obtuviste", "obtuvo", "obtuvimos", "obtuvieron"],
            "obteniendo",
            "obtén",
        ),
        _ => None,
    }
}

fn venir(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["vengo", "vienes", "viene", "venimos", "vienen"],
        ["vine", "viniste", "vino", "vinimos", "vinieron"],
        "viniendo",
        "ven",
    )
}

fn poner_like(stem: &str, ten: Ten, i: usize) -> Option<&'static str> {
    match stem {
        "pon" => pick(
            ten,
            i,
            ["pongo", "pones", "pone", "ponemos", "ponen"],
            ["puse", "pusiste", "puso", "pusimos", "pusieron"],
            "poniendo",
            "pon",
        ),
        _ => None,
    }
}

fn salir(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["salgo", "sales", "sale", "salimos", "salen"],
        ["salí", "saliste", "salió", "salimos", "salieron"],
        "saliendo",
        "sal",
    )
}

fn ver(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["veo", "ves", "ve", "vemos", "ven"],
        ["vi", "viste", "vio", "vimos", "vieron"],
        "viendo",
        "ve",
    )
}

fn dar(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["doy", "das", "da", "damos", "dan"],
        ["di", "diste", "dio", "dimos", "dieron"],
        "dando",
        "da",
    )
}

fn saber(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["sé", "sabes", "sabe", "sabemos", "saben"],
        ["supe", "supiste", "supo", "supimos", "supieron"],
        "sabiendo",
        "sabe",
    )
}

fn traer(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["traigo", "traes", "trae", "traemos", "traen"],
        ["traje", "trajiste", "trajo", "trajimos", "trajeron"],
        "trayendo",
        "trae",
    )
}

fn querer(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["quiero", "quieres", "quiere", "queremos", "quieren"],
        ["quise", "quisiste", "quiso", "quisimos", "quisieron"],
        "queriendo",
        "quiere",
    )
}

fn oir(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["oigo", "oyes", "oye", "oímos", "oyen"],
        ["oí", "oíste", "oyó", "oímos", "oyeron"],
        "oyendo",
        "oye",
    )
}

fn leer(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["leo", "lees", "lee", "leemos", "leen"],
        ["leí", "leíste", "leyó", "leímos", "leyeron"],
        "leyendo",
        "lee",
    )
}

fn caer(ten: Ten, i: usize) -> Option<&'static str> {
    pick(
        ten,
        i,
        ["caigo", "caes", "cae", "caemos", "caen"],
        ["caí", "caíste", "cayó", "caímos", "cayeron"],
        "cayendo",
        "cae",
    )
}

fn uy_verb(stem: &str, ten: Ten, i: usize) -> Option<&'static str> {
    match (stem, ten) {
        ("constru", Ten::Pres) => Some(
            [
                "construyo",
                "construyes",
                "construye",
                "construimos",
                "construyen",
            ][i],
        ),
        ("constru", Ten::Pret) => Some(
            [
                "construí",
                "construiste",
                "construyó",
                "construimos",
                "construyeron",
            ][i],
        ),
        ("constru", Ten::Ger) => Some("construyendo"),
        ("constru", Ten::Imp) => Some("construye"),
        _ => None,
    }
}

fn ducir(stem: &str, ten: Ten, i: usize) -> Option<&'static str> {
    match (stem, ten) {
        ("condu", Ten::Pres) => {
            Some(["conduzco", "conduces", "conduce", "conducimos", "conducen"][i])
        }
        ("condu", Ten::Pret) => Some(
            [
                "conduje",
                "condujiste",
                "condujo",
                "condujimos",
                "condujeron",
            ][i],
        ),
        ("condu", Ten::Ger) => Some("conduciendo"),
        ("condu", Ten::Imp) => Some("conduce"),
        ("produ", Ten::Pres) => {
            Some(["produzco", "produces", "produce", "producimos", "producen"][i])
        }
        ("produ", Ten::Pret) => Some(
            [
                "produje",
                "produjiste",
                "produjo",
                "produjimos",
                "produjeron",
            ][i],
        ),
        ("produ", Ten::Ger) => Some("produciendo"),
        ("produ", Ten::Imp) => Some("produce"),
        ("tradu", Ten::Pres) => {
            Some(["traduzco", "traduces", "traduce", "traducimos", "traducen"][i])
        }
        ("tradu", Ten::Pret) => Some(
            [
                "traduje",
                "tradujiste",
                "tradujo",
                "tradujimos",
                "tradujeron",
            ][i],
        ),
        ("tradu", Ten::Ger) => Some("traduciendo"),
        ("tradu", Ten::Imp) => Some("traduce"),
        _ => None,
    }
}

pub fn en_past(lemma: &str) -> String {
    if let Some(p) = en_irr_past(lemma) {
        return p.to_string();
    }
    let mut s = String::new();
    if lemma.ends_with('e') {
        s.push_str(lemma);
        s.push('d');
        return s;
    }
    if lemma.ends_with('y') && lemma.len() > 1 {
        let pre = &lemma[..lemma.len() - 1];
        let prev = pre.chars().last().unwrap_or('a');
        if !matches!(prev, 'a' | 'e' | 'i' | 'o' | 'u') {
            s.push_str(pre);
            s.push_str("ied");
            return s;
        }
    }
    if double_final(lemma) {
        s.push_str(lemma);
        s.push(lemma.chars().last().unwrap());
        s.push_str("ed");
        return s;
    }
    s.push_str(lemma);
    s.push_str("ed");
    s
}

pub fn en_s3(lemma: &str) -> String {
    if let Some(p) = en_irr_s3(lemma) {
        return p.to_string();
    }
    let mut s = String::new();
    if lemma.ends_with('y') && lemma.len() > 1 {
        let pre = &lemma[..lemma.len() - 1];
        let prev = pre.chars().last().unwrap_or('a');
        if !matches!(prev, 'a' | 'e' | 'i' | 'o' | 'u') {
            s.push_str(pre);
            s.push_str("ies");
            return s;
        }
    }
    if ends_sibilant(lemma) {
        s.push_str(lemma);
        s.push_str("es");
        return s;
    }
    s.push_str(lemma);
    s.push('s');
    s
}

pub fn en_ger(lemma: &str) -> String {
    if lemma == "be" {
        return "being".to_string();
    }
    let mut s = String::new();
    if lemma.ends_with('e') && !lemma.ends_with("ee") && lemma != "be" {
        let stem = &lemma[..lemma.len() - 1];
        if stem.ends_with('i') && stem.len() > 1 {
            // lie → lying
            s.push_str(&stem[..stem.len() - 1]);
            s.push('y');
            s.push_str("ing");
            return s;
        }
        s.push_str(stem);
        s.push_str("ing");
        return s;
    }
    if double_final(lemma) {
        s.push_str(lemma);
        s.push(lemma.chars().last().unwrap());
        s.push_str("ing");
        return s;
    }
    s.push_str(lemma);
    s.push_str("ing");
    s
}

fn ends_sibilant(w: &str) -> bool {
    w.ends_with('s')
        || w.ends_with('x')
        || w.ends_with('z')
        || w.ends_with("ch")
        || w.ends_with("sh")
}

fn double_final(w: &str) -> bool {
    let chars: Vec<char> = w.chars().collect();
    let n = chars.len();
    if !(3..=4).contains(&n) {
        return false;
    }
    let mut vowels = 0;
    let mut i = 0;
    while i < n {
        if matches!(chars[i], 'a' | 'e' | 'i' | 'o' | 'u') {
            vowels += 1;
        }
        i += 1;
    }
    if vowels != 1 {
        return false;
    }
    let a = chars[n - 1];
    let b = chars[n - 2];
    let c = chars[n - 3];
    is_consonant(a)
        && !matches!(a, 'w' | 'x' | 'y')
        && matches!(b, 'a' | 'e' | 'i' | 'o' | 'u')
        && is_consonant(c)
}

fn is_consonant(ch: char) -> bool {
    ch.is_ascii_alphabetic() && !matches!(ch, 'a' | 'e' | 'i' | 'o' | 'u')
}

fn en_irr_past(lemma: &str) -> Option<&'static str> {
    Some(match lemma {
        "be" => "was",
        "become" => "became",
        "begin" => "began",
        "break" => "broke",
        "bring" => "brought",
        "buy" => "bought",
        "can" => "could",
        "come" => "came",
        "do" => "did",
        "drink" => "drank",
        "drive" => "drove",
        "eat" => "ate",
        "fall" => "fell",
        "feel" => "felt",
        "find" => "found",
        "forget" => "forgot",
        "get" => "got",
        "give" => "gave",
        "go" => "went",
        "have" => "had",
        "hear" => "heard",
        "hold" => "held",
        "keep" => "kept",
        "know" => "knew",
        "lead" => "led",
        "leave" => "left",
        "lose" => "lost",
        "make" => "made",
        "mean" => "meant",
        "meet" => "met",
        "pay" => "paid",
        "put" => "put",
        "read" => "read",
        "run" => "ran",
        "say" => "said",
        "see" => "saw",
        "sell" => "sold",
        "send" => "sent",
        "sit" => "sat",
        "sleep" => "slept",
        "speak" => "spoke",
        "spend" => "spent",
        "stand" => "stood",
        "take" => "took",
        "teach" => "taught",
        "tell" => "told",
        "think" => "thought",
        "understand" => "understood",
        "wake" => "woke",
        "wear" => "wore",
        "win" => "won",
        "write" => "wrote",
        _ => return None,
    })
}

fn en_irr_s3(lemma: &str) -> Option<&'static str> {
    Some(match lemma {
        "be" => "is",
        "can" => "can",
        "do" => "does",
        "go" => "goes",
        "have" => "has",
        "will" => "will",
        _ => return None,
    })
}

#[allow(dead_code)]
pub fn en_be(past: bool, were: bool, per_they_you_we: bool) -> &'static str {
    if !past {
        return "is";
    }
    if were || per_they_you_we {
        "were"
    } else {
        "was"
    }
}
