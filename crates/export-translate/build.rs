//! Compiles data/freq.tsv into a sorted lemma table and a Spanish surface index.
//! This runs on the host. The library still has zero dependencies.

#[allow(dead_code)]
#[path = "src/esconj.rs"]
mod esconj;
#[allow(dead_code)]
#[path = "src/kind.rs"]
mod kind;

use std::env;
use std::fs;
use std::path::PathBuf;

use esconj::{class_of, conj_into, noun_pl, Pers, Ten, VCls};
use kind::*;

struct Row {
    en: String,
    es: String,
    kind: u8,
    stem: u8,
    samepl: bool,
    nogen: bool,
    pl: Option<String>,
}

fn main() {
    println!("cargo:rerun-if-changed=data/freq.tsv");
    println!("cargo:rerun-if-changed=src/esconj.rs");
    println!("cargo:rerun-if-changed=src/kind.rs");
    let mut path = PathBuf::from("data/freq.tsv");
    if !path.exists() {
        path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("data/freq.tsv");
    }
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let rows = parse(&text);
    let mut lemmas = rows;
    let surfs = surfaces(&lemmas);
    lemmas.sort_by(|a, b| a.en.cmp(&b.en));
    let mut i = 1;
    while i < lemmas.len() {
        if lemmas[i - 1].en == lemmas[i].en {
            panic!("duplicate English lemma {}", lemmas[i].en);
        }
        i += 1;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("frequent.rs");
    fs::write(&dest, emit(&lemmas, &surfs)).unwrap();
}

struct Surf {
    es: String,
    en: String,
    kind: u8,
    ten: u8,
    per: u8,
    num: u8,
    gen: u8,
}

fn parse(text: &str) -> Vec<Row> {
    let mut rows = Vec::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let cols: Vec<&str> = line.split('\t').collect();
        if cols.len() < 3 {
            panic!("line {}: expected en, es, kind", n + 1);
        }
        let kind = kind_of(cols[2], n + 1);
        let flags = if cols.len() >= 4 { cols[3] } else { "" };
        let mut stem = STEM_NONE;
        let mut samepl = false;
        let mut nogen = false;
        let mut pl = None;
        for flag in flags.split(',') {
            let flag = flag.trim();
            if flag.is_empty() {
                continue;
            }
            match flag {
                "ie" => stem = STEM_IE,
                "ue" => stem = STEM_UE,
                "ei" => stem = STEM_EI,
                "iar" => stem = STEM_IAR,
                "uar" => stem = STEM_UAR,
                "zco" => stem = STEM_ZCO,
                "samepl" => samepl = true,
                "nogen" => nogen = true,
                other if other.starts_with("pl=") => pl = Some(other[3..].to_string()),
                other => panic!("line {}: unknown flag {other}", n + 1),
            }
        }
        rows.push(Row {
            en: cols[0].to_string(),
            es: cols[1].to_string(),
            kind,
            stem,
            samepl,
            nogen,
            pl,
        });
    }
    rows
}

fn kind_of(s: &str, line: usize) -> u8 {
    match s {
        "nm" => NM,
        "nf" => NF,
        "var" => VAR,
        "ver" => VER,
        "vir" => VIR,
        "vi" => VI,
        "ao" => AO,
        "ae" => AE,
        "ai" => AI,
        "at" => AT,
        "av" => AV,
        "fn" => FN,
        "au" => AU,
        "cw" => CW,
        other => panic!("line {line}: unknown kind {other}"),
    }
}

