//! Filipino verb shapes used while compiling the lexicon.
//! The library does not call these at runtime; surfaces are stored in the table.

pub const K_N: u8 = 1;
pub const K_MAG: u8 = 2;
pub const K_UM: u8 = 3;
pub const K_IRR: u8 = 4;
pub const K_PSEUDO: u8 = 5;
pub const K_ADJ: u8 = 6;
pub const K_ADV: u8 = 7;
pub const K_FN: u8 = 8;
pub const K_AU: u8 = 9;
pub const K_CW: u8 = 10;
pub const K_PRO: u8 = 11;
pub const K_DET: u8 = 12;
pub const K_PREP: u8 = 13;
pub const K_CONJ: u8 = 14;
pub const K_AUX: u8 = 15;
pub const K_COP: u8 = 16;
pub const K_POSS: u8 = 17;
pub const K_PL: u8 = 18;

pub const SUB_NONE: u8 = 0;
pub const SUB_WILL: u8 = 1;
pub const SUB_DO: u8 = 2;
pub const SUB_THE: u8 = 3;
pub const SUB_A: u8 = 4;
pub const SUB_NOT: u8 = 5;
pub const SUB_THAT: u8 = 6;

pub const ASP_NONE: u8 = 0;
pub const ASP_INF: u8 = 1;
pub const ASP_PERF: u8 = 2;
pub const ASP_IMPF: u8 = 3;
pub const ASP_CONT: u8 = 4;

pub struct Forms {
    pub inf: String,
    pub perf: String,
    pub impf: String,
    pub cont: String,
}

pub fn mag_forms(root: &str, recip: bool) -> Forms {
    if recip {
        return Forms {
            inf: format!("makipag-{root}"),
            perf: format!("nakipag-{root}"),
            impf: format!("nakikipag-{root}"),
            cont: format!("makikipag-{root}"),
        };
    }
    let redup = first_syl(root);
    if starts_vowel(root) {
        Forms {
            inf: format!("mag-{root}"),
            perf: format!("nag-{root}"),
            impf: format!("nag-{redup}{root}"),
            cont: format!("mag-{redup}{root}"),
        }
    } else {
        Forms {
            inf: format!("mag{root}"),
            perf: format!("nag{root}"),
            impf: format!("nag{redup}{root}"),
            cont: format!("mag{redup}{root}"),
        }
    }
}

pub fn um_forms(root: &str) -> Forms {
    if root == "dating" {
        return Forms {
            inf: "dumating".to_string(),
            perf: "dumating".to_string(),
            impf: "dumarating".to_string(),
            cont: "darating".to_string(),
        };
    }
    let mut chars = root.chars();
    let Some(first) = chars.next() else {
        return Forms {
            inf: String::new(),
            perf: String::new(),
            impf: String::new(),
            cont: String::new(),
        };
    };
    if is_vowel(first) {
        let v = first_syl(root);
        return Forms {
            inf: format!("um{root}"),
            perf: format!("um{root}"),
            impf: format!("um{v}{root}"),
            cont: format!("{v}{root}"),
        };
    }
    let onset = first.to_ascii_lowercase();
    let rest = &root[first.len_utf8()..];
    let vowel = first_vowel(root);
    let completed = format!("{onset}um{rest}");
    let redup = first_syl(root);
    Forms {
        inf: completed.clone(),
        perf: completed,
        impf: format!("{onset}um{vowel}{root}"),
        cont: format!("{redup}{root}"),
    }
}

fn starts_vowel(root: &str) -> bool {
    match root.chars().next() {
        Some(ch) => is_vowel(ch),
        None => false,
    }
}

fn is_vowel(ch: char) -> bool {
    matches!(
        ch,
        'a' | 'e' | 'i' | 'o' | 'u' | 'A' | 'E' | 'I' | 'O' | 'U'
    )
}

/// Consonants through the first vowel: trabaho → tra, lakad → la, usap → u.
fn first_syl(root: &str) -> &str {
    let mut end = 0;
    for (i, ch) in root.char_indices() {
        end = i + ch.len_utf8();
        if is_vowel(ch) {
            break;
        }
    }
    if end == 0 {
        root
    } else {
        &root[..end]
    }
}

fn first_vowel(root: &str) -> char {
    for ch in root.chars() {
        if is_vowel(ch) {
            return ch.to_ascii_lowercase();
        }
    }
    'a'
}
