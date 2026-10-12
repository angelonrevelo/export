//! Document conversions for export: DOCX, ODT, markdown, HTML, and PDF.
//!
//! The hot path is this crate. It has no dependencies. It does not shell out to
//! LibreOffice, pandoc, or Ghostscript. Zip inflate, the XML pull parser, and
//! PDF content streams live here.
//!
//! A conversion written to disk is one little-endian `EXDOC001` segment:
//! magic, a fixed span table, body, CRC-32. The write is a same-directory
//! rename of `NNNN.exd.tmp`. A `.tmp` is not a segment.
//!
//! Honest limits are the `ready: false` rows from [`pairs`].

#![forbid(unsafe_code)]

mod container;
mod convert;
mod crc;
mod deflate;
mod doc;
mod docx;
mod epub;
mod error;
mod htmlpdf;
mod markdown;
mod pdf;
mod ppt;
mod pptx;
mod rtf;
mod stats;
mod xml;
mod zip;

pub use container::{
    decode as decode_segment, encode as encode_segment, is_segment_name, list_segments, write_segment,
    Meta, Segment, HEAD, MAGIC, SPANS, TABLE_BYTE,
};
pub use convert::{
    convert, convert_to, pair_ready, pairs, pdf_flate_fixture, pdf_image_matches, pdf_merge,
    pdf_page_count, pdf_rotate, pdf_shrink, pdf_split, pdf_text, pdf_watermark, save_conversion, Pair,
};
pub use error::{Error, Result};
pub use stats::RunStats;
pub use doc::word97_document;
pub use ppt::{
    ppt_eight_text_document, ppt_five_text_document, ppt_four_text_document,
    ppt_nine_text_document,
    ppt_seven_text_document,
    ppt_six_text_document, ppt_text_document,
    ppt_text_indexed_document, ppt_text_nibble_document,
    ppt_text_picture_document, ppt_three_text_document, ppt_two_text_document,
    ppt_two_text_indexed_document, ppt_text_two_indexed_document, ppt_text_three_indexed_document,
    ppt_text_four_indexed_document, ppt_text_five_indexed_document, ppt_text_six_indexed_document,
};
pub use pptx::{pptx_raster_slide, pptx_raster_stored_pdf};
pub use zip::{pack_deflated, pack_stored};
