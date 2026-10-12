//! Compiles data/tl.tsv into OUT_DIR/frequent_tl.rs. Host-only.

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::tlconj::*;

struct Row {
    en: String,
    tl: String,
    kind: u8,
    sub: u8,
    focus_ng: bool,
    comp: bool,
    saobj: bool,
    drop_det: bool,
    plural: bool,
    recip: bool,
    inf: String,
    perf: String,
    impf: String,
    cont: String,
    ang: String,
    gen: String,
    obl: String,
}

struct Surf {
    key: String,
    en: String,
    kind: u8,
    asp: u8,
    seq: u32,
}

pub fn compile() {
    let mut path = PathBuf::from("data/tl.tsv");
    if !path.exists() {
        path = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("data/tl.tsv");
    }
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    let rows = parse(&text);
    let surfs = surfaces(&rows);
    let mut lemmas = rows;
    lemmas.sort_by(|a, b| a.en.cmp(&b.en));
    let mut i = 1;
    while i < lemmas.len() {
        if lemmas[i - 1].en == lemmas[i].en {
            panic!("duplicate Filipino English lemma {}", lemmas[i].en);
        }
        i += 1;
    }
    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let dest = out_dir.join("frequent_tl.rs");
    fs::write(&dest, emit(&lemmas, &surfs)).unwrap();
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
            panic!("tl line {}: expected en, tl, kind", n + 1);
        }
        let kind = kind_of(cols[2], n + 1);
        let flags = if cols.len() >= 4 { cols[3] } else { "" };
        let mut row = Row {
            en: cols[0].to_string(),
            tl: if cols[1] == "*" {
                String::new()
            } else {
                cols[1].to_string()
            },
            kind,
            sub: SUB_NONE,
            focus_ng: false,
            comp: false,
            saobj: false,
            drop_det: false,
            plural: false,
            recip: false,
            inf: String::new(),
            perf: String::new(),
            impf: String::new(),
            cont: String::new(),
            ang: String::new(),
            gen: String::new(),
            obl: String::new(),
        };
        for flag in flags.split(',') {
            let flag = flag.trim();
            if flag.is_empty() {
                continue;
            }
            if let Some(rest) = flag.strip_prefix("inf=") {
                row.inf = rest.to_string();
            } else if let Some(rest) = flag.strip_prefix("perf=") {
                row.perf = rest.to_string();
            } else if let Some(rest) = flag.strip_prefix("impf=") {
                row.impf = rest.to_string();
            } else if let Some(rest) = flag.strip_prefix("cont=") {
                row.cont = rest.to_string();
            } else if let Some(rest) = flag.strip_prefix("ang=") {
                row.ang = rest.to_string();
            } else if let Some(rest) = flag.strip_prefix("gen=") {
                row.gen = rest.to_string();
            } else if let Some(rest) = flag.strip_prefix("obl=") {
                row.obl = rest.to_string();
            } else if let Some(rest) = flag.strip_prefix("sub=") {
                row.sub = sub_of(rest, n + 1);
            } else {
                match flag {
                    "focus=ng" => row.focus_ng = true,
                    "comp" => row.comp = true,
                    "saobj" => row.saobj = true,
                    "drop" => row.drop_det = true,
                    "plural" => row.plural = true,
                    "recip" => row.recip = true,
                    other => panic!("tl line {}: unknown flag {other}", n + 1),
                }
            }
        }
        fill_forms(&mut row, n + 1);
        rows.push(row);
    }
    rows
}

fn fill_forms(row: &mut Row, line: usize) {
    match row.kind {
        K_MAG => {
            let f = mag_forms(&row.tl, row.recip);
            row.inf = f.inf;
            row.perf = f.perf;
            row.impf = f.impf;
            row.cont = f.cont;
        }
        K_UM => {
            let f = um_forms(&row.tl);
            row.inf = f.inf;
            row.perf = f.perf;
            row.impf = f.impf;
            row.cont = f.cont;
        }
        K_IRR => {
            if row.inf.is_empty()
                || row.perf.is_empty()
                || row.impf.is_empty()
                || row.cont.is_empty()
            {
                panic!("tl line {line}: irr {} needs inf, perf, impf, cont", row.en);
            }
        }
        K_PRO => {
            if row.ang.is_empty() {
                row.ang = row.tl.clone();
            }
        }
        _ => {}
    }
}

fn kind_of(s: &str, line: usize) -> u8 {
    match s {
        "n" => K_N,
        "mag" => K_MAG,
        "um" => K_UM,
        "irr" => K_IRR,
        "pseudo" => K_PSEUDO,
        "adj" => K_ADJ,
        "adv" => K_ADV,
        "fn" => K_FN,
        "au" => K_AU,
        "cw" => K_CW,
        "pro" => K_PRO,
        "det" => K_DET,
        "prep" => K_PREP,
        "conj" => K_CONJ,
        "aux" => K_AUX,
        "cop" => K_COP,
        "poss" => K_POSS,
        other => panic!("tl line {line}: unknown kind {other}"),
    }
}

