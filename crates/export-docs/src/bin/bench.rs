//! Release timings for export-docs.
//!
//! Latency is published only from a release build. A debug build prints
//! `latency NOT JUDGED` and still lists every miss.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_docs::{
    convert, pack_deflated, pack_stored, pairs, pdf_flate_fixture, pdf_merge, pdf_rotate, pdf_shrink,
    pdf_split, pdf_watermark, word97_document,
};

const WARMUP: usize = 4;
const SAMPLES: usize = 64;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

struct CountAlloc;

// SAFETY: every method forwards the pointer and layout to the system allocator.
// Counters move only after a non-null pointer comes back.
unsafe impl GlobalAlloc for CountAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: `layout` is a valid layout from the allocator, and `System` is the default allocator.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: `ptr` came from this allocator with `layout`.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: same contract as `alloc`.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: `ptr` came from this allocator with `layout`; `new_size` is the requested size.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            ALLOCS.fetch_add(1, Ordering::Relaxed);
            BYTES.fetch_add(new_size, Ordering::Relaxed);
        }
        new_ptr
    }
}

#[global_allocator]
static GLOBAL: CountAlloc = CountAlloc;

struct Sample {
    p50_ns: u128,
    p99_ns: u128,
    cold_ns: u128,
    allocs_p50: usize,
    alloc_bytes_p50: usize,
    output_bytes: u64,
}

fn tally() -> (usize, usize) {
    (ALLOCS.load(Ordering::Relaxed), BYTES.load(Ordering::Relaxed))
}

fn percentile_u128(sorted: &[u128], pct: f64) -> u128 {
    let n = sorted.len();
    let rank = (pct / 100.0 * n as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(n - 1)]
}

fn percentile_usize(sorted: &[usize], pct: f64) -> usize {
    let n = sorted.len();
    let rank = (pct / 100.0 * n as f64).ceil() as usize;
    sorted[rank.saturating_sub(1).min(n - 1)]
}

fn measure(mut op: impl FnMut() -> u64) -> Sample {
    let t0 = Instant::now();
    let output_bytes = op();
    let cold_ns = t0.elapsed().as_nanos();
    for _ in 0..WARMUP {
        let _ = black_box(op());
    }
    let mut times = vec![0u128; SAMPLES];
    let mut allocs = vec![0usize; SAMPLES];
    let mut abytes = vec![0usize; SAMPLES];
    for i in 0..SAMPLES {
        let (before_a, before_b) = tally();
        let started = Instant::now();
        let _ = black_box(op());
        let dt = started.elapsed().as_nanos();
        let (after_a, after_b) = tally();
        times[i] = dt;
        allocs[i] = after_a.saturating_sub(before_a);
        abytes[i] = after_b.saturating_sub(before_b);
    }
    times.sort_unstable();
    allocs.sort_unstable();
    abytes.sort_unstable();
    Sample {
        p50_ns: percentile_u128(&times, 50.0),
        p99_ns: percentile_u128(&times, 99.0),
        cold_ns,
        allocs_p50: percentile_usize(&allocs, 50.0),
        alloc_bytes_p50: percentile_usize(&abytes, 50.0),
        output_bytes,
    }
}

fn mb_per_s(input_bytes: u64, p50_ns: u128) -> f64 {
    if p50_ns == 0 {
        return f64::INFINITY;
    }
    (input_bytes as f64) * 1_000.0 / (p50_ns as f64)
}

fn emit_ready(label: &str, input_bytes: u64, sample: &Sample) {
    if cfg!(debug_assertions) {
        println!(
            "{label:<32} {input_bytes:>10} {out:>10} {nj:>12} {nj:>12} {nj:>12} {nj:>10} {nj:>12} {nj:>12} ready",
            out = sample.output_bytes,
            nj = "NOT_JUDGED",
        );
        return;
    }
    let mb_s = mb_per_s(input_bytes, sample.p50_ns);
    println!(
        "{label:<32} {input_bytes:>10} {out:>10} {p50:>12} {p99:>12} {mb_s:>12.2} {allocs:>10} {abytes:>12} {cold:>12} ready",
        out = sample.output_bytes,
        p50 = sample.p50_ns / 1_000,
        p99 = sample.p99_ns / 1_000,
        allocs = sample.allocs_p50,
        abytes = sample.alloc_bytes_p50,
        cold = sample.cold_ns / 1_000,
    );
}

