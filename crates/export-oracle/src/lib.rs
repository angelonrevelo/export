//! Host converter for `scripts/outside_score.py`.
//!
//! `export-rust-oracle <src> <dest> <kind>` writes one file. The scorer names
//! that binary with `EXPORT_RUST_ORACLE`. This crate is not a hot library: it
//! calls `export-image`, `export-docs`, and `export-sheet`, and it does not
//! link FFmpeg, LibreOffice, or yt-dlp.
//!
//! A photo or video already at or under the cap is copied. That is the match
//! the app uses. A phone JPEG over the cap is a baseline 4:2:0 shrink through
//! `export-image`. HEIC and MP4 have no decoder or byte-cap shrink in the
//! crates that already exist, so those over-cap inputs fail.

#![forbid(unsafe_code)]

use std::path::Path;

use export_image::{encode_jpeg, resize, Filter, Raster, SHRINK_QUALITY_MAX};

/// Photo shrink cap in `scripts/outside_score.py` (`phone-jpeg` and `heic`).
pub const PHOTO_CAP: usize = 1_000_000;

/// Video shrink cap in `scripts/outside_score.py` (`mp4`).
pub const VIDEO_CAP: usize = 5_000_000;

/// Convert `src` for one scored kind. The bytes are what the CLI writes.
pub fn convert(kind: &str, src: &[u8]) -> Result<Vec<u8>, String> {
    match kind {
        "phone-jpeg" => {
            if src.len() <= PHOTO_CAP {
                Ok(src.to_vec())
            } else {
                shrink_baseline_jpeg(src, PHOTO_CAP)
            }
        }
        "heic" => copy_under_cap(src, PHOTO_CAP, "heic: no decoder in the existing crates"),
        "mp4" => copy_under_cap(src, VIDEO_CAP, "mp4: no shrink in the existing crates"),
        "docx" => export_docs::convert("docx", "txt", src).map_err(|err| err.to_string()),
        "pdf" => export_docs::convert("pdf", "txt", src).map_err(|err| err.to_string()),
        "xlsx" => xlsx_to_csv(src),
        other => Err(format!("unknown kind {other}")),
    }
}

/// Write `bytes` to `dest` by renaming a sibling temp file into place.
pub fn write_output(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
    }
    let mut tmp = dest.to_path_buf();
    let name = dest.file_name().and_then(|s| s.to_str()).unwrap_or("out");
    tmp.set_file_name(format!(".{name}.oracle-tmp"));
    std::fs::write(&tmp, bytes).map_err(|err| err.to_string())?;
    if dest.exists() {
        std::fs::remove_file(dest).map_err(|err| err.to_string())?;
    }
    std::fs::rename(&tmp, dest).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        err.to_string()
    })
}

fn copy_under_cap(src: &[u8], cap: usize, over: &str) -> Result<Vec<u8>, String> {
    if src.len() <= cap {
        Ok(src.to_vec())
    } else {
        Err(over.to_string())
    }
}

/// Baseline 4:2:0 JPEG shrink. Quality search, then up to six truncating
/// 0.7 scales, then COM padding to `cap`. Same control flow as the app's
/// byte target, using the encoder `export-image` already has.
pub fn shrink_baseline_jpeg(src: &[u8], cap: usize) -> Result<Vec<u8>, String> {
    if cap == 0 {
        return Err("phone-jpeg: cap is zero".to_string());
    }
    if src.len() <= cap {
        return Ok(src.to_vec());
    }
    let mut image = export_image::decode_jpeg(src).map_err(|err| format!("phone-jpeg: {err}"))?;
    for _ in 0..6 {
        if let Some(encoded) = best_under(&image, cap)? {
            let mut encoded = encoded;
            pad_jpeg(&mut encoded, cap);
            return Ok(encoded);
        }
        let width = truncated_scale(image.width);
        let height = truncated_scale(image.height);
        if width == image.width && height == image.height {
            break;
        }
        image = resize(&image, width, height, Filter::Area)
            .map_err(|err| format!("phone-jpeg: {err}"))?;
    }
    Err("phone-jpeg: does not fit the byte cap".to_string())
}

fn best_under(image: &Raster, cap: usize) -> Result<Option<Vec<u8>>, String> {
    let mut lo: u16 = 1;
    let mut hi: u16 = u16::from(SHRINK_QUALITY_MAX);
    let mut best: Option<Vec<u8>> = None;
    while lo <= hi {
        let mid = lo + (hi - lo) / 2;
        let buf = encode_jpeg(image, mid as u8).map_err(|err| format!("phone-jpeg: {err}"))?;
        if buf.len() <= cap {
            best = Some(buf);
            if mid == u16::from(SHRINK_QUALITY_MAX) {
                break;
            }
            lo = mid + 1;
        } else if mid == 1 {
            break;
        } else {
            hi = mid - 1;
        }
    }
    Ok(best)
}

/// `int(side * 0.7)`, and at least 2. That is the app's scale step.
fn truncated_scale(side: u32) -> u32 {
    let scaled = (f64::from(side) * 0.7) as u32;
    scaled.max(2)
}

