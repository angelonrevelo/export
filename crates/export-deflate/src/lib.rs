//! Raw DEFLATE (RFC 1951) compress and inflate.
//!
//! This is not a zlib wrapper: there is no CMF/FLG header and no Adler-32.
//! A host may check the bytes with Python's zlib at `wbits=-15`. This crate
//! does not link zlib.

#![forbid(unsafe_code)]

mod bits;
mod compress;
mod huff;
mod inflate;
mod lz;

pub use compress::{compress, compress_with, Method};
pub use inflate::{inflate, inflate_with_limit, Error, DEFAULT_MAX_OUT};

/// Benchmark fixture: the sentence and a newline, repeated and cut to `len`.
pub fn repeated_text(len: usize) -> Vec<u8> {
    const UNIT: &[u8] = b"The quick brown fox jumps over the lazy dog.\n";
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let n = (len - out.len()).min(UNIT.len());
        out.extend_from_slice(&UNIT[..n]);
    }
    out
}