fn emit_untimed(label: &str, input_bytes: u64, output_bytes: Option<u64>, status: &str) {
    let output = match output_bytes {
        Some(n) => n.to_string(),
        None => "-".to_string(),
    };
    println!(
        "{label:<32} {input_bytes:>10} {output:>10} {d:>12} {d:>12} {d:>12} {d:>10} {d:>12} {d:>12} {status}",
        d = "-",
    );
}

fn report_shrink(pdf: &[u8]) {
    match pdf_shrink(pdf) {
        Ok(_) => {
            eprintln!("uncompressed pdf shrink produced a file");
            std::process::exit(1);
        }
        Err(e) => {
            if !e.message.starts_with("MISS:") || !e.message.contains(&format!("input_b={}", pdf.len())) {
                eprintln!("shrink miss hid the input size: {e}");
                std::process::exit(1);
            }
            emit_untimed("pdf→shrink", pdf.len() as u64, None, &e.message);
        }
    }
    let flate = pdf_flate_fixture(pdf).unwrap_or_else(|e| {
        eprintln!("flate fixture failed: {e}");
        std::process::exit(1);
    });
    match pdf_shrink(&flate) {
        Ok(out) => {
            if out.len() >= flate.len() {
                eprintln!("shrink returned a file that is not smaller");
                std::process::exit(1);
            }
            emit_untimed(
                "pdf→shrink flate",
                flate.len() as u64,
                Some(out.len() as u64),
                "recompressed FlateDecode round-trip, file smaller",
            );
        }
        Err(e) => {
            emit_untimed("pdf→shrink flate", flate.len() as u64, None, &e.message);
        }
    }
}

fn emit_miss(label: &str, note: &str) {
    println!(
        "{label:<32} {d:>10} {d:>10} {d:>12} {d:>12} {d:>12} {d:>10} {d:>12} {d:>12} {note}",
        d = "-",
    );
}

fn report_pptx() {
    let deck = pptx_text_deck();
    let sample = measure(|| ready_bytes("pptx", "html", &deck));
    emit_ready("pptx→html text", deck.len() as u64, &sample);
    if !cfg!(debug_assertions) {
        println!(
            "pptx→html text p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
            sample.p50_ns / 1_000,
            sample.p99_ns / 1_000,
            mb_per_s(deck.len() as u64, sample.p50_ns),
            sample.allocs_p50,
            sample.alloc_bytes_p50,
            deck.len(),
            sample.output_bytes,
        );
    }
    let pic = pptx_pic_deck();
    match convert("pptx", "html", &pic) {
        Ok(_) => {
            eprintln!("picture deck produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", pic.len()))
            {
                eprintln!("pptx miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("pptx→html pic", pic.len() as u64, None, &err.message);
        }
    }
}

fn slide_xml(title: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<p:sld xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree>
    <p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr>
    <p:grpSpPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="0" cy="0"/><a:chOff x="0" y="0"/><a:chExt cx="0" cy="0"/></a:xfrm></p:grpSpPr>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="2" name="Title"/><p:cNvSpPr/><p:nvPr><p:ph type="title"/></p:nvPr></p:nvSpPr>
      <p:spPr><a:prstGeom prst="rect"><a:avLst/></a:prstGeom></p:spPr>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang="en-US"/><a:t>{title}</a:t></a:r></a:p></p:txBody>
    </p:sp>
    <p:sp>
      <p:nvSpPr><p:cNvPr id="3" name="Body"/><p:cNvSpPr/><p:nvPr><p:ph type="body"/></p:nvPr></p:nvSpPr>
      <p:spPr/>
      <p:txBody><a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr b="1"/><a:t>{body}</a:t></a:r></a:p></p:txBody>
    </p:sp>
  </p:spTree></p:cSld>
</p:sld>"#
    )
}

fn pptx_text_deck() -> Vec<u8> {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    for n in 1..=40 {
        let xml = slide_xml(&format!("Slide {n} title"), "alpha beta gamma delta text shape");
        files.push((format!("ppt/slides/slide{n}.xml"), xml.into_bytes()));
    }
    let refs: Vec<(&str, &[u8])> = files
        .iter()
        .map(|(name, bytes)| (name.as_str(), bytes.as_slice()))
        .collect();
    pack_stored(&refs)
}

fn pptx_pic_deck() -> Vec<u8> {
    let xml = br#"<?xml version="1.0"?>
<p:sld xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main">
  <p:cSld><p:spTree><p:pic/></p:spTree></p:cSld>
</p:sld>"#;
    pack_stored(&[("ppt/slides/slide1.xml", &xml[..])])
}

