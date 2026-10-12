//! FLAC subset for Export.
//!
//! The product path is pure Rust. ffmpeg is not linked and is not spawned.
//! Each input is hashed once (SHA-256, streaming), checked for a duplicate,
//! sniffed by magic bytes, and only then decoded or encoded.
//!
//! The library is single-threaded. It does not spawn work and it does not
//! read a parallelism setting.

#![forbid(unsafe_code)]

mod crc;
mod dedup;
mod error;
mod export;
mod flac;
mod md5;
mod sha256;
mod sniff;
mod wav;

pub use dedup::{digest_of, Dedup, Prior};
pub use error::{Error, Result};
pub use export::{export, export_reader, Job, Output, DEFAULT_MAX_BYTES};
pub use flac::{decode, encode, Pcm, MAX_SAMPLES};
pub use sha256::{hash, hash_reader, hex, Sha256};
pub use sniff::{sniff, Kind};
