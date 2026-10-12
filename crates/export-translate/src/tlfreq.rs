//! Frequent Filipino table compiled from data/tl.tsv.
//! Kind bytes match `tlconj` in the build script.

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

#[allow(dead_code)]
pub const SUB_NONE: u8 = 0;
pub const SUB_WILL: u8 = 1;
pub const SUB_DO: u8 = 2;
#[allow(dead_code)]
pub const SUB_THE: u8 = 3;
pub const SUB_A: u8 = 4;
pub const SUB_NOT: u8 = 5;
pub const SUB_THAT: u8 = 6;

#[allow(dead_code)]
pub const ASP_NONE: u8 = 0;
pub const ASP_INF: u8 = 1;
pub const ASP_PERF: u8 = 2;
pub const ASP_IMPF: u8 = 3;
pub const ASP_CONT: u8 = 4;

include!(concat!(env!("OUT_DIR"), "/frequent_tl.rs"));
