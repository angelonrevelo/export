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
//!
//! # XLS
//!
//! One generated BIFF8 workbook. It is a compound file with a `Workbook`
//! stream, one sheet, cell A1 = 42, and cell B1 = `cat`. The reader accepts
//! that layout: OLE version 3, BIFF version `0x0600`, a `NUMBER` record, and
//! a plain shared string. A second generated file keeps that text cell and
//! adds a `NUMBER` in C1 (7). A third file is 1 000 rows of one numeric
//! column, values 0 through 999, with no formulas. A formula cell is returned
//! only as its cached IEEE number. The expression is not evaluated. A SUM of
//! two numeric cells with no cached number is those two cells added. A SUM of
//! three numeric cells with no cached number is those three cells added. A SUM of
//! four numeric cells with no cached number is those four cells added. A SUM of
//! one contiguous three-cell range with no cached number is those three cells
//! added. A SUM of one contiguous four-cell range with no cached number is
//! those four cells added. A SUM of the column A1:A3 with no cached number is
//! those three cells added. A SUM of the column A1:A4 with no cached number is
//! those four cells added. Any other uncached expression is refused. A file that is not BIFF
//! is refused. `CONTINUE`,
//! rich shared strings, and encryption are refused rather than guessed.

#![forbid(unsafe_code)]

mod convert;
mod delim;
mod error;
mod json;
mod xls;

pub use convert::{convert, convert_bytes, Format, Options, Stats};
pub use delim::DelimReader;
pub use error::{Error, Result};
pub use json::JsonReader;
pub use xls::generated_biff_xls;
pub use xls::generated_biff_xls_numeric_column;
pub use xls::generated_biff_xls_sum_cache;
pub use xls::generated_biff_xls_sum_uncached;
pub use xls::generated_biff_xls_sum_uncached_area;
pub use xls::generated_biff_xls_sum_uncached_area_column;
pub use xls::generated_biff_xls_sum_uncached_area_column_four;
pub use xls::generated_biff_xls_sum_uncached_area_four;
pub use xls::generated_biff_xls_sum_uncached_four;
pub use xls::generated_biff_xls_sum_uncached_three;
pub use xls::generated_biff_xls_two_numbers;

/// One record larger than this is refused.
///
/// A quoted CSV field may contain newlines, so the reader has to accumulate
/// bytes until the quote closes. The cap is what stops a missing quote from
/// turning the rest of the file into one allocation.
pub const MAX_RECORD_BYTES: usize = 16 * 1024 * 1024;