fn report_epub() {
    let book = epub_one_spine();
    let sample = measure(|| ready_bytes("epub", "html", &book));
    emit_ready("epub→html spine", book.len() as u64, &sample);
    if !cfg!(debug_assertions) {
        println!(
            "epub→html spine p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
            sample.p50_ns / 1_000,
            sample.p99_ns / 1_000,
            mb_per_s(book.len() as u64, sample.p50_ns),
            sample.allocs_p50,
            sample.alloc_bytes_p50,
            book.len(),
            sample.output_bytes,
        );
    }
    let two = epub_two_spine();
    match convert("epub", "html", &two) {
        Ok(_) => {
            eprintln!("two-item spine produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", two.len()))
            {
                eprintln!("epub miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html two", two.len() as u64, None, &err.message);
        }
    }
}

fn epub_chapter() -> String {
    let mut body = String::from(
        r#"<?xml version="1.0" encoding="utf-8"?>
<html xmlns="http://www.w3.org/1999/xhtml"><head><title>Notes</title></head><body>
<h1>Notes</h1>
"#,
    );
    for i in 0..200 {
        body.push_str(&format!("<p>Paragraph {i} of the generated epub spine.</p>\n"));
    }
    body.push_str("</body></html>\n");
    body
}

fn epub_container() -> &'static str {
    r#"<?xml version="1.0"?>
<container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container">
  <rootfiles>
    <rootfile full-path="OEBPS/content.opf" media-type="application/oebps-package+xml"/>
  </rootfiles>
</container>
"#
}

fn epub_opf(itemrefs: &str) -> String {
    format!(
        r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Notes</dc:title></metadata>
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    {itemrefs}
  </spine>
</package>
"#
    )
}

fn epub_one_spine() -> Vec<u8> {
    let chapter = epub_chapter();
    let opf = epub_opf(r#"<itemref idref="chap"/>"#);
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", chapter.as_bytes()),
    ])
}

fn epub_two_spine() -> Vec<u8> {
    let chapter = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>One</p></body></html>";
    let next = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>";
    let opf = epub_opf(
        "<itemref idref=\"chap\"/>\n    <itemref idref=\"next\"/>",
    );
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", chapter),
        ("OEBPS/next.xhtml", next),
    ])
}