fn sub_of(s: &str, line: usize) -> u8 {
    match s {
        "will" => SUB_WILL,
        "do" => SUB_DO,
        "the" => SUB_THE,
        "a" => SUB_A,
        "not" => SUB_NOT,
        "that" => SUB_THAT,
        other => panic!("tl line {line}: unknown sub {other}"),
    }
}

fn surfaces(rows: &[Row]) -> Vec<Surf> {
    let mut surfs = Vec::new();
    let mut seq = 0u32;
    for row in rows {
        if matches!(row.kind, K_MAG | K_UM | K_IRR) {
            add_form(&mut surfs, &mut seq, &row.inf, &row.en, row.kind, ASP_INF);
            add_form(&mut surfs, &mut seq, &row.perf, &row.en, row.kind, ASP_PERF);
            add_form(&mut surfs, &mut seq, &row.impf, &row.en, row.kind, ASP_IMPF);
            add_form(&mut surfs, &mut seq, &row.cont, &row.en, row.kind, ASP_CONT);
        } else if row.kind == K_PRO {
            add_form(&mut surfs, &mut seq, &row.ang, &row.en, row.kind, ASP_NONE);
            add_form(&mut surfs, &mut seq, &row.gen, &row.en, row.kind, ASP_NONE);
            add_form(&mut surfs, &mut seq, &row.obl, &row.en, row.kind, ASP_NONE);
            add_form(&mut surfs, &mut seq, &row.tl, &row.en, row.kind, ASP_NONE);
        } else if !row.tl.contains(' ') {
            add_form(&mut surfs, &mut seq, &row.tl, &row.en, row.kind, ASP_NONE);
        }
    }
    add_form(&mut surfs, &mut seq, "mga", "", K_PL, ASP_NONE);
    surfs.sort_by(|a, b| a.key.cmp(&b.key).then(a.seq.cmp(&b.seq)));
    let mut out = Vec::new();
    for s in surfs {
        if out.last().map(|p: &Surf| p.key == s.key).unwrap_or(false) {
            continue;
        }
        out.push(s);
    }
    out
}

fn add_form(surfs: &mut Vec<Surf>, seq: &mut u32, tl: &str, en: &str, kind: u8, asp: u8) {
    if tl.is_empty() || tl.contains(' ') {
        return;
    }
    let key = tl.to_ascii_lowercase();
    surfs.push(Surf {
        key,
        en: en.to_string(),
        kind,
        asp,
        seq: *seq,
    });
    *seq += 1;
}