/// Grow a baseline JPEG to `cap` with COM markers before EOI.
fn pad_jpeg(buf: &mut Vec<u8>, cap: usize) {
    if buf.len() >= cap {
        return;
    }
    let need = cap - buf.len();
    if buf.len() >= 2 && buf[buf.len() - 2] == 0xff && buf[buf.len() - 1] == 0xd9 {
        buf.truncate(buf.len() - 2);
    }
    let mut remaining = need;
    while remaining >= 4 {
        let added = remaining.min(2 + 65535);
        let length = (added - 2) as u16;
        let payload = (length - 2) as usize;
        buf.push(0xff);
        buf.push(0xfe);
        buf.extend_from_slice(&length.to_be_bytes());
        buf.extend(std::iter::repeat(0u8).take(payload));
        remaining -= added;
    }
    buf.extend_from_slice(&[0xff, 0xd9]);
    buf.resize(buf.len() + remaining, 0);
}

fn xlsx_to_csv(src: &[u8]) -> Result<Vec<u8>, String> {
    let (csv, _) = export_sheet::convert_bytes(
        export_sheet::Format::Xlsx,
        export_sheet::Format::Csv,
        src,
        export_sheet::Options::default(),
    )
    .map_err(|err| err.to_string())?;
    Ok(excel_newlines(&csv))
}

/// The app's csv writer ends rows with CR LF. The sheet crate ends them with LF.
fn excel_newlines(csv: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(csv.len() + 8);
    let mut quoted = false;
    let mut i = 0;
    while i < csv.len() {
        let byte = csv[i];
        if byte == b'"' {
            out.push(byte);
            if quoted && i + 1 < csv.len() && csv[i + 1] == b'"' {
                out.push(b'"');
                i += 2;
                continue;
            }
            quoted = !quoted;
            i += 1;
            continue;
        }
        if byte == b'\n' && !quoted {
            out.extend_from_slice(b"\r\n");
            i += 1;
            continue;
        }
        out.push(byte);
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_output_replaces_one_file() {
        let dir = std::env::temp_dir().join(format!("export-oracle-write-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let dest = dir.join("rust-out");
        write_output(&dest, b"one").unwrap();
        write_output(&dest, b"two").unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"two");
        assert!(!dir.join(".rust-out.oracle-tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn under_cap_bytes_are_copied() {
        let src = b"not-a-photo";
        assert_eq!(convert("phone-jpeg", src).unwrap(), src);
        assert_eq!(convert("heic", src).unwrap(), src);
        assert_eq!(convert("mp4", src).unwrap(), src);
    }

    #[test]
    fn over_cap_heic_and_mp4_fail() {
        let heic = vec![0u8; PHOTO_CAP + 1];
        assert!(convert("heic", &heic).unwrap_err().contains("no decoder"));
        let mp4 = vec![0u8; VIDEO_CAP + 1];
        assert!(convert("mp4", &mp4).unwrap_err().contains("no shrink"));
    }

    #[test]
    fn unknown_kind_fails() {
        assert!(convert("wav", b"x").is_err());
    }

    #[test]
    fn docx_and_pdf_use_export_docs() {
        let xml = br#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body><w:p><w:r><w:t>Oracle line</w:t></w:r></w:p></w:body>
</w:document>"#;
        let docx = export_docs::pack_stored(&[("word/document.xml", xml.as_slice())]);
        let text = convert("docx", &docx).unwrap();
        assert_eq!(text, export_docs::convert("docx", "txt", &docx).unwrap());
        assert!(text.windows(11).any(|w| w == b"Oracle line"));

        let pdf = export_docs::convert("txt", "pdf", b"Oracle line\n").unwrap();
        assert_eq!(
            convert("pdf", &pdf).unwrap(),
            export_docs::convert("pdf", "txt", &pdf).unwrap()
        );
    }

    #[test]
    fn xlsx_uses_export_sheet_with_excel_newlines() {
        let (xlsx, _) = export_sheet::convert_bytes(
            export_sheet::Format::Csv,
            export_sheet::Format::Xlsx,
            b"a,b\n1,2\n",
            export_sheet::Options { header: true },
        )
        .unwrap();
        let out = convert("xlsx", &xlsx).unwrap();
        let (csv, _) = export_sheet::convert_bytes(
            export_sheet::Format::Xlsx,
            export_sheet::Format::Csv,
            &xlsx,
            export_sheet::Options::default(),
        )
        .unwrap();
        assert_eq!(out, excel_newlines(&csv));
        assert!(out.windows(2).any(|pair| pair == b"\r\n"));
    }

    #[test]
    fn non_baseline_jpeg_over_a_small_cap_fails() {
        let bytes = [0xff, 0xd8, 0xff, 0xd9, 0, 0, 0, 0];
        assert!(shrink_baseline_jpeg(&bytes, 4).is_err());
    }

    #[test]
    fn baseline_jpeg_over_the_cap_lands_on_the_cap() {
        let mut pixels = vec![0u8; 64 * 64 * 3];
        for (i, px) in pixels.iter_mut().enumerate() {
            *px = (i % 251) as u8;
        }
        let raster = Raster::new(64, 64, 3, pixels).unwrap();
        let src = encode_jpeg(&raster, 95).unwrap();
        let tight = encode_jpeg(&raster, 1).unwrap();
        if tight.len() >= src.len() {
            assert_eq!(shrink_baseline_jpeg(&src, src.len()).unwrap(), src);
            return;
        }
        let out = shrink_baseline_jpeg(&src, tight.len()).unwrap();
        assert_eq!(out.len(), tight.len());
        assert_eq!(&out[..2], &[0xff, 0xd8]);
    }
}
