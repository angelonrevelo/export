//! epub to HTML when the spine is one, two, three, or four XHTML files.
//! Those files are copied in spine order. Any other package stays a miss.
//! Pandoc is not called.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::docx::CountWrite;
use crate::error::{Error, Result};
use crate::stats::RunStats;
use crate::xml::{self, Attr, XmlSink};
use crate::zip::{self, ArchiveIndex};

const COPY: usize = 8 * 1024;

pub fn epub_to_html<R: Read + Seek, W: Write>(src: &mut R, out: &mut W) -> Result<RunStats> {
    let input_bytes = match src.seek(SeekFrom::End(0)) {
        Ok(n) => n,
        Err(err) => return Err(miss(0, &err.to_string())),
    };
    if let Err(err) = src.seek(SeekFrom::Start(0)) {
        return Err(miss(input_bytes, &err.to_string()));
    }
    read_package(src, out, input_bytes).map_err(|err| {
        if err.message.starts_with("MISS:") {
            err
        } else {
            miss(input_bytes, &err.message)
        }
    })
}

fn miss(input_bytes: u64, why: &str) -> Error {
    Error::new(format!("MISS: epub {why} (input_b={input_bytes})"))
}

fn read_package<R: Read + Seek, W: Write>(
    src: &mut R,
    out: &mut W,
    input_bytes: u64,
) -> Result<RunStats> {
    let index = ArchiveIndex::read(src)?;
    let mut scratch = 0usize;
    check_mimetype(src, &index, &mut scratch)?;
    let opf_path = container_opf(src, &index, &mut scratch)?;
    let hrefs = spine_hrefs(src, &index, &opf_path, &mut scratch)?;
    let mut counted = CountWrite::new(out);
    for href in &hrefs {
        let zip_path = join_href(&opf_path, href)?;
        if index.by_name(&zip_path).is_none() {
            return Err(spine_shape());
        }
        let mut reader = zip::open_entry(src, &index, &zip_path)?;
        scratch = scratch.max(reader.scratch_peak().saturating_add(COPY));
        let mut buf = [0u8; COPY];
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|err| Error::new(err.to_string()))?;
            if n == 0 {
                break;
            }
            counted
                .write_all(&buf[..n])
                .map_err(|err| Error::new(err.to_string()))?;
        }
    }
    Ok(RunStats {
        scratch_peak: scratch,
        input_bytes,
        output_bytes: counted.n,
    })
}

fn check_mimetype<R: Read + Seek>(
    src: &mut R,
    index: &ArchiveIndex,
    scratch: &mut usize,
) -> Result<()> {
    if index.by_name("mimetype").is_none() {
        return Err(Error::new("package is not one xhtml spine"));
    }
    let mut reader = zip::open_entry(src, index, "mimetype")?;
    *scratch = (*scratch).max(reader.scratch_peak());
    let bytes = read_capped(&mut reader, 64)?;
    let text = std::str::from_utf8(&bytes).unwrap_or("");
    if text.trim() != "application/epub+zip" {
        return Err(Error::new("package is not one xhtml spine"));
    }
    Ok(())
}

fn container_opf<R: Read + Seek>(
    src: &mut R,
    index: &ArchiveIndex,
    scratch: &mut usize,
) -> Result<String> {
    if index.by_name("META-INF/container.xml").is_none() {
        return Err(Error::new("package is not one xhtml spine"));
    }
    let mut reader = zip::open_entry(src, index, "META-INF/container.xml")?;
    *scratch = (*scratch).max(reader.scratch_peak());
    let mut sink = ContainerSink { paths: Vec::new() };
    xml::pull(&mut reader, &mut sink)?;
    if sink.paths.len() != 1 {
        return Err(Error::new("package is not one xhtml spine"));
    }
    let (path, media) = &sink.paths[0];
    if media != "application/oebps-package+xml" || path.is_empty() {
        return Err(Error::new("package is not one xhtml spine"));
    }
    Ok(path.clone())
}