fn emit(lemmas: &[Row], surfs: &[Surf]) -> String {
    let mut o = String::new();
    o.push_str("// Generated from data/tl.tsv. Do not edit.\n");
    o.push_str("#[allow(dead_code)]\n");
    o.push_str("pub struct Lem {\n");
    o.push_str("    pub en: &'static str,\n");
    o.push_str("    pub tl: &'static str,\n");
    o.push_str("    pub kind: u8,\n");
    o.push_str("    pub sub: u8,\n");
    o.push_str("    pub focus_ng: bool,\n");
    o.push_str("    pub comp: bool,\n");
    o.push_str("    pub saobj: bool,\n");
    o.push_str("    pub drop_det: bool,\n");
    o.push_str("    pub plural: bool,\n");
    o.push_str("    pub inf: &'static str,\n");
    o.push_str("    pub perf: &'static str,\n");
    o.push_str("    pub impf: &'static str,\n");
    o.push_str("    pub cont: &'static str,\n");
    o.push_str("    pub ang: &'static str,\n");
    o.push_str("    pub gen: &'static str,\n");
    o.push_str("    pub obl: &'static str,\n");
    o.push_str("}\n\n");
    o.push_str("#[allow(dead_code)]\n");
    o.push_str("pub struct Surf {\n");
    o.push_str("    pub tl: &'static str,\n");
    o.push_str("    pub en: &'static str,\n");
    o.push_str("    pub kind: u8,\n");
    o.push_str("    pub asp: u8,\n");
    o.push_str("}\n\n");
    o.push_str("pub static LEMS: &[Lem] = &[\n");
    for row in lemmas {
        o.push_str("    Lem {\n");
        push_field(&mut o, "en", &row.en);
        push_field(&mut o, "tl", &row.tl);
        o.push_str(&format!("        kind: {},\n", row.kind));
        o.push_str(&format!("        sub: {},\n", row.sub));
        o.push_str(&format!("        focus_ng: {},\n", row.focus_ng));
        o.push_str(&format!("        comp: {},\n", row.comp));
        o.push_str(&format!("        saobj: {},\n", row.saobj));
        o.push_str(&format!("        drop_det: {},\n", row.drop_det));
        o.push_str(&format!("        plural: {},\n", row.plural));
        push_field(&mut o, "inf", &row.inf);
        push_field(&mut o, "perf", &row.perf);
        push_field(&mut o, "impf", &row.impf);
        push_field(&mut o, "cont", &row.cont);
        push_field(&mut o, "ang", &row.ang);
        push_field(&mut o, "gen", &row.gen);
        push_field(&mut o, "obl", &row.obl);
        o.push_str("    },\n");
    }
    o.push_str("];\n\n");
    o.push_str("pub static SURFS: &[Surf] = &[\n");
    for s in surfs {
        o.push_str("    Surf { ");
        o.push_str("tl: ");
        o.push_str(&rust_str(&s.key));
        o.push_str(", en: ");
        o.push_str(&rust_str(&s.en));
        o.push_str(&format!(", kind: {}, asp: {} }},\n", s.kind, s.asp));
    }
    o.push_str("];\n\n");
    o.push_str("pub fn find_lemma(key: &str) -> Option<&'static Lem> {\n");
    o.push_str("    let mut lo = 0usize;\n");
    o.push_str("    let mut hi = LEMS.len();\n");
    o.push_str("    while lo < hi {\n");
    o.push_str("        let mid = lo + (hi - lo) / 2;\n");
    o.push_str("        if LEMS[mid].en < key {\n");
    o.push_str("            lo = mid + 1;\n");
    o.push_str("        } else {\n");
    o.push_str("            hi = mid;\n");
    o.push_str("        }\n");
    o.push_str("    }\n");
    o.push_str("    if lo < LEMS.len() && LEMS[lo].en == key {\n");
    o.push_str("        Some(&LEMS[lo])\n");
    o.push_str("    } else {\n");
    o.push_str("        None\n");
    o.push_str("    }\n");
    o.push_str("}\n\n");
    o.push_str("pub fn find_surf(key: &str) -> Option<&'static Surf> {\n");
    o.push_str("    let mut lo = 0usize;\n");
    o.push_str("    let mut hi = SURFS.len();\n");
    o.push_str("    while lo < hi {\n");
    o.push_str("        let mid = lo + (hi - lo) / 2;\n");
    o.push_str("        if SURFS[mid].tl < key {\n");
    o.push_str("            lo = mid + 1;\n");
    o.push_str("        } else {\n");
    o.push_str("            hi = mid;\n");
    o.push_str("        }\n");
    o.push_str("    }\n");
    o.push_str("    if lo < SURFS.len() && SURFS[lo].tl == key {\n");
    o.push_str("        Some(&SURFS[lo])\n");
    o.push_str("    } else {\n");
    o.push_str("        None\n");
    o.push_str("    }\n");
    o.push_str("}\n\n");
    o.push_str("pub fn lemma_count() -> usize {\n");
    o.push_str("    LEMS.len()\n");
    o.push_str("}\n\n");
    o.push_str("pub fn static_bytes() -> usize {\n");
    o.push_str("    let mut n = core::mem::size_of_val(LEMS) + core::mem::size_of_val(SURFS);\n");
    o.push_str("    let mut i = 0;\n");
    o.push_str("    while i < LEMS.len() {\n");
    o.push_str("        let l = &LEMS[i];\n");
    o.push_str(
        "        n += l.en.len() + l.tl.len() + l.inf.len() + l.perf.len() + l.impf.len()\n",
    );
    o.push_str("            + l.cont.len() + l.ang.len() + l.gen.len() + l.obl.len();\n");
    o.push_str("        i += 1;\n");
    o.push_str("    }\n");
    o.push_str("    i = 0;\n");
    o.push_str("    while i < SURFS.len() {\n");
    o.push_str("        n += SURFS[i].tl.len() + SURFS[i].en.len();\n");
    o.push_str("        i += 1;\n");
    o.push_str("    }\n");
    o.push_str("    n\n");
    o.push_str("}\n");
    o
}

fn push_field(o: &mut String, name: &str, val: &str) {
    o.push_str("        ");
    o.push_str(name);
    o.push_str(": ");
    o.push_str(&rust_str(val));
    o.push_str(",\n");
}

fn rust_str(s: &str) -> String {
    let mut o = String::from("\"");
    for ch in s.chars() {
        match ch {
            '\\' => o.push_str("\\\\"),
            '"' => o.push_str("\\\""),
            _ => o.push(ch),
        }
    }
    o.push('"');
    o
}