fn surfaces(rows: &[Row]) -> Vec<Surf> {
    let mut out = Vec::new();
    let mut seen = Vec::<String>::new();
    let mut buf = String::new();
    for row in rows {
        if row.nogen {
            continue;
        }
        match row.kind {
            NM | NF => {
                add(
                    &mut out,
                    &mut seen,
                    &row.es,
                    &row.en,
                    row.kind,
                    TEN_NONE,
                    PER_NONE,
                    NUM_SG,
                    gen_of(row.kind),
                );
                let pl = noun_pl(&row.es, row.samepl, row.pl.as_deref());
                if pl != row.es {
                    add(
                        &mut out,
                        &mut seen,
                        &pl,
                        &row.en,
                        row.kind,
                        TEN_NONE,
                        PER_NONE,
                        NUM_PL,
                        gen_of(row.kind),
                    );
                }
            }
            AO | AE | AI | AT => add_adj(&mut out, &mut seen, row),
            VAR | VER | VIR | VI | AU => add_verb(&mut out, &mut seen, row, &mut buf),
            _ => add_extra(&mut out, &mut seen, row),
        }
    }
    add_copula(&mut out, &mut seen);
    out.sort_by(|a, b| a.es.cmp(&b.es));
    let mut i = 1;
    while i < out.len() {
        if out[i - 1].es == out[i].es {
            panic!("surface sort failed on {}", out[i].es);
        }
        i += 1;
    }
    out
}

fn gen_of(kind: u8) -> u8 {
    if kind == NF {
        GEN_F
    } else if kind == NM {
        GEN_M
    } else {
        GEN_NONE
    }
}

fn add_adj(out: &mut Vec<Surf>, seen: &mut Vec<String>, row: &Row) {
    if row.kind == AT {
        add(
            out, seen, &row.es, &row.en, row.kind, TEN_NONE, PER_NONE, NUM_SG, GEN_NONE,
        );
        return;
    }
    if row.pl.is_some() || row.kind == AI || !row.es.ends_with('o') {
        add(
            out, seen, &row.es, &row.en, row.kind, TEN_NONE, PER_NONE, NUM_SG, GEN_NONE,
        );
        let pl = noun_pl(&row.es, false, row.pl.as_deref());
        if pl != row.es {
            add(
                out, seen, &pl, &row.en, row.kind, TEN_NONE, PER_NONE, NUM_PL, GEN_NONE,
            );
        }
        return;
    }
    let forms = adj_forms(&row.es, row.kind);
    for (form, gen, num) in forms {
        add(
            out, seen, &form, &row.en, row.kind, TEN_NONE, PER_NONE, num, gen,
        );
    }
}

fn adj_forms(ms: &str, kind: u8) -> Vec<(String, u8, u8)> {
    if kind == AI || !ms.ends_with('o') {
        let pl = noun_pl(ms, false, None);
        let mut v = vec![(ms.to_string(), GEN_NONE, NUM_SG)];
        if pl != ms {
            v.push((pl, GEN_NONE, NUM_PL));
        }
        return v;
    }
    let stem = &ms[..ms.len() - 1];
    vec![
        (format!("{stem}o"), GEN_M, NUM_SG),
        (format!("{stem}a"), GEN_F, NUM_SG),
        (format!("{stem}os"), GEN_M, NUM_PL),
        (format!("{stem}as"), GEN_F, NUM_PL),
    ]
}

