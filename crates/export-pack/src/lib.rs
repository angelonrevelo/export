//! Streaming zip archives.
//!
//! The hot crate has no dependencies. An archive is a structured file:
//! local headers, payload bytes, a CRC-32 of those bytes, then a central
//! directory. It is published by writing `path.tmp` and renaming onto `path`.
//! A general-purpose compressor wrapped around that file is not this design.

#![forbid(unsafe_code)]

mod crc;
mod error;
mod zip;

pub use crc::Crc32;
pub use error::{Error, Result};
pub use zip::{
    Entry, Method, ZipReader, ZipWriter, CHUNK_BYTES, MAX_ARCHIVE_LEN, MAX_ENTRIES, MAX_ENTRY_LEN,
};
