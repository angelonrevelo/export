//! Part-of-speech tags shared by the lexicon compiler and the runtime.
//! Function words and auxiliaries do not count as translated content.

pub const NM: u8 = 1;
pub const NF: u8 = 2;
pub const VAR: u8 = 3;
pub const VER: u8 = 4;
pub const VIR: u8 = 5;
pub const AO: u8 = 6;
pub const AE: u8 = 7;
pub const AI: u8 = 8;
pub const AT: u8 = 9;
pub const AV: u8 = 10;
pub const FN: u8 = 11;
pub const AU: u8 = 12;
pub const CW: u8 = 13;
pub const VI: u8 = 14;

pub const STEM_NONE: u8 = 0;
pub const STEM_IE: u8 = 1;
pub const STEM_UE: u8 = 2;
pub const STEM_EI: u8 = 3;
pub const STEM_IAR: u8 = 4;
pub const STEM_UAR: u8 = 5;
pub const STEM_ZCO: u8 = 6;

pub const TEN_NONE: u8 = 0;
pub const TEN_PRES: u8 = 1;
pub const TEN_PRET: u8 = 2;
pub const TEN_GER: u8 = 3;
pub const TEN_INF: u8 = 4;
pub const TEN_IMP: u8 = 5;
pub const TEN_IMPERF: u8 = 6;

pub const PER_NONE: u8 = 0;
pub const PER_I: u8 = 1;
pub const PER_YOU: u8 = 2;
pub const PER_HE: u8 = 3;
pub const PER_WE: u8 = 4;
pub const PER_THEY: u8 = 5;

pub const NUM_NONE: u8 = 0;
pub const NUM_SG: u8 = 1;
pub const NUM_PL: u8 = 2;

pub const GEN_NONE: u8 = 0;
pub const GEN_M: u8 = 1;
pub const GEN_F: u8 = 2;

#[inline]
pub fn is_content(kind: u8) -> bool {
    matches!(
        kind,
        NM | NF | VAR | VER | VIR | AO | AE | AI | AT | AV | CW | VI
    )
}

#[inline]
pub fn is_verb(kind: u8) -> bool {
    matches!(kind, VAR | VER | VIR | VI)
}

#[inline]
pub fn is_noun(kind: u8) -> bool {
    matches!(kind, NM | NF)
}

#[inline]
pub fn is_adj(kind: u8) -> bool {
    matches!(kind, AO | AE | AI | AT)
}
