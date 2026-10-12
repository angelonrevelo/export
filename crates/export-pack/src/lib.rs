//! Store-zip archives, CBZ to a minimal PDF, and TTF/OTF container copy.
//!
//! The hot crate has no dependencies. An archive is a structured file:
//! local headers, payload bytes, a CRC-32 of those bytes, then a central
//! directory. It is published by writing `path.tmp` and renaming onto `path`.
//! A general-purpose compressor wrapped around that file is not this design.

#![forbid(unsafe_code)]

mod crc;
mod deflate;
mod error;
mod font;
mod pdf;
mod png;
mod zip;

pub use crc::Crc32;
pub use deflate::{inflate_raw, inflate_zlib, zlib_store};
pub use error::{Error, Result};
pub use font::{
    convert_font, convert_font_file, font_container, font_tables, tiny_ttf, FontContainer,
};
pub use pdf::{cbz_to_pdf, PdfInfo};
pub use png::{decode_png, encode_png_rgb, MAX_PIXELS};
pub use zip::{
    Entry, Method, ZipReader, ZipWriter, CHUNK_BYTES, MAX_ARCHIVE_LEN, MAX_ENTRIES, MAX_ENTRY_LEN,
};