fn add_verb(out: &mut Vec<Surf>, seen: &mut Vec<String>, row: &Row, buf: &mut String) {
    let inf = &row.es;
    let stem = if row.kind == VI || row.kind == AU {
        STEM_NONE
    } else {
        row.stem
    };
    // AU uses the Spanish infinitive the same way a lexical verb does.
    let _cls: VCls = class_of(inf);
    conj_into(
        buf,
        inf,
        if row.kind == VI { STEM_NONE } else { stem },
        Ten::Inf,
        Pers::He,
    );
    // Irregular tables return None for the infinitive; conj_into then copies it.
    add(
        out, seen, inf, &row.en, row.kind, TEN_INF, PER_NONE, NUM_NONE, GEN_NONE,
    );
    conj_into(buf, inf, stem_of(row), Ten::Ger, Pers::He);
    add(
        out, seen, buf, &row.en, row.kind, TEN_GER, PER_NONE, NUM_NONE, GEN_NONE,
    );
    conj_into(buf, inf, stem_of(row), Ten::Imp, Pers::He);
    add(
        out, seen, buf, &row.en, row.kind, TEN_IMP, PER_YOU, NUM_NONE, GEN_NONE,
    );
    for (ten, code) in [(Ten::Pres, TEN_PRES), (Ten::Pret, TEN_PRET)] {
        for per in Pers::all() {
            conj_into(buf, inf, stem_of(row), ten, per);
            add(
                out,
                seen,
                buf,
                &row.en,
                row.kind,
                code,
                per.code(),
                NUM_NONE,
                GEN_NONE,
            );
        }
    }
    if inf == "poder" || inf == "ir" {
        for per in Pers::all() {
            conj_into(buf, inf, stem_of(row), Ten::Imperf, per);
            add(
                out,
                seen,
                buf,
                &row.en,
                row.kind,
                TEN_IMPERF,
                per.code(),
                NUM_NONE,
                GEN_NONE,
            );
        }
    }
}

fn stem_of(row: &Row) -> u8 {
    if row.kind == VI {
        STEM_NONE
    } else {
        row.stem
    }
}

fn add_extra(out: &mut Vec<Surf>, seen: &mut Vec<String>, row: &Row) {
    let forms: Vec<(String, u8, u8)> = match row.en.as_str() {
        "this" => pairs(&["este", "esta", "estos", "estas"]),
        "that" => pairs(&["que", "ese", "esa", "esos", "esas"]),
        "these" => pairs(&["estos", "estas"]),
        "those" => pairs(&["esos", "esas"]),
        "our" => pairs(&["nuestro", "nuestra", "nuestros", "nuestras"]),
        "my" => pairs(&["mi", "mis"]),
        "your" => pairs(&["tu", "tus"]),
        "his" | "her" | "their" | "its" => pairs(&["su", "sus"]),
        _ => {
            if row.es.contains(' ') {
                return;
            }
            vec![(row.es.clone(), GEN_NONE, NUM_NONE)]
        }
    };
    for (form, gen, num) in forms {
        add(
            out, seen, &form, &row.en, row.kind, TEN_NONE, PER_NONE, num, gen,
        );
    }
}

fn pairs(forms: &[&str]) -> Vec<(String, u8, u8)> {
    let mut v = Vec::new();
    for f in forms {
        v.push(((*f).to_string(), GEN_NONE, NUM_NONE));
    }
    v
}

