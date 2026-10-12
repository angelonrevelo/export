use std::io::Cursor;

use export_docs::{
    convert, decode_segment, list_segments, pack_deflated, pack_stored, pairs, pdf_merge, pdf_page_count,
    pdf_rotate, pdf_split, pdf_text, pdf_watermark, save_conversion,
};

fn document_xml() -> String {
    r#"<?xml version="1.0" encoding="UTF-8"?>
<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main">
  <w:body>
    <w:p>
      <w:pPr><w:pStyle w:val="Heading1"/></w:pPr>
      <w:r><w:t>Title &amp; Co</w:t></w:r>
    </w:p>
    <w:p>
      <w:r><w:rPr><w:b/></w:rPr><w:t>Bold</w:t></w:r>
      <w:r><w:t> tail</w:t></w:r>
    </w:p>
    <w:tbl>
      <w:tr>
        <w:tc><w:p><w:r><w:t>A</w:t></w:r></w:p></w:tc>
        <w:tc><w:p><w:r><w:t>B</w:t></w:r></w:p></w:tc>
      </w:tr>
    </w:tbl>
  </w:body>
</w:document>"#
        .to_string()
}

#[test]
fn docx_to_text_and_html_stored_and_deflated() {
    let xml = document_xml();
    for bytes in [
        pack_stored(&[("word/document.xml", xml.as_bytes())]),
        pack_deflated(&[("word/document.xml", xml.as_bytes())]),
    ] {
        let text = convert("docx", "txt", &bytes).unwrap();
        let text = String::from_utf8(text).unwrap();
        assert!(text.contains("Title & Co"), "{text}");
        assert!(text.contains("Bold tail"), "{text}");
        assert!(text.contains("A\tB") || text.contains("A") && text.contains("B"), "{text}");
        let html = String::from_utf8(convert("docx", "html", &bytes).unwrap()).unwrap();
        assert!(html.contains("<h1>"));
        assert!(html.contains("<strong>Bold</strong>"), "{html}");
        assert!(html.contains("Title &amp; Co"), "{html}");
        assert!(html.contains("<table>"), "{html}");
    }
}

#[test]
fn large_stored_docx_stays_inside_the_window() {
    let mut xml = String::from(
        r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    for i in 0..8000 {
        xml.push_str(&format!("<w:p><w:r><w:t>para {i} alpha</w:t></w:r></w:p>"));
    }
    xml.push_str("</w:body></w:document>");
    assert!(xml.len() > 200_000);
    let docx = pack_stored(&[("word/document.xml", xml.as_bytes())]);
    let mut out = Vec::new();
    let stats = export_docs::convert_to(&mut Cursor::new(&docx), "docx", "txt", &mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("para 0 alpha"));
    assert!(text.contains("para 7999 alpha"));
    assert!(
        stats.scratch_peak <= 128 * 1024,
        "scratch {} exceeded the window",
        stats.scratch_peak
    );
}

#[test]
fn markdown_and_html_pdf() {
    let html = convert("md", "html", b"# Notes\n\nA **word** here.\n").unwrap();
    let html_s = String::from_utf8(html.clone()).unwrap();
    assert!(html_s.contains("<h1>Notes</h1>"));
    assert!(html_s.contains("<strong>word</strong>"));
    let pdf = convert("html", "pdf", &html).unwrap();
    assert!(pdf.starts_with(b"%PDF-"));
    assert!(pdf.windows(9).any(|w| w == b"startxref"));
    let text = pdf_text(&pdf).unwrap();
    assert!(text.contains("Notes"), "{text}");
    assert!(text.contains("word"), "{text}");
    assert!(pdf_page_count(&pdf).unwrap() >= 1);
}

#[test]
fn pdf_split_merge_rotate_watermark() {
    let mut md = String::from("# PAGEONEWORD\n\n");
    for i in 0..70 {
        md.push_str(&format!("filler line {i} stays on a page.\n\n"));
    }
    md.push_str("# PAGETWOWORD\n");
    let pdf = convert("md", "pdf", md.as_bytes()).unwrap();
    assert!(pdf_page_count(&pdf).unwrap() >= 2, "pages {}", pdf_page_count(&pdf).unwrap());
    let parts = pdf_split(&pdf).unwrap();
    assert!(parts.len() >= 2);
    for part in &parts {
        assert_eq!(pdf_page_count(part).unwrap(), 1);
        assert!(part.windows(7).any(|w| w == b"trailer"));
    }
    let first = pdf_text(&parts[0]).unwrap();
    let last = pdf_text(parts.last().unwrap()).unwrap();
    assert!(first.contains("PAGEONEWORD"), "{first}");
    assert!(last.contains("PAGETWOWORD"), "{last}");
    let merged = pdf_merge(&[&parts[0], parts.last().unwrap()]).unwrap();
    assert_eq!(pdf_page_count(&merged).unwrap(), 2);
    let both = pdf_text(&merged).unwrap();
    assert!(both.contains("PAGEONEWORD") && both.contains("PAGETWOWORD"), "{both}");
    let turned = pdf_rotate(&parts[0], 90).unwrap();
    assert!(turned.windows(10).any(|w| w == b"/Rotate 90"));
    let marked = pdf_watermark(&parts[0], "SECRET").unwrap();
    assert!(pdf_text(&marked).unwrap().contains("SECRET"));
}

#[test]
fn segment_roundtrip_and_misses_are_listed() {
    let dir = std::env::temp_dir().join(format!(
        "export-docs-conv-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let path = save_conversion(&dir, 0, "md", "html", b"# Hi\n").unwrap();
    let bytes = std::fs::read(&path).unwrap();
    let seg = decode_segment(&bytes).unwrap();
    let html = String::from_utf8(seg.body).unwrap();
    assert!(html.contains("<h1>Hi</h1>"));
    assert_eq!(list_segments(&dir).unwrap().len(), 1);
    std::fs::write(dir.join("0001.exd.tmp"), b"partial").unwrap();
    assert_eq!(list_segments(&dir).unwrap().len(), 1);
    let _ = std::fs::remove_dir_all(&dir);

    let misses: Vec<_> = pairs().iter().filter(|p| !p.ready).collect();
    assert!(misses.iter().any(|p| p.target == "shrink" && p.note.contains("MISS")));
    assert!(misses.iter().any(|p| p.target == "encrypt"));
    assert!(convert("pdf", "shrink", b"%PDF-").is_err());
    assert!(convert("rtf", "html", b"{\\rtf}").is_err());
}

#[test]
fn odt_text() {
    let xml = r#"<?xml version="1.0"?>
<office:document-content xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
  <office:body><office:text>
    <text:h text:outline-level="1">Chapter</text:h>
    <text:p>Body text</text:p>
  </office:text></office:body>
</office:document-content>"#;
    let odt = pack_stored(&[("content.xml", xml.as_bytes())]);
    let text = String::from_utf8(convert("odt", "txt", &odt).unwrap()).unwrap();
    assert!(text.contains("Chapter"), "{text}");
    assert!(text.contains("Body text"), "{text}");
    let html = String::from_utf8(convert("odt", "html", &odt).unwrap()).unwrap();
    assert!(html.contains("<h1>"), "{html}");
}