fn report_rtf() {
    let doc = rtf_paragraphs();
    let sample = measure(|| ready_bytes("rtf", "html", &doc));
    emit_ready("rtf→html text", doc.len() as u64, &sample);
    if !cfg!(debug_assertions) {
        println!(
            "rtf→html text p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
            sample.p50_ns / 1_000,
            sample.p99_ns / 1_000,
            mb_per_s(doc.len() as u64, sample.p50_ns),
            sample.allocs_p50,
            sample.alloc_bytes_p50,
            doc.len(),
            sample.output_bytes,
        );
    }
    let pict = rtf_pict();
    match convert("rtf", "html", &pict) {
        Ok(_) => {
            eprintln!("pict rtf produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", pict.len()))
            {
                eprintln!("rtf miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("rtf→html pict", pict.len() as u64, None, &err.message);
        }
    }
}

fn rtf_paragraphs() -> Vec<u8> {
    let mut s = String::from("{\\rtf1\\ansi\\deff0{\\fonttbl{\\f0 Arial;}}");
    for i in 0..200 {
        if i % 2 == 0 {
            s.push_str(&format!("Paragraph {i} of the generated rtf file.\\par\n"));
        } else {
            s.push_str(&format!("{{\\b Paragraph {i} bold.}}\\par\n"));
        }
    }
    s.push('}');
    s.into_bytes()
}

fn rtf_pict() -> Vec<u8> {
    b"{\\rtf1\\ansi{\\pict hello}\\par}".to_vec()
}

fn report_doc() {
    let text = doc_paragraph();
    let doc = word97_document(&[&text]).unwrap_or_else(|err| {
        eprintln!("word97 fixture failed: {err}");
        std::process::exit(1);
    });
    let sample = measure(|| ready_bytes("doc", "html", &doc));
    emit_ready("doc→html para", doc.len() as u64, &sample);
    if !cfg!(debug_assertions) {
        println!(
            "doc→html para p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
            sample.p50_ns / 1_000,
            sample.p99_ns / 1_000,
            mb_per_s(doc.len() as u64, sample.p50_ns),
            sample.allocs_p50,
            sample.alloc_bytes_p50,
            doc.len(),
            sample.output_bytes,
        );
    }
    report_doc_two();
}

fn doc_paragraph() -> String {
    let mut s = String::new();
    for i in 0..400 {
        s.push_str(&format!("Sentence {i} of the generated Word 97 paragraph. "));
    }
    s
}

fn report_doc_two() {
    let doc = doc_two_paragraphs();
    let sample = measure(|| ready_bytes("doc", "html", &doc));
    emit_ready("doc→html two", doc.len() as u64, &sample);
    if !cfg!(debug_assertions) {
        println!(
            "doc→html two p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
            sample.p50_ns / 1_000,
            sample.p99_ns / 1_000,
            mb_per_s(doc.len() as u64, sample.p50_ns),
            sample.allocs_p50,
            sample.alloc_bytes_p50,
            doc.len(),
            sample.output_bytes,
        );
    }
    let enc = doc_encrypted();
    match convert("doc", "html", &enc) {
        Ok(_) => {
            eprintln!("encrypted doc produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", enc.len()))
            {
                eprintln!("doc miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("doc→html enc", enc.len() as u64, None, &err.message);
        }
    }
}

fn doc_two_paragraphs() -> Vec<u8> {
    let mut first = String::new();
    let mut second = String::new();
    for i in 0..200 {
        first.push_str(&format!("Alpha {i} of the first generated paragraph. "));
        second.push_str(&format!("Beta {i} of the second generated paragraph. "));
    }
    word97_document(&[&first, &second]).unwrap_or_else(|err| {
        eprintln!("word97 two-paragraph fixture failed: {err}");
        std::process::exit(1);
    })
}

fn doc_encrypted() -> Vec<u8> {
    let mut doc = word97_document(&["Hi"]).unwrap_or_else(|err| {
        eprintln!("word97 encrypted fixture failed: {err}");
        std::process::exit(1);
    });
    let pos = doc.windows(2).position(|w| w == [0xEC, 0xA5]).unwrap_or_else(|| {
        eprintln!("word97 fixture has no fib");
        std::process::exit(1);
    });
    doc[pos + 0x0B] = 0x01;
    doc
}

fn shrink_pdf_fixture() -> Vec<u8> {
    let mut md = String::from("# Notes\n\nA **word** and a list.\n\n- one\n- two\n\n");
    for i in 0..80 {
        md.push_str(&format!(
            "Paragraph {i} of the markdown fixture stays in the stream.\n\n"
        ));
    }
    convert("md", "pdf", md.as_bytes()).expect("md pdf fixture")
}

fn docx_xml(paras: usize) -> String {
    let mut xml = String::new();
    xml.push_str(
        r#"<?xml version="1.0"?><w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body>"#,
    );
    xml.push_str(
        r#"<w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>Title &amp; Co</w:t></w:r></w:p>"#,
    );
    for i in 0..paras {
        xml.push_str("<w:p><w:r><w:rPr><w:b/></w:rPr><w:t>");
        xml.push_str(&format!("para {i} alpha beta gamma"));
        xml.push_str("</w:t></w:r></w:p>");
    }
    xml.push_str("</w:body></w:document>");
    xml
}

fn ready_bytes(source: &str, target: &str, input: &[u8]) -> u64 {
    let out = convert(source, target, input).unwrap_or_else(|e| {
        eprintln!("ready pair {source}→{target} failed: {e}");
        std::process::exit(1);
    });
    let n = out.len() as u64;
    black_box(out);
    n
}

fn main() {
    if std::env::args().any(|arg| arg == "--shrink") {
        let pdf = shrink_pdf_fixture();
        println!("export-docs shrink byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_shrink(&pdf);
        return;
    }
    if std::env::args().any(|arg| arg == "--pptx") {
        println!("export-docs pptx byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_pptx();
        return;
    }
    if std::env::args().any(|arg| arg == "--epub") {
        println!("export-docs epub byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_epub();
        return;
    }
    if std::env::args().any(|arg| arg == "--rtf") {
        println!("export-docs rtf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_rtf();
        return;
    }
    if std::env::args().any(|arg| arg == "--doc") {
        println!("export-docs doc byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_doc();
        return;
    }
    if std::env::args().any(|arg| arg == "--doc-two") {
        println!("export-docs doc two-paragraph byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_doc_two();
        return;
    }
    let xml = docx_xml(2500);
    let docx_deflate = pack_deflated(&[("word/document.xml", xml.as_bytes())]);
    let docx_store = pack_stored(&[("word/document.xml", xml.as_bytes())]);
    let odt_xml = br#"<?xml version="1.0"?>
<office:document-content xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">
  <office:body><office:text>
    <text:h text:outline-level="1">Chapter</text:h>
    <text:p>Body text for the odt path.</text:p>
  </office:text></office:body>
</office:document-content>"#;
    let odt = pack_stored(&[("content.xml", odt_xml)]);
    let markdown = {
        let mut md = String::from("# Notes\n\nA **word** and a list.\n\n- one\n- two\n\n");
        for i in 0..80 {
            md.push_str(&format!("Paragraph {i} of the markdown fixture stays in the stream.\n\n"));
        }
        md
    };
    let html = convert("md", "html", markdown.as_bytes()).expect("md html fixture");
    let text = b"Plain paragraph one.\n\nPlain paragraph two.\n";
    let pdf = shrink_pdf_fixture();
    let page_a = convert("txt", "pdf", b"PAGEONEWORD\n").expect("page a");
    let page_b = convert("txt", "pdf", b"PAGETWOWORD\n").expect("page b");

    println!("export-docs bench");
    println!("samples {SAMPLES}");
    println!("warmup {WARMUP}");
    if cfg!(debug_assertions) {
        println!("latency NOT JUDGED");
    } else {
        println!("profile release");
    }
    println!(
        "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
        "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
    );

    let headline = measure(|| ready_bytes("docx", "html", &docx_deflate));
    emit_ready("docx→html deflate", docx_deflate.len() as u64, &headline);
    if !cfg!(debug_assertions) {
        println!(
            "headline docx→html deflate p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
            headline.p50_ns / 1_000,
            headline.p99_ns / 1_000,
            mb_per_s(docx_deflate.len() as u64, headline.p50_ns),
            headline.allocs_p50,
            headline.alloc_bytes_p50,
            docx_deflate.len(),
            headline.output_bytes,
        );
    }

    let rows: [(&str, &str, &str, &[u8]); 12] = [
        ("docx→txt deflate", "docx", "txt", &docx_deflate),
        ("docx→pdf deflate", "docx", "pdf", &docx_deflate),
        ("docx→html store", "docx", "html", &docx_store),
        ("odt→html store", "odt", "html", &odt),
        ("md→html", "md", "html", markdown.as_bytes()),
        ("md→txt", "md", "txt", markdown.as_bytes()),
        ("md→pdf", "md", "pdf", markdown.as_bytes()),
        ("html→pdf", "html", "pdf", &html),
        ("html→txt", "html", "txt", &html),
        ("txt→html", "txt", "html", text),
        ("txt→pdf", "txt", "pdf", text),
        ("pdf→txt", "pdf", "txt", &pdf),
    ];
    for (label, source, target, input) in rows {
        let sample = measure(|| ready_bytes(source, target, input));
        emit_ready(label, input.len() as u64, &sample);
    }

    let split = measure(|| {
        let parts = pdf_split(&pdf).unwrap_or_else(|e| {
            eprintln!("pdf split failed: {e}");
            std::process::exit(1);
        });
        let n = parts.iter().map(Vec::len).sum::<usize>() as u64;
        black_box(parts);
        n
    });
    emit_ready("pdf→split", pdf.len() as u64, &split);

    let merge = measure(|| {
        let out = pdf_merge(&[&page_a, &page_b]).unwrap_or_else(|e| {
            eprintln!("pdf merge failed: {e}");
            std::process::exit(1);
        });
        let n = out.len() as u64;
        black_box(out);
        n
    });
    emit_ready("pdf→merge", (page_a.len() + page_b.len()) as u64, &merge);

    let rotate = measure(|| {
        let out = pdf_rotate(&page_a, 90).unwrap_or_else(|e| {
            eprintln!("pdf rotate failed: {e}");
            std::process::exit(1);
        });
        let n = out.len() as u64;
        black_box(out);
        n
    });
    emit_ready("pdf→rotate", page_a.len() as u64, &rotate);

    let watermark = measure(|| {
        let out = pdf_watermark(&page_a, "SECRET").unwrap_or_else(|e| {
            eprintln!("pdf watermark failed: {e}");
            std::process::exit(1);
        });
        let n = out.len() as u64;
        black_box(out);
        n
    });
    emit_ready("pdf→watermark", page_a.len() as u64, &watermark);

    report_shrink(&pdf);

    let mut misses = 0usize;
    for pair in pairs().iter().filter(|p| !p.ready) {
        let label = format!("{}→{}", pair.source, pair.target);
        let err = match convert(pair.source, pair.target, b"%PDF-1.4 miss") {
            Err(e) => e,
            Ok(_) => {
                eprintln!("miss {label} produced output");
                std::process::exit(1);
            }
        };
        if !err.message.starts_with("MISS:") {
            eprintln!("miss {label} hid its failure: {err}");
            std::process::exit(1);
        }
        emit_miss(&label, &err.message);
        misses += 1;
    }
    println!("misses {misses}");
}