fn spine_shape() -> Error {
    Error::new("spine is not one, two, three, or four xhtml files")
}

fn spine_hrefs<R: Read + Seek>(
    src: &mut R,
    index: &ArchiveIndex,
    opf_path: &str,
    scratch: &mut usize,
) -> Result<Vec<String>> {
    if index.by_name(opf_path).is_none() {
        return Err(Error::new("package is not one xhtml spine"));
    }
    let mut reader = zip::open_entry(src, index, opf_path)?;
    *scratch = (*scratch).max(reader.scratch_peak());
    let mut sink = OpfSink {
        manifest: false,
        spine: false,
        items: Vec::new(),
        refs: Vec::new(),
    };
    xml::pull(&mut reader, &mut sink)?;
    if !(1..=4).contains(&sink.refs.len()) {
        return Err(spine_shape());
    }
    let mut hrefs = Vec::new();
    for idref in &sink.refs {
        let matches: Vec<_> = sink.items.iter().filter(|item| item.id == *idref).collect();
        if matches.len() != 1 {
            return Err(spine_shape());
        }
        let item = matches[0];
        if item.media != "application/xhtml+xml" {
            return Err(spine_shape());
        }
        hrefs.push(item.href.clone());
    }
    Ok(hrefs)
}

fn join_href(opf_path: &str, href: &str) -> Result<String> {
    if href.is_empty()
        || href.starts_with('/')
        || href.contains('\\')
        || href.contains('#')
        || href.contains('?')
        || href.contains('%')
    {
        return Err(spine_shape());
    }
    let mut parts = Vec::new();
    if let Some((dir, _)) = opf_path.rsplit_once('/') {
        for part in dir.split('/') {
            if part.is_empty() || part == "." || part == ".." {
                return Err(spine_shape());
            }
            parts.push(part);
        }
    }
    for part in href.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(spine_shape());
        }
        parts.push(part);
    }
    Ok(parts.join("/"))
}

fn read_capped<R: Read>(reader: &mut R, max: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut buf = [0u8; 64];
    loop {
        let n = reader
            .read(&mut buf)
            .map_err(|err| Error::new(err.to_string()))?;
        if n == 0 {
            break;
        }
        if out.len() + n > max {
            return Err(Error::new("package is not one xhtml spine"));
        }
        out.extend_from_slice(&buf[..n]);
    }
    Ok(out)
}

fn local_name(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

fn attr<'a>(attrs: &'a [Attr], key: &str) -> Option<&'a str> {
    attrs
        .iter()
        .find(|attr| attr.name == key || local_name(&attr.name) == key)
        .map(|attr| attr.value.as_str())
}

struct ContainerSink {
    paths: Vec<(String, String)>,
}

impl XmlSink for ContainerSink {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        if local_name(name) == "rootfile" {
            let path = attr(attrs, "full-path").unwrap_or("").to_string();
            let media = attr(attrs, "media-type").unwrap_or("").to_string();
            self.paths.push((path, media));
        }
        Ok(())
    }

    fn end(&mut self, _name: &str) -> Result<()> {
        Ok(())
    }

    fn text(&mut self, _text: &str) -> Result<()> {
        Ok(())
    }
}

struct Item {
    id: String,
    href: String,
    media: String,
}

struct OpfSink {
    manifest: bool,
    spine: bool,
    items: Vec<Item>,
    refs: Vec<String>,
}

impl XmlSink for OpfSink {
    fn start(&mut self, name: &str, attrs: &[Attr]) -> Result<()> {
        match local_name(name) {
            "manifest" => self.manifest = true,
            "spine" => self.spine = true,
            "item" if self.manifest => self.items.push(Item {
                id: attr(attrs, "id").unwrap_or("").to_string(),
                href: attr(attrs, "href").unwrap_or("").to_string(),
                media: attr(attrs, "media-type").unwrap_or("").to_string(),
            }),
            "itemref" if self.spine => {
                self.refs
                    .push(attr(attrs, "idref").unwrap_or("").to_string());
            }
            _ => {}
        }
        Ok(())
    }

