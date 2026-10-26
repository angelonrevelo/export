//! The command the outside score calls: `export-oracle <src> <dest> <kind>`.
//!
//! One file is written at `dest`; a failure prints a reason on stderr and
//! exits non-zero. The kinds mirror `scripts/outside_score.py`:
//!
//! - `phone-jpeg`, `heic`: a source over the 1 MB cap is re-encoded under it
//!   and grown back to exactly the cap; a source at or under the cap is
//!   copied. Only JPEG sources re-encode today.
//! - `mp4`: a source at or under the 5 MB cap is copied; a larger one is an
//!   honest failure until the crates can re-encode.
//! - `docx`: the document's text.
//! - `pdf`: the literal text of each content stream, shaped per page.
//! - `xlsx`: the active sheet as CSV.

use std::process::ExitCode;

use export_image::ShrinkCache;
use export_sheet::{Format, Options};

const IMAGE_CAP: u64 = 1_000_000;
const VIDEO_CAP: u64 = 5_000_000;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 4 {
        eprintln!("usage: export-oracle <src> <dest> <kind>");
        return ExitCode::from(2);
    }
    match run(&args[1], &args[2], &args[3]) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn run(src: &str, dest: &str, kind: &str) -> Result<(), String> {
    let bytes = std::fs::read(src).map_err(|e| format!("cannot read {src}: {e}"))?;
    let out = match kind {
        "phone-jpeg" | "heic" => shrink_image(&bytes, kind),
        "mp4" => shrink_video(&bytes),
        "docx" => export_docs::convert("docx", "txt", &bytes)
            .map(|text| match std::str::from_utf8(&text) {
                Ok(s) => text_mode_bytes(s),
                Err(_) => text,
            })
            .map_err(|e| e.to_string()),
        "pdf" => pdf_text(&bytes),
        "xlsx" => xlsx_csv(&bytes),
        other => Err(format!("unknown kind {other}")),
    }?;
    std::fs::write(dest, &out).map_err(|e| format!("cannot write {dest}: {e}"))?;
    Ok(())
}

/// Shrink to the image cap: copy what fits, re-encode what does not.
fn shrink_image(bytes: &[u8], kind: &str) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 <= IMAGE_CAP {
        return Ok(bytes.to_vec());
    }
    match kind {
        "phone-jpeg" => {
            let mut cache = ShrinkCache::new();
            let outcome = export_image::shrink_jpeg(bytes, IMAGE_CAP, &mut cache)
                .map_err(|e| e.to_string())?;
            Ok(outcome.bytes)
        }
        _ => Err("heic decode is not implemented; only under-cap files convert".to_string()),
    }
}

/// Shrink to the video cap: copy what fits, refuse the rest for now.
fn shrink_video(bytes: &[u8]) -> Result<Vec<u8>, String> {
    if bytes.len() as u64 <= VIDEO_CAP {
        return Ok(bytes.to_vec());
    }
    Err("mp4 re-encode is not implemented; only under-cap files convert".to_string())
}

/// The app strips each page, joins the pages with a blank line, strips the
/// whole, and writes through a text-mode handle, so a `\n` becomes the
/// platform line separator.
fn pdf_text(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let pages = export_docs::pdf_text_pages(bytes).map_err(|e| e.to_string())?;
    let shaped = pages
        .iter()
        .map(|page| page.trim())
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(text_mode_bytes(shaped.trim()))
}

fn text_mode_bytes(text: &str) -> Vec<u8> {
    if cfg!(windows) {
        text.replace('\n', "\r\n").into_bytes()
    } else {
        text.as_bytes().to_vec()
    }
}

/// The active sheet as CSV. The Python csv writer ends every record with
/// `\r\n`; the streaming writer here ends records with `\n`. A field that
/// contains a newline is always quoted, so every unquoted `\n` in the output
/// is a record terminator.
fn xlsx_csv(bytes: &[u8]) -> Result<Vec<u8>, String> {
    let (csv, _) = export_sheet::convert_bytes(Format::Xlsx, Format::Csv, bytes, Options::default())
        .map_err(|e| e.to_string())?;
    Ok(csv_record_separators(&csv))
}

/// The Python csv writer ends every record with `\r\n`, and a record that is
/// one empty field prints as `""`; the streaming writer here ends records
/// with `\n` and prints that record as an empty line. A field that contains
/// a newline is always quoted, so every unquoted `\n` in the output is a
/// record terminator.
fn csv_record_separators(csv: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(csv.len() + 16);
    let mut in_quotes = false;
    let mut record_start = 0usize;
    let mut i = 0;
    while i < csv.len() {
        let b = csv[i];
        if in_quotes {
            out.push(b);
            if b == b'"' {
                if i + 1 < csv.len() && csv[i + 1] == b'"' {
                    out.push(b'"');
                    i += 1;
                } else {
                    in_quotes = false;
                }
            }
        } else if b == b'"' {
            in_quotes = true;
            out.push(b);
        } else if b == b'\n' {
            if out.len() == record_start {
                out.extend_from_slice(b"\"\"");
            }
            out.extend_from_slice(b"\r\n");
            record_start = out.len();
        } else {
            out.push(b);
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_terminators_become_crlf_outside_quotes() {
        // a,"b
        // c",d\n → a,"b
        // c",d\r\n
        let csv = b"a,\"b\nc\",d\n\"x\"\"y\",e\n";
        assert_eq!(csv_record_separators(csv), b"a,\"b\nc\",d\r\n\"x\"\"y\",e\r\n");
    }

    #[test]
    fn an_empty_single_field_record_prints_as_two_quotes() {
        // The Python csv writer writes [""] as "" and ["a", ""] as "a,".
        assert_eq!(csv_record_separators(b"\n"), b"\"\"\r\n");
        assert_eq!(csv_record_separators(b"a,\n"), b"a,\r\n");
        assert_eq!(csv_record_separators(b",\n"), b",\r\n");
    }

    #[test]
    fn the_pdf_shape_strips_pages_and_joins_with_a_blank_line() {
        let pages = ["  first  ".to_string(), "".to_string(), "third".to_string()];
        let shaped = pages
            .iter()
            .map(|page| page.trim())
            .collect::<Vec<_>>()
            .join("\n\n");
        assert_eq!(shaped.trim(), "first\n\n\n\nthird");
    }

    #[test]
    fn text_mode_matches_the_platform_handle() {
        let bytes = text_mode_bytes("a\nb");
        if cfg!(windows) {
            assert_eq!(bytes, b"a\r\nb");
        } else {
            assert_eq!(bytes, b"a\nb");
        }
    }
}