fn add_copula(out: &mut Vec<Surf>, seen: &mut Vec<String>) {
    let estar = [
        ("estaba", PER_HE),
        ("estabas", PER_YOU),
        ("estábamos", PER_WE),
        ("estaban", PER_THEY),
    ];
    let ser = [
        ("era", PER_HE),
        ("eras", PER_YOU),
        ("éramos", PER_WE),
        ("eran", PER_THEY),
    ];
    for (form, per) in estar {
        add(
            out, seen, form, "be", AU, TEN_IMPERF, per, NUM_NONE, GEN_NONE,
        );
    }
    for (form, per) in ser {
        add(
            out, seen, form, "be", AU, TEN_IMPERF, per, NUM_NONE, GEN_NONE,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn add(
    out: &mut Vec<Surf>,
    seen: &mut Vec<String>,
    es: &str,
    en: &str,
    kind: u8,
    ten: u8,
    per: u8,
    num: u8,
    gen: u8,
) {
    if es.is_empty() || es.contains(' ') {
        return;
    }
    let mut i = 0;
    while i < seen.len() {
        if seen[i] == es {
            return;
        }
        i += 1;
    }
    seen.push(es.to_string());
    out.push(Surf {
        es: es.to_string(),
        en: en.to_string(),
        kind,
        ten,
        per,
        num,
        gen,
    });
}

fn emit(lemmas: &[Row], surfs: &[Surf]) -> String {
    let mut s = String::new();
    s.push_str("// Generated by build.rs from data/freq.tsv. Do not edit.\n");
    s.push_str("#[derive(Clone, Copy)]\n");
    s.push_str("pub struct Lem {\n");
    s.push_str("    pub en: &'static str,\n");
    s.push_str("    pub es: &'static str,\n");
    s.push_str("    pub kind: u8,\n");
    s.push_str("    pub stem: u8,\n");
    s.push_str("    pub samepl: bool,\n");
    s.push_str("    pub pl: &'static str,\n");
    s.push_str("}\n");
    s.push_str("#[derive(Clone, Copy)]\n");
    s.push_str("#[allow(dead_code)]\n");
    s.push_str("pub struct Surf {\n");
    s.push_str("    pub es: &'static str,\n");
    s.push_str("    pub en: &'static str,\n");
    s.push_str("    pub kind: u8,\n");
    s.push_str("    pub ten: u8,\n");
    s.push_str("    pub per: u8,\n");
    s.push_str("    pub num: u8,\n");
    s.push_str("    pub gen: u8,\n");
    s.push_str("}\n");
    s.push_str("pub static LEMMAS: &[Lem] = &[\n");
    for row in lemmas {
        let pl = row.pl.as_deref().unwrap_or("");
        s.push_str(&format!(
            "    Lem {{ en: \"{}\", es: \"{}\", kind: {}, stem: {}, samepl: {}, pl: \"{}\" }},\n",
            esc(&row.en),
            esc(&row.es),
            row.kind,
            row.stem,
            row.samepl,
            esc(pl)
        ));
    }
    s.push_str("];\n");
    s.push_str("pub static ES_SURF: &[Surf] = &[\n");
    for row in surfs {
        s.push_str(&format!(
            "    Surf {{ es: \"{}\", en: \"{}\", kind: {}, ten: {}, per: {}, num: {}, gen: {} }},\n",
            esc(&row.es),
            esc(&row.en),
            row.kind,
            row.ten,
            row.per,
            row.num,
            row.gen
        ));
    }
    s.push_str("];\n");
    s.push_str(
        r#"
pub fn find_lemma(key: &str) -> Option<&'static Lem> {
    debug_assert!(lemmas_sorted());
    find(LEMMAS, key, |e| e.en)
}

pub fn find_surf(key: &str) -> Option<&'static Surf> {
    debug_assert!(surfs_sorted());
    find(ES_SURF, key, |e| e.es)
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

fn lemmas_sorted() -> bool {
    let mut i = 1;
    while i < LEMMAS.len() {
        if LEMMAS[i - 1].en >= LEMMAS[i].en {
            return false;
        }
        i += 1;
    }
    true
}

fn surfs_sorted() -> bool {
    let mut i = 1;
    while i < ES_SURF.len() {
        if ES_SURF[i - 1].es >= ES_SURF[i].es {
            return false;
        }
        i += 1;
    }
    true
}

pub fn static_bytes() -> usize {
    let mut n = core::mem::size_of_val(LEMMAS) + core::mem::size_of_val(ES_SURF);
    let mut i = 0;
    while i < LEMMAS.len() {
        n += LEMMAS[i].en.len() + LEMMAS[i].es.len() + LEMMAS[i].pl.len();
        i += 1;
    }
    i = 0;
    while i < ES_SURF.len() {
        n += ES_SURF[i].es.len() + ES_SURF[i].en.len();
        i += 1;
    }
    n
}

#[allow(dead_code)]
pub fn lemma_count() -> usize {
    LEMMAS.len()
}

#[allow(dead_code)]
pub fn surface_count() -> usize {
    ES_SURF.len()
}
"#,
    );
    s
}

fn esc(s: &str) -> String {
    let mut o = String::new();
    for ch in s.chars() {
        match ch {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            other => o.push(other),
        }
    }
    o
}