    fn end(&mut self, name: &str) -> Result<()> {
        match local_name(name) {
            "manifest" => self.manifest = false,
            "spine" => self.spine = false,
            _ => {}
        }
        Ok(())
    }

    fn text(&mut self, _text: &str) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::{pack_deflated, pack_stored};
    use std::io::Cursor;

    const XHTML: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Notes</title></head>
<body><h1>Hello &amp; Co</h1><p>Spine body</p></body></html>
"#;

    fn container() -> &'static str {
        r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#
    }

    fn opf(spine: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="style" href="style.css" media-type="text/css"/>
  </manifest>
  <spine>
    {itemrefs}
  </spine>
</package>
"#,
            itemrefs = spine
        )
    }

    fn one_spine() -> String {
        opf(r#"<itemref idref="chap"/>"#)
    }

    fn package(files: &[(&str, &[u8])], deflate: bool) -> Vec<u8> {
        if deflate {
            pack_deflated(files)
        } else {
            pack_stored(files)
        }
    }

    fn html_of(bytes: &[u8]) -> Result<Vec<u8>> {
        let mut out = Vec::new();
        epub_to_html(&mut Cursor::new(bytes), &mut out)?;
        Ok(out)
    }

    #[test]
    fn one_xhtml_spine_is_copied() {
        let opf = one_spine();
        let css = b"p { color: black; }";
        for deflate in [false, true] {
            let bytes = package(
                &[
                    ("mimetype", b"application/epub+zip"),
                    ("META-INF/container.xml", container().as_bytes()),
                    ("OEBPS/content.opf", opf.as_bytes()),
                    ("OEBPS/chapter.xhtml", XHTML.as_bytes()),
                    ("OEBPS/style.css", css),
                ],
                deflate,
            );
            let got = html_of(&bytes).unwrap();
            assert_eq!(got, XHTML.as_bytes());
        }
    }

    fn two_opf(spine: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    {spine}
  </spine>
</package>
"#
        )
    }

    #[test]
    fn two_xhtml_files_are_copied_in_spine_order() {
        let first = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>One</p></body></html>";
        let second = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>";
        let forward = two_opf("<itemref idref=\"chap\"/>\n    <itemref idref=\"next\"/>");
        let bytes = pack_stored(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container().as_bytes()),
            ("OEBPS/content.opf", forward.as_bytes()),
            ("OEBPS/chapter.xhtml", first),
            ("OEBPS/next.xhtml", second),
        ]);
        let mut expect = first.to_vec();
        expect.extend_from_slice(second);
        assert_eq!(html_of(&bytes).unwrap(), expect);
        let reverse = two_opf("<itemref idref=\"next\"/>\n    <itemref idref=\"chap\"/>");
        let bytes = pack_stored(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container().as_bytes()),
            ("OEBPS/content.opf", reverse.as_bytes()),
            ("OEBPS/chapter.xhtml", first),
            ("OEBPS/next.xhtml", second),
        ]);
        let mut expect = second.to_vec();
        expect.extend_from_slice(first);
        assert_eq!(html_of(&bytes).unwrap(), expect);
    }

    #[test]
    fn three_xhtml_files_are_copied_in_spine_order() {
        let first = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>One</p></body></html>";
        let second = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>";
        let third = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Three</p></body></html>";
        let forward = two_opf(
            "<itemref idref=\"chap\"/>\n    <itemref idref=\"next\"/>\n    <itemref idref=\"more\"/>",
        );
        let bytes = pack_stored(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container().as_bytes()),
            ("OEBPS/content.opf", forward.as_bytes()),
            ("OEBPS/chapter.xhtml", first),
            ("OEBPS/next.xhtml", second),
            ("OEBPS/more.xhtml", third),
        ]);
        let mut expect = first.to_vec();
        expect.extend_from_slice(second);
        expect.extend_from_slice(third);
        assert_eq!(html_of(&bytes).unwrap(), expect);
        let reverse = two_opf(
            "<itemref idref=\"more\"/>\n    <itemref idref=\"chap\"/>\n    <itemref idref=\"next\"/>",
        );
        let bytes = pack_stored(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container().as_bytes()),
            ("OEBPS/content.opf", reverse.as_bytes()),
            ("OEBPS/chapter.xhtml", first),
            ("OEBPS/next.xhtml", second),
            ("OEBPS/more.xhtml", third),
        ]);
        let mut expect = third.to_vec();
        expect.extend_from_slice(first);
        expect.extend_from_slice(second);
        assert_eq!(html_of(&bytes).unwrap(), expect);
    }

    fn four_opf(spine: &str) -> String {
        format!(
            r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
    <item id="tail" href="tail.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    {spine}
  </spine>
</package>
"#
        )
    }

    #[test]
    fn four_xhtml_files_are_copied_in_spine_order() {
        let first = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>One</p></body></html>";
        let second = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>";
        let third = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Three</p></body></html>";
        let fourth = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Four</p></body></html>";
        let forward = four_opf(
            "<itemref idref=\"chap\"/>\n    <itemref idref=\"next\"/>\n    <itemref idref=\"more\"/>\n    <itemref idref=\"tail\"/>",
        );
        let bytes = pack_stored(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container().as_bytes()),
            ("OEBPS/content.opf", forward.as_bytes()),
            ("OEBPS/chapter.xhtml", first),
            ("OEBPS/next.xhtml", second),
            ("OEBPS/more.xhtml", third),
            ("OEBPS/tail.xhtml", fourth),
        ]);
        let mut expect = first.to_vec();
        expect.extend_from_slice(second);
        expect.extend_from_slice(third);
        expect.extend_from_slice(fourth);
        assert_eq!(html_of(&bytes).unwrap(), expect);
        let reverse = four_opf(
            "<itemref idref=\"tail\"/>\n    <itemref idref=\"chap\"/>\n    <itemref idref=\"next\"/>\n    <itemref idref=\"more\"/>",
        );
        let bytes = pack_stored(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container().as_bytes()),
            ("OEBPS/content.opf", reverse.as_bytes()),
            ("OEBPS/chapter.xhtml", first),
            ("OEBPS/next.xhtml", second),
            ("OEBPS/more.xhtml", third),
            ("OEBPS/tail.xhtml", fourth),
        ]);
        let mut expect = fourth.to_vec();
        expect.extend_from_slice(first);
        expect.extend_from_slice(second);
        expect.extend_from_slice(third);
        assert_eq!(html_of(&bytes).unwrap(), expect);
    }

    #[test]
    fn five_spine_items_stay_a_miss() {
        let opf = four_opf(
            "<itemref idref=\"chap\"/>\n    <itemref idref=\"next\"/>\n    <itemref idref=\"more\"/>\n    <itemref idref=\"tail\"/>\n    <itemref idref=\"end\"/>",
        );
        let bytes = pack_stored(&[
            ("mimetype", b"application/epub+zip"),
            ("META-INF/container.xml", container().as_bytes()),
            ("OEBPS/content.opf", opf.as_bytes()),
            ("OEBPS/chapter.xhtml", XHTML.as_bytes()),
            (
                "OEBPS/next.xhtml",
                b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>",
            ),
            (
                "OEBPS/more.xhtml",
                b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Three</p></body></html>",
            ),
            (
                "OEBPS/tail.xhtml",
                b"<html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Four</p></body></html>",
            ),
        ]);
        let err = html_of(&bytes).unwrap_err();
        assert!(err.message.starts_with("MISS:"), "{err}");
        assert!(
            err.message.contains("not one, two, three, or four"),
            "{err}"
        );
        assert!(
            err.message.contains(&format!("input_b={}", bytes.len())),
            "{err}"
        );
    }
}
