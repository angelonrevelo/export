//! Streaming spreadsheet conversions for the tabular subset Export already
//! accepts: CSV, TSV, and one JSON array.
//!
//! Nothing in this crate is imported. The parsers are hand-written because a
//! dependency would put a supply chain on the hot path, and because the only
//! values we need are strings in a rectangle. A record is parsed, written,
//! and dropped. Memory stays at one record plus a fixed read window, capped
//! by [`MAX_RECORD_BYTES`].
//!
//! Invalid UTF-8 is repaired to U+FFFD and counted on [`Stats::lossy`]. One
//! bad byte does not abort the rest of the file. That count is per record
//! that needed a repair, so a caller can report it instead of guessing.
//!
//! # JSON subset
//!
//! One array. Each element is either an object of scalars (string, number,
//! bool, null) or an array of scalars. The first element chooses which.
//! Object keys come from the first object, in that order. A later object may
//! omit a key (the cell is empty). A later object that introduces a key is an
//! error: dropping it would silently lose a column. Nested objects and arrays
//! are an error. Numbers, bools, and null are kept as their literal text,
//! except null, which is an empty cell. There is no number type on the CSV
//! side: every CSV cell is a string.

#![forbid(unsafe_code)]

mod convert;
mod delim;
mod error;
mod json;

pub use convert::{convert, convert_bytes, Format, Options, Stats};
pub use delim::DelimReader;
pub use error::{Error, Result};
pub use json::JsonReader;

/// One record larger than this is refused.
///
/// A quoted CSV field may contain newlines, so the reader has to accumulate
/// bytes until the quote closes. The cap is what stops a missing quote from
/// turning the rest of the file into one allocation.
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
