use export_pack::{
    cbz_to_pdf, convert_font, convert_font_file, decode_png, encode_png_rgb, font_container,
    font_tables, tiny_ttf, FontContainer, ZipWriter,
};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static SCRATCH: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new() -> Self {
        let n = SCRATCH.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!("export-pack-cf-{}-{n}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn sibling_tmp(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".tmp");
    PathBuf::from(os)
}

#[test]
fn png_roundtrip_pixels() {
    let rgb = vec![10, 20, 30, 40, 50, 60, 70, 80, 90, 1, 2, 3];
    let png = encode_png_rgb(2, 2, &rgb).unwrap();
    let image = decode_png(&png).unwrap();
    assert_eq!((image.width, image.height), (2, 2));
    assert_eq!(image.rgb, rgb);
}

#[test]
fn cbz_to_multipage_pdf() {
    let dir = Scratch::new();
    let a = encode_png_rgb(2, 2, &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12]).unwrap();
    let b = encode_png_rgb(3, 1, &[9, 8, 7, 6, 5, 4, 3, 2, 1]).unwrap();
    let jpeg = [
        0xFFu8, 0xD8, 0xFF, 0xC0, 0x00, 0x0B, 0x08, 0x00, 0x01, 0x00, 0x01, 0x01, 0x01, 0x11, 0x00,
        0xFF, 0xDA, 0x00, 0x08, 0x01, 0x01, 0x00, 0x00, 0x3F, 0x00, 0x7F, 0xFF, 0xD9,
    ];
    let cbz = dir.path("comic.cbz");
    let mut zip = ZipWriter::create(&cbz).unwrap();
    zip.write_bytes("ComicInfo.xml", b"<ComicInfo/>").unwrap();
    zip.write_bytes("00/a.png", &a).unwrap();
    zip.write_bytes("01/b.png", &b).unwrap();
    zip.write_bytes("02/c.jpg", &jpeg).unwrap();
    zip.finish().unwrap();

    let pdf_path = dir.path("comic.pdf");
    let info = cbz_to_pdf(&cbz, &pdf_path).unwrap();
    assert_eq!(info.pages, 3);
    assert!(!sibling_tmp(&pdf_path).exists());
    let pdf = fs::read(&pdf_path).unwrap();
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.starts_with("%PDF-1.4\n"));
    assert!(text.contains("%%EOF\n"));
    assert_eq!(text.matches("/Type /Page ").count(), 3);
    assert!(text.contains("/MediaBox [0 0 2 2]"));
    assert!(text.contains("/MediaBox [0 0 3 1]"));
    assert!(text.contains("/MediaBox [0 0 1 1]"));
    assert!(text.contains("/Filter /DCTDecode"));
    let page_a = text.find("/MediaBox [0 0 2 2]").unwrap();
    let page_b = text.find("/MediaBox [0 0 3 1]").unwrap();
    let page_c = text.find("/MediaBox [0 0 1 1]").unwrap();
    assert!(page_a < page_b && page_b < page_c);
}

#[test]
fn cbz_without_images_errors_and_does_not_publish() {
    let dir = Scratch::new();
    let cbz = dir.path("notes.cbz");
    let mut zip = ZipWriter::create(&cbz).unwrap();
    zip.write_bytes("readme.txt", b"nope").unwrap();
    zip.finish().unwrap();
    let pdf = dir.path("out.pdf");
    assert!(cbz_to_pdf(&cbz, &pdf).is_err());
    assert!(!pdf.exists());
    assert!(!sibling_tmp(&pdf).exists());
}

#[test]
fn ttf_otf_container_copies_tables() {
    let ttf = tiny_ttf();
    assert_eq!(font_container(&ttf).unwrap(), FontContainer::Ttf);
    let otf = convert_font(&ttf, FontContainer::Otf).unwrap();
    assert_eq!(font_container(&otf).unwrap(), FontContainer::Otf);
    assert_ne!(&ttf[..4], &otf[..4]);
    assert_eq!(&otf[..4], b"OTTO");

    let ttf_tables = font_tables(&ttf).unwrap();
    let otf_tables = font_tables(&otf).unwrap();
    assert_eq!(ttf_tables.len(), otf_tables.len());
    for ((tag_a, body_a), (tag_b, body_b)) in ttf_tables.iter().zip(otf_tables.iter()) {
        assert_eq!(tag_a, tag_b);
        if tag_a == b"head" {
            assert_eq!(&body_a[..8], &body_b[..8]);
            assert_ne!(&body_a[8..12], &body_b[8..12]);
            assert_eq!(&body_a[12..], &body_b[12..]);
        } else {
            assert_eq!(body_a, body_b, "{}", String::from_utf8_lossy(tag_a));
        }
    }
    let name = font_tables(&otf)
        .unwrap()
        .into_iter()
        .find(|(tag, _)| tag == b"name")
        .unwrap()
        .1;
    assert!(name
        .windows(20)
        .any(|window| window == utf16("ExportPack").as_slice()));

    let back = convert_font(&otf, FontContainer::Ttf).unwrap();
    assert_eq!(back, ttf);

    let mut flipped = ttf.clone();
    let name_off = ttf
        .windows(20)
        .position(|w| w == utf16("ExportPack").as_slice())
        .unwrap();
    flipped[name_off] ^= 0xFF;
    assert!(convert_font(&flipped, FontContainer::Otf).is_err());
}

#[test]
fn font_file_is_published_by_rename() {
    let dir = Scratch::new();
    let path = dir.path("tiny.otf");
    convert_font_file(&tiny_ttf(), FontContainer::Otf, &path).unwrap();
    assert!(path.exists());
    assert!(!sibling_tmp(&path).exists());
    assert_eq!(
        font_container(&fs::read(&path).unwrap()).unwrap(),
        FontContainer::Otf
    );
}

fn utf16(text: &str) -> Vec<u8> {
    text.encode_utf16()
        .flat_map(|unit| unit.to_be_bytes())
        .collect()
}
