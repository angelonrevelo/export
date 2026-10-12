//! Release timings for export-docs.
//!
//! Latency is published only from a release build. A debug build prints
//! `latency NOT JUDGED` and still lists every miss.

use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

use export_docs::{
    convert, pack_deflated, pack_stored, pairs, pdf_flate_fixture, pdf_image_matches, pdf_merge,
    pdf_rotate, pdf_shrink, pdf_split, pdf_text, pdf_watermark, ppt_five_text_document,
    ppt_seven_text_document, ppt_six_text_document,
    ppt_four_text_document,
    ppt_text_document, ppt_text_indexed_document, ppt_text_nibble_document,
    ppt_text_picture_document, ppt_three_text_document, ppt_two_text_document,
    ppt_two_text_indexed_document, ppt_text_two_indexed_document, ppt_text_three_indexed_document,
    ppt_text_four_indexed_document, ppt_text_five_indexed_document, ppt_text_six_indexed_document,
    pptx_raster_slide,
    pptx_raster_stored_pdf, word97_document,
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

fn report_pptx_pdf() {
    let deck = pptx_text_deck();
    match convert("pptx", "pdf", &deck) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("pptx→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let has_slides = extracted.contains("Slide 1 title")
                && extracted.contains("Slide 40 title")
                && extracted.contains("alpha beta gamma delta text shape");
            if !has_slides {
                emit_untimed(
                    "pptx→pdf text",
                    deck.len() as u64,
                    Some(pdf.len() as u64),
                    "MISS: pdf text does not match",
                );
            } else {
                let sample = measure(|| ready_bytes("pptx", "pdf", &deck));
                emit_ready("pptx→pdf text", deck.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "pptx→pdf text p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(deck.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        deck.len(),
                        sample.output_bytes,
                    );
                }
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", deck.len()))
            {
                eprintln!("pptx→pdf hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("pptx→pdf text", deck.len() as u64, None, &err.message);
        }
    }
    let pic = pptx_pic_deck();
    match convert("pptx", "pdf", &pic) {
        Ok(_) => {
            eprintln!("picture deck produced pdf");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", pic.len()))
            {
                eprintln!("pptx pdf miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("pptx→pdf pic", pic.len() as u64, None, &err.message);
        }
    }
}

fn report_pptx_raster() {
    let pic = pptx_pic_deck();
    match convert("pptx", "pdf", &pic) {
        Ok(_) => {
            eprintln!("empty picture deck produced pdf");
            std::process::exit(1);
        }
        Err(err) => {
            if pic.len() != 296
                || !err.message.starts_with("MISS:")
                || !err.message.contains("not a text shape")
                || !err.message.contains(&format!("input_b={}", pic.len()))
            {
                eprintln!("empty picture miss changed: {err}");
                std::process::exit(1);
            }
            emit_untimed("pptx→pdf pic", pic.len() as u64, None, &err.message);
        }
    }
    let raster = pptx_raster_slide().unwrap_or_else(|err| {
        eprintln!("raster slide failed: {err}");
        std::process::exit(1);
    });
    let stored = pptx_raster_stored_pdf().unwrap_or_else(|err| {
        eprintln!("stored pixel pdf failed: {err}");
        std::process::exit(1);
    });
    match convert("pptx", "pdf", &raster) {
        Ok(pdf) => {
            let image = pdf.windows(15).any(|w| w == b"/Subtype /Image");
            let flate = pdf.windows(12).any(|w| w == b"/FlateDecode");
            if !image || !flate || pdf.len() >= stored.len() {
                eprintln!(
                    "raster pdf is not a smaller image XObject pdf_b={} stored_b={}",
                    pdf.len(),
                    stored.len()
                );
                std::process::exit(1);
            }
            let sample = measure(|| ready_bytes("pptx", "pdf", &raster));
            emit_ready("pptx→pdf raster", raster.len() as u64, &sample);
            if !cfg!(debug_assertions) {
                println!(
                    "pptx→pdf raster p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={} stored_b={}",
                    sample.p50_ns / 1_000,
                    sample.p99_ns / 1_000,
                    mb_per_s(raster.len() as u64, sample.p50_ns),
                    sample.allocs_p50,
                    sample.alloc_bytes_p50,
                    raster.len(),
                    sample.output_bytes,
                    stored.len(),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains("pdf_b=")
                || !err.message.contains("stored_b=")
                || !err.message.contains(&format!("input_b={}", raster.len()))
            {
                eprintln!("raster miss hid both sizes: {err}");
                std::process::exit(1);
            }
            emit_untimed("pptx→pdf raster", raster.len() as u64, None, &err.message);
            println!("stored_b={}", stored.len());
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
    let six = epub_six_spine();
    match convert("epub", "html", &six) {
        Ok(_) => {
            eprintln!("six-item spine produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", six.len()))
            {
                eprintln!("epub miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html six", six.len() as u64, None, &err.message);
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

fn report_epub_two() {
    let book = epub_two_book();
    let sample = measure(|| ready_bytes("epub", "html", &book));
    emit_ready("epub→html two", book.len() as u64, &sample);
    if !cfg!(debug_assertions) {
        println!(
            "epub→html two p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
            sample.p50_ns / 1_000,
            sample.p99_ns / 1_000,
            mb_per_s(book.len() as u64, sample.p50_ns),
            sample.allocs_p50,
            sample.alloc_bytes_p50,
            book.len(),
            sample.output_bytes,
        );
    }
    let six = epub_six_spine();
    match convert("epub", "html", &six) {
        Ok(_) => {
            eprintln!("six-item spine produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", six.len()))
            {
                eprintln!("epub miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html six", six.len() as u64, None, &err.message);
        }
    }
}

fn report_epub_three() {
    let two = epub_two_book();
    match convert("epub", "html", &two) {
        Ok(html) if two.len() == 14838 && html.len() == 13548 => {}
        Ok(html) => {
            eprintln!(
                "two-item epub size changed input_b={} output_b={}",
                two.len(),
                html.len()
            );
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("two-item epub failed: {err}");
            std::process::exit(1);
        }
    }
    let small = epub_three_spine();
    match convert("epub", "html", &small) {
        Ok(html) => {
            let text = String::from_utf8_lossy(&html);
            let ordered = match (text.find("<p>One</p>"), text.find("<p>Two</p>"), text.find("<p>Three</p>")) {
                (Some(one), Some(two), Some(three)) => one < two && two < three,
                _ => false,
            };
            if small.len() != 1691 || !ordered {
                eprintln!(
                    "small three-item spine changed input_b={} output_b={}",
                    small.len(),
                    html.len()
                );
                std::process::exit(1);
            }
            emit_untimed(
                "epub→html three small",
                small.len() as u64,
                Some(html.len() as u64),
                "copied in spine order, not timed",
            );
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", small.len()))
            {
                eprintln!("small three-item spine hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html three small", small.len() as u64, None, &err.message);
            return;
        }
    }
    let book = epub_three_book();
    match convert("epub", "html", &book) {
        Ok(html) => {
            let text = String::from_utf8_lossy(&html);
            let ordered = match (
                text.find("First paragraph 0"),
                text.find("Second paragraph 0"),
                text.find("Third paragraph 0"),
            ) {
                (Some(first), Some(second), Some(third)) => first < second && second < third,
                _ => false,
            };
            if !ordered {
                emit_untimed(
                    "epub→html three",
                    book.len() as u64,
                    Some(html.len() as u64),
                    "MISS: spine order does not match",
                );
                return;
            }
            let sample = measure(|| ready_bytes("epub", "html", &book));
            emit_ready("epub→html three", book.len() as u64, &sample);
            if !cfg!(debug_assertions) {
                println!(
                    "epub→html three p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                    sample.p50_ns / 1_000,
                    sample.p99_ns / 1_000,
                    mb_per_s(book.len() as u64, sample.p50_ns),
                    sample.allocs_p50,
                    sample.alloc_bytes_p50,
                    book.len(),
                    sample.output_bytes,
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", book.len()))
            {
                eprintln!("epub three hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html three", book.len() as u64, None, &err.message);
        }
    }
    let six = epub_six_spine();
    match convert("epub", "html", &six) {
        Ok(_) => {
            eprintln!("six-item spine produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", six.len()))
            {
                eprintln!("epub six miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html six", six.len() as u64, None, &err.message);
        }
    }
}

fn report_epub_four() {
    let three = epub_three_book();
    match convert("epub", "html", &three) {
        Ok(html) if three.len() == 21763 && html.len() == 20262 => {
            let text = String::from_utf8_lossy(&html);
            let ordered = match (
                text.find("First paragraph 0"),
                text.find("Second paragraph 0"),
                text.find("Third paragraph 0"),
            ) {
                (Some(first), Some(second), Some(third)) => first < second && second < third,
                _ => false,
            };
            if !ordered {
                eprintln!("three-item epub order changed");
                std::process::exit(1);
            }
        }
        Ok(html) => {
            eprintln!(
                "three-item epub size changed input_b={} output_b={}",
                three.len(),
                html.len()
            );
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("three-item epub failed: {err}");
            std::process::exit(1);
        }
    }
    let small = epub_four_spine();
    match convert("epub", "html", &small) {
        Ok(html) => {
            let text = String::from_utf8_lossy(&html);
            let ordered = match (
                text.find("<p>One</p>"),
                text.find("<p>Two</p>"),
                text.find("<p>Three</p>"),
                text.find("<p>Four</p>"),
            ) {
                (Some(one), Some(two), Some(three), Some(four)) => one < two && two < three && three < four,
                _ => false,
            };
            if small.len() != 1997 || !ordered {
                eprintln!(
                    "small four-item spine changed input_b={} output_b={}",
                    small.len(),
                    html.len()
                );
                std::process::exit(1);
            }
            emit_untimed(
                "epub→html four small",
                small.len() as u64,
                Some(html.len() as u64),
                "copied in spine order, not timed",
            );
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", small.len()))
            {
                eprintln!("small four-item spine hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html four small", small.len() as u64, None, &err.message);
            return;
        }
    }
    let book = epub_four_book();
    match convert("epub", "html", &book) {
        Ok(html) => {
            let text = String::from_utf8_lossy(&html);
            let ordered = match (
                text.find("First paragraph 0"),
                text.find("Second paragraph 0"),
                text.find("Third paragraph 0"),
                text.find("Fourth paragraph 0"),
            ) {
                (Some(first), Some(second), Some(third), Some(fourth)) => {
                    first < second && second < third && third < fourth
                }
                _ => false,
            };
            if !ordered {
                emit_untimed(
                    "epub→html four",
                    book.len() as u64,
                    Some(html.len() as u64),
                    "MISS: spine order does not match",
                );
                return;
            }
            let sample = measure(|| ready_bytes("epub", "html", &book));
            emit_ready("epub→html four", book.len() as u64, &sample);
            if !cfg!(debug_assertions) {
                println!(
                    "epub→html four p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                    sample.p50_ns / 1_000,
                    sample.p99_ns / 1_000,
                    mb_per_s(book.len() as u64, sample.p50_ns),
                    sample.allocs_p50,
                    sample.alloc_bytes_p50,
                    book.len(),
                    sample.output_bytes,
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", book.len()))
            {
                eprintln!("epub four hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html four", book.len() as u64, None, &err.message);
        }
    }
    let six = epub_six_spine();
    match convert("epub", "html", &six) {
        Ok(_) => {
            eprintln!("six-item spine produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", six.len()))
            {
                eprintln!("epub six miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html six", six.len() as u64, None, &err.message);
        }
    }
}

fn report_epub_five() {
    let four = epub_four_book();
    match convert("epub", "html", &four) {
        Ok(html) if four.len() == 28808 && html.len() == 27096 => {
            let text = String::from_utf8_lossy(&html);
            let ordered = match (
                text.find("First paragraph 0"),
                text.find("Second paragraph 0"),
                text.find("Third paragraph 0"),
                text.find("Fourth paragraph 0"),
            ) {
                (Some(first), Some(second), Some(third), Some(fourth)) => {
                    first < second && second < third && third < fourth
                }
                _ => false,
            };
            if !ordered {
                eprintln!("four-item epub order changed");
                std::process::exit(1);
            }
        }
        Ok(html) => {
            eprintln!(
                "four-item epub size changed input_b={} output_b={}",
                four.len(),
                html.len()
            );
            std::process::exit(1);
        }
        Err(err) => {
            eprintln!("four-item epub failed: {err}");
            std::process::exit(1);
        }
    }
    let small = epub_five_spine();
    match convert("epub", "html", &small) {
        Ok(html) => {
            let text = String::from_utf8_lossy(&html);
            let ordered = match (
                text.find("<p>One</p>"),
                text.find("<p>Two</p>"),
                text.find("<p>Three</p>"),
                text.find("<p>Four</p>"),
                text.find("<p>Five</p>"),
            ) {
                (Some(one), Some(two), Some(three), Some(four), Some(five)) => {
                    one < two && two < three && three < four && four < five
                }
                _ => false,
            };
            if small.len() != 2298 || !ordered {
                eprintln!(
                    "small five-item spine changed input_b={} output_b={}",
                    small.len(),
                    html.len()
                );
                std::process::exit(1);
            }
            emit_untimed(
                "epub→html five small",
                small.len() as u64,
                Some(html.len() as u64),
                "copied in spine order, not timed",
            );
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", small.len()))
            {
                eprintln!("small five-item spine hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html five small", small.len() as u64, None, &err.message);
            return;
        }
    }
    let book = epub_five_book();
    match convert("epub", "html", &book) {
        Ok(html) => {
            let text = String::from_utf8_lossy(&html);
            let ordered = match (
                text.find("First paragraph 0"),
                text.find("Second paragraph 0"),
                text.find("Third paragraph 0"),
                text.find("Fourth paragraph 0"),
                text.find("Fifth paragraph 0"),
            ) {
                (Some(first), Some(second), Some(third), Some(fourth), Some(fifth)) => {
                    first < second && second < third && third < fourth && fourth < fifth
                }
                _ => false,
            };
            if !ordered {
                emit_untimed(
                    "epub→html five",
                    book.len() as u64,
                    Some(html.len() as u64),
                    "MISS: spine order does not match",
                );
                return;
            }
            let sample = measure(|| ready_bytes("epub", "html", &book));
            emit_ready("epub→html five", book.len() as u64, &sample);
            if !cfg!(debug_assertions) {
                println!(
                    "epub→html five p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                    sample.p50_ns / 1_000,
                    sample.p99_ns / 1_000,
                    mb_per_s(book.len() as u64, sample.p50_ns),
                    sample.allocs_p50,
                    sample.alloc_bytes_p50,
                    book.len(),
                    sample.output_bytes,
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", book.len()))
            {
                eprintln!("epub five hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html five", book.len() as u64, None, &err.message);
        }
    }
    let six = epub_six_spine();
    match convert("epub", "html", &six) {
        Ok(_) => {
            eprintln!("six-item spine produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", six.len()))
            {
                eprintln!("epub six miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("epub→html six", six.len() as u64, None, &err.message);
        }
    }
}

fn epub_part(label: &str) -> String {
    let mut body = String::from(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\"><body>\n",
    );
    for i in 0..120 {
        body.push_str(&format!("<p>{label} paragraph {i} of the generated epub spine.</p>\n"));
    }
    body.push_str("</body></html>\n");
    body
}

fn epub_two_book() -> Vec<u8> {
    let first = epub_part("First");
    let second = epub_part("Second");
    let opf = epub_opf("<itemref idref=\"chap\"/>\n    <itemref idref=\"next\"/>");
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", first.as_bytes()),
        ("OEBPS/next.xhtml", second.as_bytes()),
    ])
}

fn epub_three_spine() -> Vec<u8> {
    let chapter = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>One</p></body></html>";
    let next = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>";
    let more = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Three</p></body></html>";
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap"/>
    <itemref idref="next"/>
    <itemref idref="more"/>
  </spine>
</package>
"#
    .to_string();
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", chapter),
        ("OEBPS/next.xhtml", next),
        ("OEBPS/more.xhtml", more),
    ])
}

fn epub_three_book() -> Vec<u8> {
    let first = epub_part("First");
    let second = epub_part("Second");
    let third = epub_part("Third");
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Notes</dc:title></metadata>
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap"/>
    <itemref idref="next"/>
    <itemref idref="more"/>
  </spine>
</package>
"#;
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", first.as_bytes()),
        ("OEBPS/next.xhtml", second.as_bytes()),
        ("OEBPS/more.xhtml", third.as_bytes()),
    ])
}

fn epub_four_spine() -> Vec<u8> {
    let chapter = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>One</p></body></html>";
    let next = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>";
    let more = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Three</p></body></html>";
    let tail = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Four</p></body></html>";
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
    <item id="tail" href="tail.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap"/>
    <itemref idref="next"/>
    <itemref idref="more"/>
    <itemref idref="tail"/>
  </spine>
</package>
"#;
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", chapter),
        ("OEBPS/next.xhtml", next),
        ("OEBPS/more.xhtml", more),
        ("OEBPS/tail.xhtml", tail),
    ])
}

fn epub_four_book() -> Vec<u8> {
    let first = epub_part("First");
    let second = epub_part("Second");
    let third = epub_part("Third");
    let fourth = epub_part("Fourth");
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Notes</dc:title></metadata>
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
    <item id="tail" href="tail.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap"/>
    <itemref idref="next"/>
    <itemref idref="more"/>
    <itemref idref="tail"/>
  </spine>
</package>
"#;
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", first.as_bytes()),
        ("OEBPS/next.xhtml", second.as_bytes()),
        ("OEBPS/more.xhtml", third.as_bytes()),
        ("OEBPS/tail.xhtml", fourth.as_bytes()),
    ])
}

fn epub_five_spine() -> Vec<u8> {
    let chapter = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>One</p></body></html>";
    let next = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>";
    let more = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Three</p></body></html>";
    let tail = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Four</p></body></html>";
    let end = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Five</p></body></html>";
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
    <item id="tail" href="tail.xhtml" media-type="application/xhtml+xml"/>
    <item id="end" href="end.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap"/>
    <itemref idref="next"/>
    <itemref idref="more"/>
    <itemref idref="tail"/>
    <itemref idref="end"/>
  </spine>
</package>
"#;
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", chapter),
        ("OEBPS/next.xhtml", next),
        ("OEBPS/more.xhtml", more),
        ("OEBPS/tail.xhtml", tail),
        ("OEBPS/end.xhtml", end),
    ])
}

fn epub_five_book() -> Vec<u8> {
    let first = epub_part("First");
    let second = epub_part("Second");
    let third = epub_part("Third");
    let fourth = epub_part("Fourth");
    let fifth = epub_part("Fifth");
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <metadata xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Notes</dc:title></metadata>
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
    <item id="tail" href="tail.xhtml" media-type="application/xhtml+xml"/>
    <item id="end" href="end.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap"/>
    <itemref idref="next"/>
    <itemref idref="more"/>
    <itemref idref="tail"/>
    <itemref idref="end"/>
  </spine>
</package>
"#;
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", first.as_bytes()),
        ("OEBPS/next.xhtml", second.as_bytes()),
        ("OEBPS/more.xhtml", third.as_bytes()),
        ("OEBPS/tail.xhtml", fourth.as_bytes()),
        ("OEBPS/end.xhtml", fifth.as_bytes()),
    ])
}

fn epub_six_spine() -> Vec<u8> {
    let chapter = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>One</p></body></html>";
    let next = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Two</p></body></html>";
    let more = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Three</p></body></html>";
    let tail = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Four</p></body></html>";
    let end = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Five</p></body></html>";
    let last = b"<?xml version=\"1.0\"?><html xmlns=\"http://www.w3.org/1999/xhtml\"><body><p>Six</p></body></html>";
    let opf = r#"<?xml version="1.0"?>
<package xmlns="http://www.idpf.org/2007/opf" version="3.0" unique-identifier="uid">
  <manifest>
    <item id="chap" href="chapter.xhtml" media-type="application/xhtml+xml"/>
    <item id="next" href="next.xhtml" media-type="application/xhtml+xml"/>
    <item id="more" href="more.xhtml" media-type="application/xhtml+xml"/>
    <item id="tail" href="tail.xhtml" media-type="application/xhtml+xml"/>
    <item id="end" href="end.xhtml" media-type="application/xhtml+xml"/>
    <item id="last" href="last.xhtml" media-type="application/xhtml+xml"/>
  </manifest>
  <spine>
    <itemref idref="chap"/>
    <itemref idref="next"/>
    <itemref idref="more"/>
    <itemref idref="tail"/>
    <itemref idref="end"/>
    <itemref idref="last"/>
  </spine>
</package>
"#;
    let container = epub_container();
    pack_stored(&[
        ("mimetype", b"application/epub+zip"),
        ("META-INF/container.xml", container.as_bytes()),
        ("OEBPS/content.opf", opf.as_bytes()),
        ("OEBPS/chapter.xhtml", chapter),
        ("OEBPS/next.xhtml", next),
        ("OEBPS/more.xhtml", more),
        ("OEBPS/tail.xhtml", tail),
        ("OEBPS/end.xhtml", end),
        ("OEBPS/last.xhtml", last),
    ])
}

fn report_ppt_html() {
    let text = ppt_html_text();
    let file = ppt_text_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("ppt", "html", &file) {
        Ok(html) => {
            let body = String::from_utf8_lossy(&html);
            if body.contains(&text) {
                let sample = measure(|| ready_bytes("ppt", "html", &file));
                emit_ready("ppt→html text", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→html text p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→html text",
                    file.len() as u64,
                    Some(html.len() as u64),
                    &format!(
                        "MISS: html does not contain the text (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html text", file.len() as u64, None, &err.message);
        }
    }
    let blob = b"not a ppt";
    match convert("ppt", "html", blob) {
        Ok(_) => {
            eprintln!("unreadable ppt produced html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains("compound document is not readable")
                || !err.message.contains(&format!("input_b={}", blob.len()))
            {
                eprintln!("ppt unreadable miss hid the reason: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html blob", blob.len() as u64, None, &err.message);
        }
    }
    let word = word97_document(&["Hi"]).unwrap_or_else(|err| {
        eprintln!("word fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("ppt", "html", &word) {
        Ok(_) => {
            eprintln!("word file produced ppt html");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains("has no PowerPoint Document stream")
                || !err.message.contains(&format!("input_b={}", word.len()))
            {
                eprintln!("ppt word miss hid the reason: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html word", word.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_html_two() {
    let one = ppt_text_document(&ppt_html_text()).unwrap_or_else(|err| {
        eprintln!("ppt one-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let one_html = convert("ppt", "html", &one).unwrap_or_else(|err| {
        eprintln!("ppt one-shape html failed: {err}");
        std::process::exit(1);
    });
    if one.len() != 5632
        || one_html.len() != 3193
        || !String::from_utf8_lossy(&one_html).contains(&ppt_html_text())
    {
        eprintln!(
            "ppt one-shape size changed: input_b={} output_b={}",
            one.len(),
            one_html.len()
        );
        std::process::exit(1);
    }
    let (first, second) = ppt_html_two_texts();
    let file = ppt_two_text_document(&first, &second).unwrap_or_else(|err| {
        eprintln!("ppt two-shape fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("ppt", "html", &file) {
        Ok(html) => {
            let body = String::from_utf8_lossy(&html);
            let first_p = format!("<p>{first}</p>");
            let second_p = format!("<p>{second}</p>");
            let ordered = body.find(&first_p).zip(body.find(&second_p));
            if body.contains(&first) && body.contains(&second) && ordered.is_some_and(|(a, b)| a < b)
            {
                let sample = measure(|| ready_bytes("ppt", "html", &file));
                emit_ready("ppt→html two", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→html two p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→html two",
                    file.len() as u64,
                    Some(html.len() as u64),
                    &format!(
                        "MISS: html does not contain both text shapes (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt two miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html two", file.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_html_three() {
    let one = ppt_text_document(&ppt_html_text()).unwrap_or_else(|err| {
        eprintln!("ppt one-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let one_html = convert("ppt", "html", &one).unwrap_or_else(|err| {
        eprintln!("ppt one-shape html failed: {err}");
        std::process::exit(1);
    });
    if one.len() != 5632 || one_html.len() != 3193 {
        eprintln!(
            "ppt one-shape size changed: input_b={} output_b={}",
            one.len(),
            one_html.len()
        );
        std::process::exit(1);
    }
    let (first, second) = ppt_html_two_texts();
    let two = ppt_two_text_document(&first, &second).unwrap_or_else(|err| {
        eprintln!("ppt two-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let two_html = convert("ppt", "html", &two).unwrap_or_else(|err| {
        eprintln!("ppt two-shape html failed: {err}");
        std::process::exit(1);
    });
    if two.len() != 6656 || two_html.len() != 4750 {
        eprintln!(
            "ppt two-shape size changed: input_b={} output_b={}",
            two.len(),
            two_html.len()
        );
        std::process::exit(1);
    }
    let (alpha, beta, gamma) = ppt_html_three_texts();
    let file = ppt_three_text_document(&alpha, &beta, &gamma).unwrap_or_else(|err| {
        eprintln!("ppt three-shape fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("ppt", "html", &file) {
        Ok(html) => {
            let body = String::from_utf8_lossy(&html);
            let alpha_p = format!("<p>{alpha}</p>");
            let beta_p = format!("<p>{beta}</p>");
            let gamma_p = format!("<p>{gamma}</p>");
            let a = body.find(&alpha_p);
            let b = body.find(&beta_p);
            let c = body.find(&gamma_p);
            let ordered = match (a, b, c) {
                (Some(a), Some(b), Some(c)) => a < b && b < c,
                _ => false,
            };
            if body.contains(&alpha) && body.contains(&beta) && body.contains(&gamma) && ordered {
                let sample = measure(|| ready_bytes("ppt", "html", &file));
                emit_ready("ppt→html three", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→html three p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→html three",
                    file.len() as u64,
                    Some(html.len() as u64),
                    &format!(
                        "MISS: html does not contain all three text shapes (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt three miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html three", file.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_html_four() {
    let one = ppt_text_document(&ppt_html_text()).unwrap_or_else(|err| {
        eprintln!("ppt one-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let one_html = convert("ppt", "html", &one).unwrap_or_else(|err| {
        eprintln!("ppt one-shape html failed: {err}");
        std::process::exit(1);
    });
    if one.len() != 5632 || one_html.len() != 3193 {
        eprintln!(
            "ppt one-shape size changed: input_b={} output_b={}",
            one.len(),
            one_html.len()
        );
        std::process::exit(1);
    }
    let (first, second) = ppt_html_two_texts();
    let two = ppt_two_text_document(&first, &second).unwrap_or_else(|err| {
        eprintln!("ppt two-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let two_html = convert("ppt", "html", &two).unwrap_or_else(|err| {
        eprintln!("ppt two-shape html failed: {err}");
        std::process::exit(1);
    });
    if two.len() != 6656 || two_html.len() != 4750 {
        eprintln!(
            "ppt two-shape size changed: input_b={} output_b={}",
            two.len(),
            two_html.len()
        );
        std::process::exit(1);
    }
    let (alpha, beta, gamma) = ppt_html_three_texts();
    let three = ppt_three_text_document(&alpha, &beta, &gamma).unwrap_or_else(|err| {
        eprintln!("ppt three-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let three_html = convert("ppt", "html", &three).unwrap_or_else(|err| {
        eprintln!("ppt three-shape html failed: {err}");
        std::process::exit(1);
    });
    if three.len() != 7168 || three_html.len() != 5257 {
        eprintln!(
            "ppt three-shape size changed: input_b={} output_b={}",
            three.len(),
            three_html.len()
        );
        std::process::exit(1);
    }
    let (north, east, south, west) = ppt_html_four_texts();
    let file = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("ppt", "html", &file) {
        Ok(html) => {
            let body = String::from_utf8_lossy(&html);
            let north_p = format!("<p>{north}</p>");
            let east_p = format!("<p>{east}</p>");
            let south_p = format!("<p>{south}</p>");
            let west_p = format!("<p>{west}</p>");
            let n = body.find(&north_p);
            let e = body.find(&east_p);
            let s = body.find(&south_p);
            let w = body.find(&west_p);
            let ordered = match (n, e, s, w) {
                (Some(n), Some(e), Some(s), Some(w)) => n < e && e < s && s < w,
                _ => false,
            };
            if body.contains(&north)
                && body.contains(&east)
                && body.contains(&south)
                && body.contains(&west)
                && ordered
            {
                let sample = measure(|| ready_bytes("ppt", "html", &file));
                emit_ready("ppt→html four", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→html four p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→html four",
                    file.len() as u64,
                    Some(html.len() as u64),
                    &format!(
                        "MISS: html does not contain all four text shapes (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt four miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html four", file.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_html_five() {
    let (north, east, south, west) = ppt_html_four_texts();
    let four = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let four_html = convert("ppt", "html", &four).unwrap_or_else(|err| {
        eprintln!("ppt four-shape html failed: {err}");
        std::process::exit(1);
    });
    if four.len() != 7680 || four_html.len() != 5814 {
        eprintln!(
            "ppt four-shape html size changed: input_b={} output_b={}",
            four.len(),
            four_html.len()
        );
        std::process::exit(1);
    }
    let (north, east, south, west, center) = ppt_html_five_texts();
    let file = ppt_five_text_document(&north, &east, &south, &west, &center).unwrap_or_else(|err| {
        eprintln!("ppt five-shape fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("ppt", "html", &file) {
        Ok(html) => {
            let body = String::from_utf8_lossy(&html);
            let north_p = format!("<p>{north}</p>");
            let east_p = format!("<p>{east}</p>");
            let south_p = format!("<p>{south}</p>");
            let west_p = format!("<p>{west}</p>");
            let center_p = format!("<p>{center}</p>");
            let n = body.find(&north_p);
            let e = body.find(&east_p);
            let s = body.find(&south_p);
            let w = body.find(&west_p);
            let c = body.find(&center_p);
            let ordered = match (n, e, s, w, c) {
                (Some(n), Some(e), Some(s), Some(w), Some(c)) => n < e && e < s && s < w && w < c,
                _ => false,
            };
            if body.contains(&north)
                && body.contains(&east)
                && body.contains(&south)
                && body.contains(&west)
                && body.contains(&center)
                && ordered
            {
                let sample = measure(|| ready_bytes("ppt", "html", &file));
                emit_ready("ppt→html five", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→html five p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→html five",
                    file.len() as u64,
                    Some(html.len() as u64),
                    &format!(
                        "MISS: html does not contain all five text shapes (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt five miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html five", file.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_html_six() {
    let (north, east, south, west) = ppt_html_four_texts();
    let four = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let four_html = convert("ppt", "html", &four).unwrap_or_else(|err| {
        eprintln!("ppt four-shape html failed: {err}");
        std::process::exit(1);
    });
    if four.len() != 7680 || four_html.len() != 5814 {
        eprintln!(
            "ppt four-shape html size changed: input_b={} output_b={}",
            four.len(),
            four_html.len()
        );
        std::process::exit(1);
    }
    let (north, east, south, west, center) = ppt_html_five_texts();
    let five = ppt_five_text_document(&north, &east, &south, &west, &center).unwrap_or_else(|err| {
        eprintln!("ppt five-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let five_html = convert("ppt", "html", &five).unwrap_or_else(|err| {
        eprintln!("ppt five-shape html failed: {err}");
        std::process::exit(1);
    });
    if five.len() != 9216 || five_html.len() != 7286 {
        eprintln!(
            "ppt five-shape html size changed: input_b={} output_b={}",
            five.len(),
            five_html.len()
        );
        std::process::exit(1);
    }
    let (north, east, south, west, center, inner) = ppt_html_six_texts();
    let file = ppt_six_text_document(&north, &east, &south, &west, &center, &inner).unwrap_or_else(
        |err| {
            eprintln!("ppt six-shape fixture failed: {err}");
            std::process::exit(1);
        },
    );
    match convert("ppt", "html", &file) {
        Ok(html) => {
            let body = String::from_utf8_lossy(&html);
            let north_p = format!("<p>{north}</p>");
            let east_p = format!("<p>{east}</p>");
            let south_p = format!("<p>{south}</p>");
            let west_p = format!("<p>{west}</p>");
            let center_p = format!("<p>{center}</p>");
            let inner_p = format!("<p>{inner}</p>");
            let n = body.find(&north_p);
            let e = body.find(&east_p);
            let s = body.find(&south_p);
            let w = body.find(&west_p);
            let c = body.find(&center_p);
            let i = body.find(&inner_p);
            let ordered = match (n, e, s, w, c, i) {
                (Some(n), Some(e), Some(s), Some(w), Some(c), Some(i)) => {
                    n < e && e < s && s < w && w < c && c < i
                }
                _ => false,
            };
            if body.contains(&north)
                && body.contains(&east)
                && body.contains(&south)
                && body.contains(&west)
                && body.contains(&center)
                && body.contains(&inner)
                && ordered
            {
                let sample = measure(|| ready_bytes("ppt", "html", &file));
                emit_ready("ppt→html six", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→html six p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→html six",
                    file.len() as u64,
                    Some(html.len() as u64),
                    &format!(
                        "MISS: html does not contain all six text shapes (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt six miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html six", file.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_html_seven() {
    let (north, east, south, west) = ppt_html_four_texts();
    let four = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let four_html = convert("ppt", "html", &four).unwrap_or_else(|err| {
        eprintln!("ppt four-shape html failed: {err}");
        std::process::exit(1);
    });
    if four.len() != 7680 || four_html.len() != 5814 {
        eprintln!(
            "ppt four-shape html size changed: input_b={} output_b={}",
            four.len(),
            four_html.len()
        );
        std::process::exit(1);
    }
    let (north, east, south, west, center) = ppt_html_five_texts();
    let five = ppt_five_text_document(&north, &east, &south, &west, &center).unwrap_or_else(|err| {
        eprintln!("ppt five-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let five_html = convert("ppt", "html", &five).unwrap_or_else(|err| {
        eprintln!("ppt five-shape html failed: {err}");
        std::process::exit(1);
    });
    if five.len() != 9216 || five_html.len() != 7286 {
        eprintln!(
            "ppt five-shape html size changed: input_b={} output_b={}",
            five.len(),
            five_html.len()
        );
        std::process::exit(1);
    }
    let (north, east, south, west, center, inner) = ppt_html_six_texts();
    let six = ppt_six_text_document(&north, &east, &south, &west, &center, &inner).unwrap_or_else(
        |err| {
            eprintln!("ppt six-shape fixture failed: {err}");
            std::process::exit(1);
        },
    );
    let six_html = convert("ppt", "html", &six).unwrap_or_else(|err| {
        eprintln!("ppt six-shape html failed: {err}");
        std::process::exit(1);
    });
    if six.len() != 10752 || six_html.len() != 8733 {
        eprintln!(
            "ppt six-shape html size changed: input_b={} output_b={}",
            six.len(),
            six_html.len()
        );
        std::process::exit(1);
    }
    let (north, east, south, west, center, inner, outer) = ppt_html_seven_texts();
    let file = ppt_seven_text_document(
        &north, &east, &south, &west, &center, &inner, &outer,
    )
    .unwrap_or_else(|err| {
        eprintln!("ppt seven-shape fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("ppt", "html", &file) {
        Ok(html) => {
            let body = String::from_utf8_lossy(&html);
            let north_p = format!("<p>{north}</p>");
            let east_p = format!("<p>{east}</p>");
            let south_p = format!("<p>{south}</p>");
            let west_p = format!("<p>{west}</p>");
            let center_p = format!("<p>{center}</p>");
            let inner_p = format!("<p>{inner}</p>");
            let outer_p = format!("<p>{outer}</p>");
            let n = body.find(&north_p);
            let e = body.find(&east_p);
            let s = body.find(&south_p);
            let w = body.find(&west_p);
            let c = body.find(&center_p);
            let i = body.find(&inner_p);
            let o = body.find(&outer_p);
            let ordered = match (n, e, s, w, c, i, o) {
                (Some(n), Some(e), Some(s), Some(w), Some(c), Some(i), Some(o)) => {
                    n < e && e < s && s < w && w < c && c < i && i < o
                }
                _ => false,
            };
            if body.contains(&north)
                && body.contains(&east)
                && body.contains(&south)
                && body.contains(&west)
                && body.contains(&center)
                && body.contains(&inner)
                && body.contains(&outer)
                && ordered
            {
                let sample = measure(|| ready_bytes("ppt", "html", &file));
                emit_ready("ppt→html seven", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→html seven p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→html seven",
                    file.len() as u64,
                    Some(html.len() as u64),
                    &format!(
                        "MISS: html does not contain all seven text shapes (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt seven miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→html seven", file.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_pdf_four() {
    let (north, east, south, west) = ppt_html_four_texts();
    let file = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let html = convert("ppt", "html", &file).unwrap_or_else(|err| {
        eprintln!("ppt four-shape html failed: {err}");
        std::process::exit(1);
    });
    if file.len() != 7680 || html.len() != 5814 {
        eprintln!(
            "ppt four-shape html size changed: input_b={} output_b={}",
            file.len(),
            html.len()
        );
        std::process::exit(1);
    }
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let n = extracted.find(&north);
            let e = extracted.find(&east);
            let s = extracted.find(&south);
            let w = extracted.find(&west);
            let ordered = match (n, e, s, w) {
                (Some(n), Some(e), Some(s), Some(w)) => n < e && e < s && s < w,
                _ => false,
            };
            if ordered {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf four", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf four p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→pdf four",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text does not contain the text (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt pdf miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→pdf four", file.len() as u64, None, &err.message);
        }
    }
    let one = ppt_text_document("Only one generated text shape.").unwrap_or_else(|err| {
        eprintln!("ppt one-shape fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("ppt", "pdf", &one) {
        Ok(_) => {
            eprintln!("one-shape ppt produced pdf");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains("not four text shapes")
                || !err.message.contains(&format!("input_b={}", one.len()))
            {
                eprintln!("ppt pdf one miss hid the reason: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→pdf one", one.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_pdf_one() {
    let (north, east, south, west) = ppt_html_four_texts();
    let four = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let four_pdf = convert("ppt", "pdf", &four).unwrap_or_else(|err| {
        eprintln!("ppt four-shape pdf changed: {err}");
        std::process::exit(1);
    });
    if four.len() != 7680 || four_pdf.len() != 6710 {
        eprintln!(
            "ppt four-shape pdf size changed: input_b={} output_b={}",
            four.len(),
            four_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_html_text();
    let file = ppt_text_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt one-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let html = convert("ppt", "html", &file).unwrap_or_else(|err| {
        eprintln!("ppt one-shape html failed: {err}");
        std::process::exit(1);
    });
    if file.len() != 5632 || html.len() != 3193 {
        eprintln!(
            "ppt one-shape html size changed: input_b={} output_b={}",
            file.len(),
            html.len()
        );
        std::process::exit(1);
    }
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            if extracted.contains(&text) {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf one shape", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf one shape p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                emit_untimed(
                    "ppt→pdf one shape",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text does not contain the text (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains(&format!("input_b={}", file.len()))
            {
                eprintln!("ppt pdf one miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→pdf one shape", file.len() as u64, None, &err.message);
        }
    }
    let small = ppt_text_document("Only one generated text shape.").unwrap_or_else(|err| {
        eprintln!("ppt 2560 fixture failed: {err}");
        std::process::exit(1);
    });
    if small.len() != 2560 {
        eprintln!("2560-byte miss changed size: {}", small.len());
        std::process::exit(1);
    }
    match convert("ppt", "pdf", &small) {
        Ok(_) => {
            eprintln!("2560-byte ppt produced pdf");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:")
                || !err.message.contains("not four text shapes")
                || !err.message.contains("input_b=2560")
            {
                eprintln!("ppt pdf 2560 miss hid the reason: {err}");
                std::process::exit(1);
            }
            emit_untimed("ppt→pdf 2560", small.len() as u64, None, &err.message);
        }
    }
}

fn report_ppt_pdf_picture() {
    let (north, east, south, west) = ppt_html_four_texts();
    let four = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let four_pdf = convert("ppt", "pdf", &four).unwrap_or_else(|err| {
        eprintln!("ppt four-shape pdf changed: {err}");
        std::process::exit(1);
    });
    if four.len() != 7680 || four_pdf.len() != 6710 {
        eprintln!(
            "ppt four-shape pdf size changed: input_b={} output_b={}",
            four.len(),
            four_pdf.len()
        );
        std::process::exit(1);
    }
    let one_text = ppt_html_text();
    let one = ppt_text_document(&one_text).unwrap_or_else(|err| {
        eprintln!("ppt one-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let one_pdf = convert("ppt", "pdf", &one).unwrap_or_else(|err| {
        eprintln!("ppt one-shape pdf changed: {err}");
        std::process::exit(1);
    });
    if one.len() != 5632 || one_pdf.len() != 3981 {
        eprintln!(
            "ppt one-shape pdf size changed: input_b={} output_b={}",
            one.len(),
            one_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_picture_text();
    let file = ppt_text_picture_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt picture fixture failed: {err}");
        std::process::exit(1);
    });
    if file.len() == 5632 || file.len() == 7680 || file.len() == 2560 {
        eprintln!("ppt picture fixture collided with a published file: {}", file.len());
        std::process::exit(1);
    }
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(15).any(|w| w == b"/Subtype /Image")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(10).any(|w| w == b"/Height 48");
            if extracted.contains(&text) && image {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf picture", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf picture p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                eprintln!("picture was not extracted into the pdf");
                emit_untimed(
                    "ppt→pdf picture",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text or picture is missing (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("picture could not be extracted: {err}");
        }
    }
}

fn ppt_picture_text() -> String {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "Picture shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    text
}

fn report_ppt_pdf_indexed() {
    let (north, east, south, west) = ppt_html_four_texts();
    let four = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let four_pdf = convert("ppt", "pdf", &four).unwrap_or_else(|err| {
        eprintln!("ppt four-shape pdf changed: {err}");
        std::process::exit(1);
    });
    if four.len() != 7680 || four_pdf.len() != 6710 {
        eprintln!(
            "ppt four-shape pdf size changed: input_b={} output_b={}",
            four.len(),
            four_pdf.len()
        );
        std::process::exit(1);
    }
    let one_text = ppt_html_text();
    let one = ppt_text_document(&one_text).unwrap_or_else(|err| {
        eprintln!("ppt one-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let one_pdf = convert("ppt", "pdf", &one).unwrap_or_else(|err| {
        eprintln!("ppt one-shape pdf changed: {err}");
        std::process::exit(1);
    });
    if one.len() != 5632 || one_pdf.len() != 3981 {
        eprintln!(
            "ppt one-shape pdf size changed: input_b={} output_b={}",
            one.len(),
            one_pdf.len()
        );
        std::process::exit(1);
    }
    let picture_text = ppt_picture_text();
    let picture = ppt_text_picture_document(&picture_text).unwrap_or_else(|err| {
        eprintln!("ppt 24-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let picture_pdf = convert("ppt", "pdf", &picture).unwrap_or_else(|err| {
        eprintln!("ppt 24-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if picture.len() != 14336 || picture_pdf.len() != 1991 {
        eprintln!(
            "ppt 24-bit picture size changed: input_b={} output_b={}",
            picture.len(),
            picture_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_indexed_text();
    let file = ppt_text_indexed_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    if file.len() == 14336 || file.len() == 5632 || file.len() == 7680 {
        eprintln!("ppt 8-bit fixture collided with a published file: {}", file.len());
        std::process::exit(1);
    }
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(15).any(|w| w == b"/Subtype /Image")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(10).any(|w| w == b"/Height 64");
            if extracted.contains(&text) && image {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf indexed", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf indexed p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                    );
                }
            } else {
                eprintln!("8-bit picture was not extracted into the pdf");
                emit_untimed(
                    "ppt→pdf indexed",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text or picture is missing (input_b={})",
                        file.len()
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("8-bit picture could not be extracted: {err}");
        }
    }
}

fn report_ppt_pdf_nibble() {
    let (north, east, south, west) = ppt_html_four_texts();
    let four = ppt_four_text_document(&north, &east, &south, &west).unwrap_or_else(|err| {
        eprintln!("ppt four-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let four_pdf = convert("ppt", "pdf", &four).unwrap_or_else(|err| {
        eprintln!("ppt four-shape pdf changed: {err}");
        std::process::exit(1);
    });
    if four.len() != 7680 || four_pdf.len() != 6710 {
        eprintln!(
            "ppt four-shape pdf size changed: input_b={} output_b={}",
            four.len(),
            four_pdf.len()
        );
        std::process::exit(1);
    }
    let one_text = ppt_html_text();
    let one = ppt_text_document(&one_text).unwrap_or_else(|err| {
        eprintln!("ppt one-shape fixture failed: {err}");
        std::process::exit(1);
    });
    let one_pdf = convert("ppt", "pdf", &one).unwrap_or_else(|err| {
        eprintln!("ppt one-shape pdf changed: {err}");
        std::process::exit(1);
    });
    if one.len() != 5632 || one_pdf.len() != 3981 {
        eprintln!(
            "ppt one-shape pdf size changed: input_b={} output_b={}",
            one.len(),
            one_pdf.len()
        );
        std::process::exit(1);
    }
    let picture_text = ppt_picture_text();
    let picture = ppt_text_picture_document(&picture_text).unwrap_or_else(|err| {
        eprintln!("ppt 24-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let picture_pdf = convert("ppt", "pdf", &picture).unwrap_or_else(|err| {
        eprintln!("ppt 24-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if picture.len() != 14336 || picture_pdf.len() != 1991 {
        eprintln!(
            "ppt 24-bit picture size changed: input_b={} output_b={}",
            picture.len(),
            picture_pdf.len()
        );
        std::process::exit(1);
    }
    let indexed_text = ppt_indexed_text();
    let indexed = ppt_text_indexed_document(&indexed_text).unwrap_or_else(|err| {
        eprintln!("ppt 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let indexed_pdf = convert("ppt", "pdf", &indexed).unwrap_or_else(|err| {
        eprintln!("ppt 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if indexed.len() != 8704 || indexed_pdf.len() != 2199 {
        eprintln!(
            "ppt 8-bit picture size changed: input_b={} output_b={}",
            indexed.len(),
            indexed_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_nibble_text();
    let file = ppt_text_nibble_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    if file.len() == 8704 || file.len() == 14336 || file.len() == 5632 || file.len() == 7680 {
        eprintln!("ppt 4-bit fixture collided with a published file: {}", file.len());
        std::process::exit(1);
    }
    let (width, height, rgb) = nibble_rgb();
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(15).any(|w| w == b"/Subtype /Image")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(11).any(|w| w == b"/Height 160");
            let pixels_match = pdf_image_matches(&pdf, width, height, &rgb).unwrap_or(false);
            if extracted.contains(&text) && image && pixels_match {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf nibble", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf nibble p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={} pixels_match={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                        pixels_match,
                    );
                }
            } else {
                eprintln!(
                    "4-bit picture was not extracted into the pdf (pixels_match={pixels_match})"
                );
                emit_untimed(
                    "ppt→pdf nibble",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text, picture, or pixels (input_b={} pixels_match={})",
                        file.len(),
                        pixels_match
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("4-bit picture could not be extracted: {err}");
        }
    }
}

fn report_ppt_pdf_indexed_two() {
    let indexed_text = ppt_indexed_text();
    let indexed = ppt_text_indexed_document(&indexed_text).unwrap_or_else(|err| {
        eprintln!("ppt 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let indexed_pdf = convert("ppt", "pdf", &indexed).unwrap_or_else(|err| {
        eprintln!("ppt 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if indexed.len() != 8704 || indexed_pdf.len() != 2199 {
        eprintln!(
            "ppt 8-bit picture size changed: input_b={} output_b={}",
            indexed.len(),
            indexed_pdf.len()
        );
        std::process::exit(1);
    }
    let picture_text = ppt_picture_text();
    let picture = ppt_text_picture_document(&picture_text).unwrap_or_else(|err| {
        eprintln!("ppt 24-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let picture_pdf = convert("ppt", "pdf", &picture).unwrap_or_else(|err| {
        eprintln!("ppt 24-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if picture.len() != 14336 || picture_pdf.len() != 1991 {
        eprintln!(
            "ppt 24-bit picture size changed: input_b={} output_b={}",
            picture.len(),
            picture_pdf.len()
        );
        std::process::exit(1);
    }
    let nibble_text = ppt_nibble_text();
    let nibble = ppt_text_nibble_document(&nibble_text).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let nibble_pdf = convert("ppt", "pdf", &nibble).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if nibble.len() != 9216 || nibble_pdf.len() != 2469 {
        eprintln!(
            "ppt 4-bit picture size changed: input_b={} output_b={}",
            nibble.len(),
            nibble_pdf.len()
        );
        std::process::exit(1);
    }
    let (first, second) = ppt_two_indexed_text();
    let file = ppt_two_text_indexed_document(&first, &second).unwrap_or_else(|err| {
        eprintln!("ppt two-shape 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    for banned in [2560, 5632, 6656, 7168, 7680, 8704, 9216, 14336] {
        if file.len() == banned {
            eprintln!(
                "ppt two-shape 8-bit fixture collided with a published file: {}",
                file.len()
            );
            std::process::exit(1);
        }
    }
    let (width, height, rgb) = indexed_rgb();
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(15).any(|w| w == b"/Subtype /Image")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(10).any(|w| w == b"/Height 64");
            let pixels_match = pdf_image_matches(&pdf, width, height, &rgb).unwrap_or(false);
            if extracted.contains(&first) && extracted.contains(&second) && image && pixels_match {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf indexed two", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf indexed two p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={} pixels_match={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                        pixels_match,
                    );
                }
            } else {
                eprintln!(
                    "two-shape 8-bit picture was not extracted into the pdf (pixels_match={pixels_match})"
                );
                emit_untimed(
                    "ppt→pdf indexed two",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text, picture, or pixels (input_b={} pixels_match={})",
                        file.len(),
                        pixels_match
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("two-shape 8-bit picture could not be extracted: {err}");
        }
    }
}

fn indexed_rgb() -> (u32, u32, Vec<u8>) {
    indexed_rgb_add(0)
}

fn indexed_rgb_add(add: u8) -> (u32, u32, Vec<u8>) {
    let width = 80u32;
    let height = 64u32;
    let mut rgb = vec![0u8; (width * height * 3) as usize];
    for y in 0..height as usize {
        let index = (y as u8).wrapping_add(add);
        for x in 0..width as usize {
            let d = (y * width as usize + x) * 3;
            rgb[d] = index;
            rgb[d + 1] = index.wrapping_mul(3).wrapping_add(20);
            rgb[d + 2] = 180u8.wrapping_sub(index);
        }
    }
    (width, height, rgb)
}

fn report_ppt_pdf_indexed_images() {
    let (first_text, second_text) = ppt_two_indexed_text();
    let two = ppt_two_text_indexed_document(&first_text, &second_text).unwrap_or_else(|err| {
        eprintln!("ppt two-shape 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let two_pdf = convert("ppt", "pdf", &two).unwrap_or_else(|err| {
        eprintln!("ppt two-shape 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if two.len() != 9728 || two_pdf.len() != 3085 {
        eprintln!(
            "ppt two-shape 8-bit picture size changed: input_b={} output_b={}",
            two.len(),
            two_pdf.len()
        );
        std::process::exit(1);
    }
    let indexed_text = ppt_indexed_text();
    let indexed = ppt_text_indexed_document(&indexed_text).unwrap_or_else(|err| {
        eprintln!("ppt 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let indexed_pdf = convert("ppt", "pdf", &indexed).unwrap_or_else(|err| {
        eprintln!("ppt 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if indexed.len() != 8704 || indexed_pdf.len() != 2199 {
        eprintln!(
            "ppt 8-bit picture size changed: input_b={} output_b={}",
            indexed.len(),
            indexed_pdf.len()
        );
        std::process::exit(1);
    }
    let nibble_text = ppt_nibble_text();
    let nibble = ppt_text_nibble_document(&nibble_text).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let nibble_pdf = convert("ppt", "pdf", &nibble).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if nibble.len() != 9216 || nibble_pdf.len() != 2469 {
        eprintln!(
            "ppt 4-bit picture size changed: input_b={} output_b={}",
            nibble.len(),
            nibble_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_indexed_images_text();
    let file = ppt_text_two_indexed_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt two 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    for banned in [2560, 5632, 6656, 7168, 7680, 8704, 9216, 9728, 14336] {
        if file.len() == banned {
            eprintln!(
                "ppt two 8-bit picture fixture collided with a published file: {}",
                file.len()
            );
            std::process::exit(1);
        }
    }
    let (width, height, first_rgb) = indexed_rgb_add(0);
    let (_, _, second_rgb) = indexed_rgb_add(1);
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(8).any(|w| w == b"/Im1 Do ")
                && pdf.windows(8).any(|w| w == b"/Im2 Do ")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(10).any(|w| w == b"/Height 64");
            let first_match = pdf_image_matches(&pdf, width, height, &first_rgb).unwrap_or(false);
            let second_match =
                pdf_image_matches(&pdf, width, height, &second_rgb).unwrap_or(false);
            let pixels_match = first_match && second_match && first_rgb != second_rgb;
            if extracted.contains(&text) && image && pixels_match {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf indexed images", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf indexed images p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={} pixels_match={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                        pixels_match,
                    );
                }
            } else {
                eprintln!(
                    "two 8-bit pictures were not extracted into the pdf (pixels_match={pixels_match})"
                );
                emit_untimed(
                    "ppt→pdf indexed images",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text, picture, or pixels (input_b={} pixels_match={})",
                        file.len(),
                        pixels_match
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("two 8-bit pictures could not be extracted: {err}");
        }
    }
}

fn ppt_indexed_images_text() -> String {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "Pair indexed shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    text
}

fn report_ppt_pdf_indexed_three() {
    let two_text = ppt_indexed_images_text();
    let two = ppt_text_two_indexed_document(&two_text).unwrap_or_else(|err| {
        eprintln!("ppt two 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let two_pdf = convert("ppt", "pdf", &two).unwrap_or_else(|err| {
        eprintln!("ppt two 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if two.len() != 14848 || two_pdf.len() != 2816 {
        eprintln!(
            "ppt two 8-bit picture size changed: input_b={} output_b={}",
            two.len(),
            two_pdf.len()
        );
        std::process::exit(1);
    }
    let (first_text, second_text) = ppt_two_indexed_text();
    let shapes = ppt_two_text_indexed_document(&first_text, &second_text).unwrap_or_else(|err| {
        eprintln!("ppt two-shape 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let shapes_pdf = convert("ppt", "pdf", &shapes).unwrap_or_else(|err| {
        eprintln!("ppt two-shape 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if shapes.len() != 9728 || shapes_pdf.len() != 3085 {
        eprintln!(
            "ppt two-shape 8-bit picture size changed: input_b={} output_b={}",
            shapes.len(),
            shapes_pdf.len()
        );
        std::process::exit(1);
    }
    let nibble_text = ppt_nibble_text();
    let nibble = ppt_text_nibble_document(&nibble_text).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let nibble_pdf = convert("ppt", "pdf", &nibble).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if nibble.len() != 9216 || nibble_pdf.len() != 2469 {
        eprintln!(
            "ppt 4-bit picture size changed: input_b={} output_b={}",
            nibble.len(),
            nibble_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_indexed_three_text();
    let file = ppt_text_three_indexed_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt three 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    for banned in [2560, 5632, 6656, 7168, 7680, 8704, 9216, 9728, 14336, 14848] {
        if file.len() == banned {
            eprintln!(
                "ppt three 8-bit picture fixture collided with a published file: {}",
                file.len()
            );
            std::process::exit(1);
        }
    }
    let (width, height, first_rgb) = indexed_rgb_add(0);
    let (_, _, second_rgb) = indexed_rgb_add(1);
    let (_, _, third_rgb) = indexed_rgb_add(2);
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(8).any(|w| w == b"/Im1 Do ")
                && pdf.windows(8).any(|w| w == b"/Im2 Do ")
                && pdf.windows(8).any(|w| w == b"/Im3 Do ")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(10).any(|w| w == b"/Height 64");
            let first_match = pdf_image_matches(&pdf, width, height, &first_rgb).unwrap_or(false);
            let second_match =
                pdf_image_matches(&pdf, width, height, &second_rgb).unwrap_or(false);
            let third_match = pdf_image_matches(&pdf, width, height, &third_rgb).unwrap_or(false);
            let pixels_match = first_match
                && second_match
                && third_match
                && first_rgb != second_rgb
                && second_rgb != third_rgb;
            if extracted.contains(&text) && image && pixels_match {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf indexed three", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf indexed three p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={} pixels_match={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                        pixels_match,
                    );
                }
            } else {
                eprintln!(
                    "three 8-bit pictures were not extracted into the pdf (pixels_match={pixels_match})"
                );
                emit_untimed(
                    "ppt→pdf indexed three",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text, picture, or pixels (input_b={} pixels_match={})",
                        file.len(),
                        pixels_match
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("three 8-bit pictures could not be extracted: {err}");
        }
    }
}

fn ppt_indexed_three_text() -> String {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "Triple indexed shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    text
}

fn report_ppt_pdf_indexed_four() {
    let three_text = ppt_indexed_three_text();
    let three = ppt_text_three_indexed_document(&three_text).unwrap_or_else(|err| {
        eprintln!("ppt three 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let three_pdf = convert("ppt", "pdf", &three).unwrap_or_else(|err| {
        eprintln!("ppt three 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if three.len() != 21504 || three_pdf.len() != 3421 {
        eprintln!(
            "ppt three 8-bit picture size changed: input_b={} output_b={}",
            three.len(),
            three_pdf.len()
        );
        std::process::exit(1);
    }
    let two_text = ppt_indexed_images_text();
    let two = ppt_text_two_indexed_document(&two_text).unwrap_or_else(|err| {
        eprintln!("ppt two 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let two_pdf = convert("ppt", "pdf", &two).unwrap_or_else(|err| {
        eprintln!("ppt two 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if two.len() != 14848 || two_pdf.len() != 2816 {
        eprintln!(
            "ppt two 8-bit picture size changed: input_b={} output_b={}",
            two.len(),
            two_pdf.len()
        );
        std::process::exit(1);
    }
    let nibble_text = ppt_nibble_text();
    let nibble = ppt_text_nibble_document(&nibble_text).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let nibble_pdf = convert("ppt", "pdf", &nibble).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if nibble.len() != 9216 || nibble_pdf.len() != 2469 {
        eprintln!(
            "ppt 4-bit picture size changed: input_b={} output_b={}",
            nibble.len(),
            nibble_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_indexed_four_text();
    let file = ppt_text_four_indexed_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt four 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    for banned in [2560, 5632, 6656, 7168, 7680, 8704, 9216, 9728, 14336, 14848, 21504] {
        if file.len() == banned {
            eprintln!(
                "ppt four 8-bit picture fixture collided with a published file: {}",
                file.len()
            );
            std::process::exit(1);
        }
    }
    let (width, height, first_rgb) = indexed_rgb_add(0);
    let (_, _, second_rgb) = indexed_rgb_add(1);
    let (_, _, third_rgb) = indexed_rgb_add(2);
    let (_, _, fourth_rgb) = indexed_rgb_add(3);
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(8).any(|w| w == b"/Im1 Do ")
                && pdf.windows(8).any(|w| w == b"/Im2 Do ")
                && pdf.windows(8).any(|w| w == b"/Im3 Do ")
                && pdf.windows(8).any(|w| w == b"/Im4 Do ")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(10).any(|w| w == b"/Height 64");
            let first_match = pdf_image_matches(&pdf, width, height, &first_rgb).unwrap_or(false);
            let second_match =
                pdf_image_matches(&pdf, width, height, &second_rgb).unwrap_or(false);
            let third_match = pdf_image_matches(&pdf, width, height, &third_rgb).unwrap_or(false);
            let fourth_match =
                pdf_image_matches(&pdf, width, height, &fourth_rgb).unwrap_or(false);
            let pixels_match = first_match
                && second_match
                && third_match
                && fourth_match
                && first_rgb != second_rgb
                && second_rgb != third_rgb
                && third_rgb != fourth_rgb;
            if extracted.contains(&text) && image && pixels_match {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf indexed four", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf indexed four p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={} pixels_match={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                        pixels_match,
                    );
                }
            } else {
                eprintln!(
                    "four 8-bit pictures were not extracted into the pdf (pixels_match={pixels_match})"
                );
                emit_untimed(
                    "ppt→pdf indexed four",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text, picture, or pixels (input_b={} pixels_match={})",
                        file.len(),
                        pixels_match
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("four 8-bit pictures could not be extracted: {err}");
        }
    }
}

fn ppt_indexed_four_text() -> String {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "Quad indexed shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    text
}

fn report_ppt_pdf_indexed_five() {
    let four_text = ppt_indexed_four_text();
    let four = ppt_text_four_indexed_document(&four_text).unwrap_or_else(|err| {
        eprintln!("ppt four 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let four_pdf = convert("ppt", "pdf", &four).unwrap_or_else(|err| {
        eprintln!("ppt four 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if four.len() != 27648 || four_pdf.len() != 3978 {
        eprintln!(
            "ppt four 8-bit picture size changed: input_b={} output_b={}",
            four.len(),
            four_pdf.len()
        );
        std::process::exit(1);
    }
    let three_text = ppt_indexed_three_text();
    let three = ppt_text_three_indexed_document(&three_text).unwrap_or_else(|err| {
        eprintln!("ppt three 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let three_pdf = convert("ppt", "pdf", &three).unwrap_or_else(|err| {
        eprintln!("ppt three 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if three.len() != 21504 || three_pdf.len() != 3421 {
        eprintln!(
            "ppt three 8-bit picture size changed: input_b={} output_b={}",
            three.len(),
            three_pdf.len()
        );
        std::process::exit(1);
    }
    let nibble_text = ppt_nibble_text();
    let nibble = ppt_text_nibble_document(&nibble_text).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let nibble_pdf = convert("ppt", "pdf", &nibble).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if nibble.len() != 9216 || nibble_pdf.len() != 2469 {
        eprintln!(
            "ppt 4-bit picture size changed: input_b={} output_b={}",
            nibble.len(),
            nibble_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_indexed_five_text();
    let file = ppt_text_five_indexed_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt five 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    for banned in [2560, 5632, 6656, 7168, 7680, 8704, 9216, 9728, 14336, 14848, 21504, 27648] {
        if file.len() == banned {
            eprintln!(
                "ppt five 8-bit picture fixture collided with a published file: {}",
                file.len()
            );
            std::process::exit(1);
        }
    }
    let (width, height, first_rgb) = indexed_rgb_add(0);
    let (_, _, second_rgb) = indexed_rgb_add(1);
    let (_, _, third_rgb) = indexed_rgb_add(2);
    let (_, _, fourth_rgb) = indexed_rgb_add(3);
    let (_, _, fifth_rgb) = indexed_rgb_add(4);
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(8).any(|w| w == b"/Im1 Do ")
                && pdf.windows(8).any(|w| w == b"/Im2 Do ")
                && pdf.windows(8).any(|w| w == b"/Im3 Do ")
                && pdf.windows(8).any(|w| w == b"/Im4 Do ")
                && pdf.windows(8).any(|w| w == b"/Im5 Do ")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(10).any(|w| w == b"/Height 64");
            let first_match = pdf_image_matches(&pdf, width, height, &first_rgb).unwrap_or(false);
            let second_match =
                pdf_image_matches(&pdf, width, height, &second_rgb).unwrap_or(false);
            let third_match = pdf_image_matches(&pdf, width, height, &third_rgb).unwrap_or(false);
            let fourth_match =
                pdf_image_matches(&pdf, width, height, &fourth_rgb).unwrap_or(false);
            let fifth_match = pdf_image_matches(&pdf, width, height, &fifth_rgb).unwrap_or(false);
            let pixels_match = first_match
                && second_match
                && third_match
                && fourth_match
                && fifth_match
                && first_rgb != second_rgb
                && second_rgb != third_rgb
                && third_rgb != fourth_rgb
                && fourth_rgb != fifth_rgb;
            if extracted.contains(&text) && image && pixels_match {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf indexed five", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf indexed five p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={} pixels_match={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                        pixels_match,
                    );
                }
            } else {
                eprintln!(
                    "five 8-bit pictures were not extracted into the pdf (pixels_match={pixels_match})"
                );
                emit_untimed(
                    "ppt→pdf indexed five",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text, picture, or pixels (input_b={} pixels_match={})",
                        file.len(),
                        pixels_match
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("five 8-bit pictures could not be extracted: {err}");
        }
    }
}

fn ppt_indexed_five_text() -> String {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "Quint indexed shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    text
}

fn report_ppt_pdf_indexed_six() {
    let five_text = ppt_indexed_five_text();
    let five = ppt_text_five_indexed_document(&five_text).unwrap_or_else(|err| {
        eprintln!("ppt five 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let five_pdf = convert("ppt", "pdf", &five).unwrap_or_else(|err| {
        eprintln!("ppt five 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if five.len() != 33792 || five_pdf.len() != 4574 {
        eprintln!(
            "ppt five 8-bit picture size changed: input_b={} output_b={}",
            five.len(),
            five_pdf.len()
        );
        std::process::exit(1);
    }
    let four_text = ppt_indexed_four_text();
    let four = ppt_text_four_indexed_document(&four_text).unwrap_or_else(|err| {
        eprintln!("ppt four 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let four_pdf = convert("ppt", "pdf", &four).unwrap_or_else(|err| {
        eprintln!("ppt four 8-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if four.len() != 27648 || four_pdf.len() != 3978 {
        eprintln!(
            "ppt four 8-bit picture size changed: input_b={} output_b={}",
            four.len(),
            four_pdf.len()
        );
        std::process::exit(1);
    }
    let nibble_text = ppt_nibble_text();
    let nibble = ppt_text_nibble_document(&nibble_text).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    let nibble_pdf = convert("ppt", "pdf", &nibble).unwrap_or_else(|err| {
        eprintln!("ppt 4-bit picture pdf changed: {err}");
        std::process::exit(1);
    });
    if nibble.len() != 9216 || nibble_pdf.len() != 2469 {
        eprintln!(
            "ppt 4-bit picture size changed: input_b={} output_b={}",
            nibble.len(),
            nibble_pdf.len()
        );
        std::process::exit(1);
    }
    let text = ppt_indexed_six_text();
    let file = ppt_text_six_indexed_document(&text).unwrap_or_else(|err| {
        eprintln!("ppt six 8-bit picture fixture failed: {err}");
        std::process::exit(1);
    });
    for banned in [
        2560, 5632, 6656, 7168, 7680, 8704, 9216, 9728, 14336, 14848, 21504, 27648, 33792,
    ] {
        if file.len() == banned {
            eprintln!(
                "ppt six 8-bit picture fixture collided with a published file: {}",
                file.len()
            );
            std::process::exit(1);
        }
    }
    let (width, height, first_rgb) = indexed_rgb_add(0);
    let (_, _, second_rgb) = indexed_rgb_add(1);
    let (_, _, third_rgb) = indexed_rgb_add(2);
    let (_, _, fourth_rgb) = indexed_rgb_add(3);
    let (_, _, fifth_rgb) = indexed_rgb_add(4);
    let (_, _, sixth_rgb) = indexed_rgb_add(5);
    match convert("ppt", "pdf", &file) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("ppt→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            let image = pdf.windows(8).any(|w| w == b"/Im1 Do ")
                && pdf.windows(8).any(|w| w == b"/Im2 Do ")
                && pdf.windows(8).any(|w| w == b"/Im3 Do ")
                && pdf.windows(8).any(|w| w == b"/Im4 Do ")
                && pdf.windows(8).any(|w| w == b"/Im5 Do ")
                && pdf.windows(8).any(|w| w == b"/Im6 Do ")
                && pdf.windows(9).any(|w| w == b"/Width 80")
                && pdf.windows(10).any(|w| w == b"/Height 64");
            let first_match = pdf_image_matches(&pdf, width, height, &first_rgb).unwrap_or(false);
            let second_match =
                pdf_image_matches(&pdf, width, height, &second_rgb).unwrap_or(false);
            let third_match = pdf_image_matches(&pdf, width, height, &third_rgb).unwrap_or(false);
            let fourth_match =
                pdf_image_matches(&pdf, width, height, &fourth_rgb).unwrap_or(false);
            let fifth_match = pdf_image_matches(&pdf, width, height, &fifth_rgb).unwrap_or(false);
            let sixth_match = pdf_image_matches(&pdf, width, height, &sixth_rgb).unwrap_or(false);
            let pixels_match = first_match
                && second_match
                && third_match
                && fourth_match
                && fifth_match
                && sixth_match
                && first_rgb != second_rgb
                && second_rgb != third_rgb
                && third_rgb != fourth_rgb
                && fourth_rgb != fifth_rgb
                && fifth_rgb != sixth_rgb;
            if extracted.contains(&text) && image && pixels_match {
                let sample = measure(|| ready_bytes("ppt", "pdf", &file));
                emit_ready("ppt→pdf indexed six", file.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "ppt→pdf indexed six p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={} pixels_match={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(file.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        file.len(),
                        sample.output_bytes,
                        pixels_match,
                    );
                }
            } else {
                eprintln!(
                    "six 8-bit pictures were not extracted into the pdf (pixels_match={pixels_match})"
                );
                emit_untimed(
                    "ppt→pdf indexed six",
                    file.len() as u64,
                    Some(pdf.len() as u64),
                    &format!(
                        "MISS: pdf text, picture, or pixels (input_b={} pixels_match={})",
                        file.len(),
                        pixels_match
                    ),
                );
            }
        }
        Err(err) => {
            eprintln!("six 8-bit pictures could not be extracted: {err}");
        }
    }
}

fn ppt_indexed_six_text() -> String {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "Sext indexed shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    text
}

fn ppt_two_indexed_text() -> (String, String) {
    let mut first = String::new();
    let mut second = String::new();
    for i in 0..12 {
        first.push_str(&format!(
            "First indexed shape sentence {i} of the generated PowerPoint file. "
        ));
        second.push_str(&format!(
            "Second indexed shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    (first, second)
}

fn nibble_rgb() -> (u32, u32, Vec<u8>) {
    let width = 80u32;
    let height = 160u32;
    let mut rgb = vec![0u8; (width * height * 3) as usize];
    for y in 0..height as usize {
        for x in 0..width as usize {
            let index = (x as u8).wrapping_add(y as u8) & 0x0F;
            let d = (y * width as usize + x) * 3;
            rgb[d] = index;
            rgb[d + 1] = index.wrapping_mul(3).wrapping_add(20);
            rgb[d + 2] = 180u8.wrapping_sub(index);
        }
    }
    (width, height, rgb)
}

fn ppt_nibble_text() -> String {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "Nibble picture sentence {i} of the generated PowerPoint file. "
        ));
    }
    text
}

fn ppt_indexed_text() -> String {
    let mut text = String::new();
    for i in 0..12 {
        text.push_str(&format!(
            "Indexed picture sentence {i} of the generated PowerPoint file. "
        ));
    }
    text
}

fn ppt_html_seven_texts() -> (String, String, String, String, String, String, String) {
    let (north, east, south, west, center, inner) = ppt_html_six_texts();
    let mut outer = String::new();
    for i in 0..25 {
        outer.push_str(&format!(
            "Outer shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    (north, east, south, west, center, inner, outer)
}

fn ppt_html_six_texts() -> (String, String, String, String, String, String) {
    let (north, east, south, west, center) = ppt_html_five_texts();
    let mut inner = String::new();
    for i in 0..25 {
        inner.push_str(&format!(
            "Inner shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    (north, east, south, west, center, inner)
}

fn ppt_html_five_texts() -> (String, String, String, String, String) {
    let (north, east, south, west) = ppt_html_four_texts();
    let mut center = String::new();
    for i in 0..25 {
        center.push_str(&format!(
            "Center shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    (north, east, south, west, center)
}

fn ppt_html_four_texts() -> (String, String, String, String) {
    let mut north = String::new();
    let mut east = String::new();
    let mut south = String::new();
    let mut west = String::new();
    for i in 0..25 {
        north.push_str(&format!(
            "North shape sentence {i} of the generated PowerPoint file. "
        ));
        east.push_str(&format!(
            "East shape sentence {i} of the generated PowerPoint file. "
        ));
        south.push_str(&format!(
            "South shape sentence {i} of the generated PowerPoint file. "
        ));
        west.push_str(&format!(
            "West shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    (north, east, south, west)
}

fn ppt_html_three_texts() -> (String, String, String) {
    let mut alpha = String::new();
    let mut beta = String::new();
    let mut gamma = String::new();
    for i in 0..30 {
        alpha.push_str(&format!(
            "Alpha shape sentence {i} of the generated PowerPoint file. "
        ));
        beta.push_str(&format!(
            "Beta shape sentence {i} of the generated PowerPoint file. "
        ));
        gamma.push_str(&format!(
            "Gamma shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    (alpha, beta, gamma)
}

fn ppt_html_two_texts() -> (String, String) {
    let mut first = String::new();
    let mut second = String::new();
    for i in 0..40 {
        first.push_str(&format!(
            "First shape sentence {i} of the generated PowerPoint file. "
        ));
        second.push_str(&format!(
            "Second shape sentence {i} of the generated PowerPoint file. "
        ));
    }
    (first, second)
}

fn ppt_html_text() -> String {
    let mut s = String::new();
    for i in 0..60 {
        s.push_str(&format!(
            "Sentence {i} of the generated PowerPoint text shape. "
        ));
    }
    s
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

fn doc_two_texts() -> (String, String) {
    let mut first = String::new();
    let mut second = String::new();
    for i in 0..200 {
        first.push_str(&format!("Alpha {i} of the first generated paragraph. "));
        second.push_str(&format!("Beta {i} of the second generated paragraph. "));
    }
    (first, second)
}

fn doc_two_paragraphs() -> Vec<u8> {
    let (first, second) = doc_two_texts();
    word97_document(&[&first, &second]).unwrap_or_else(|err| {
        eprintln!("word97 two-paragraph fixture failed: {err}");
        std::process::exit(1);
    })
}

fn doc_three_texts() -> (String, String, String) {
    let mut first = String::new();
    let mut second = String::new();
    let mut third = String::new();
    for i in 0..150 {
        first.push_str(&format!("Alpha {i} of the first generated paragraph. "));
        second.push_str(&format!("Beta {i} of the second generated paragraph. "));
        third.push_str(&format!("Gamma {i} of the third generated paragraph. "));
    }
    (first, second, third)
}

fn report_doc_pdf() {
    let text = doc_paragraph();
    let doc = word97_document(&[&text]).unwrap_or_else(|err| {
        eprintln!("word97 fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("doc", "pdf", &doc) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("doc→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            if !extracted.contains(&text) {
                emit_untimed(
                    "doc→pdf para",
                    doc.len() as u64,
                    Some(pdf.len() as u64),
                    "MISS: pdf text does not match",
                );
            } else {
                let sample = measure(|| ready_bytes("doc", "pdf", &doc));
                emit_ready("doc→pdf para", doc.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "doc→pdf para p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(doc.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        doc.len(),
                        sample.output_bytes,
                    );
                }
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", doc.len()))
            {
                eprintln!("doc→pdf hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("doc→pdf para", doc.len() as u64, None, &err.message);
        }
    }
    emit_encrypted_pdf_miss();
}

fn report_doc_pdf_two() {
    let (first, second) = doc_two_texts();
    let doc = word97_document(&[&first, &second]).unwrap_or_else(|err| {
        eprintln!("word97 two-paragraph fixture failed: {err}");
        std::process::exit(1);
    });
    match convert("doc", "pdf", &doc) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("doc→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            if !extracted.contains(&first) || !extracted.contains(&second) {
                emit_untimed(
                    "doc→pdf two",
                    doc.len() as u64,
                    Some(pdf.len() as u64),
                    "MISS: pdf text does not match",
                );
            } else {
                let sample = measure(|| ready_bytes("doc", "pdf", &doc));
                emit_ready("doc→pdf two", doc.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "doc→pdf two p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(doc.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        doc.len(),
                        sample.output_bytes,
                    );
                }
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", doc.len()))
            {
                eprintln!("doc→pdf hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("doc→pdf two", doc.len() as u64, None, &err.message);
        }
    }
    emit_encrypted_pdf_miss();
}

fn report_doc_pdf_three() {
    let (first, second, third) = doc_three_texts();
    let doc = word97_document(&[&first, &second, &third]).unwrap_or_else(|err| {
        eprintln!("word97 three-paragraph fixture failed: {err}");
        std::process::exit(1);
    });
    if doc.len() == 2560 {
        eprintln!("three-paragraph fixture is the 2560-byte file");
        std::process::exit(1);
    }
    match convert("doc", "pdf", &doc) {
        Ok(pdf) => {
            let extracted = pdf_text(&pdf).unwrap_or_else(|err| {
                eprintln!("doc→pdf text unreadable: {err}");
                std::process::exit(1);
            });
            if !extracted.contains(&first) || !extracted.contains(&second) || !extracted.contains(&third) {
                emit_untimed(
                    "doc→pdf three",
                    doc.len() as u64,
                    Some(pdf.len() as u64),
                    "MISS: pdf text does not match",
                );
            } else {
                let sample = measure(|| ready_bytes("doc", "pdf", &doc));
                emit_ready("doc→pdf three", doc.len() as u64, &sample);
                if !cfg!(debug_assertions) {
                    println!(
                        "doc→pdf three p50_us={} p99_us={} MB/s={:.2} allocs_p50={} alloc_b_p50={} input_b={} output_b={}",
                        sample.p50_ns / 1_000,
                        sample.p99_ns / 1_000,
                        mb_per_s(doc.len() as u64, sample.p50_ns),
                        sample.allocs_p50,
                        sample.alloc_bytes_p50,
                        doc.len(),
                        sample.output_bytes,
                    );
                }
            }
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", doc.len()))
            {
                eprintln!("doc→pdf hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("doc→pdf three", doc.len() as u64, None, &err.message);
        }
    }
    emit_encrypted_pdf_miss();
}

fn emit_encrypted_pdf_miss() {
    let enc = doc_encrypted();
    match convert("doc", "pdf", &enc) {
        Ok(_) => {
            eprintln!("encrypted doc produced pdf");
            std::process::exit(1);
        }
        Err(err) => {
            if !err.message.starts_with("MISS:") || !err.message.contains(&format!("input_b={}", enc.len()))
            {
                eprintln!("doc pdf miss hid the input size: {err}");
                std::process::exit(1);
            }
            emit_untimed("doc→pdf enc", enc.len() as u64, None, &err.message);
        }
    }
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
    if std::env::args().any(|arg| arg == "--pptx-pdf") {
        println!("export-docs pptx pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_pptx_pdf();
        return;
    }
    if std::env::args().any(|arg| arg == "--pptx-raster") {
        println!("export-docs pptx raster byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_pptx_raster();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-html") {
        println!("export-docs ppt html byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_html();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-html-two") {
        println!("export-docs ppt two-shape html byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_html_two();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-html-three") {
        println!("export-docs ppt three-shape html byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_html_three();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-html-four") {
        println!("export-docs ppt four-shape html byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_html_four();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-html-five") {
        println!("export-docs ppt five-shape html byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_html_five();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-html-six") {
        println!("export-docs ppt six-shape html byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_html_six();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-html-seven") {
        println!("export-docs ppt seven-shape html byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_html_seven();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-four") {
        println!("export-docs ppt four-shape pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_four();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-one") {
        println!("export-docs ppt one-shape pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_one();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic") {
        println!("export-docs ppt picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_picture();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic8") {
        println!("export-docs ppt 8-bit picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_indexed();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic4") {
        println!("export-docs ppt 4-bit picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_nibble();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic8-two") {
        println!("export-docs ppt two-shape 8-bit picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_indexed_two();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic8-images") {
        println!("export-docs ppt two 8-bit picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_indexed_images();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic8-three") {
        println!("export-docs ppt three 8-bit picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_indexed_three();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic8-four") {
        println!("export-docs ppt four 8-bit picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_indexed_four();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic8-five") {
        println!("export-docs ppt five 8-bit picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_indexed_five();
        return;
    }
    if std::env::args().any(|arg| arg == "--ppt-pdf-pic8-six") {
        println!("export-docs ppt six 8-bit picture pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_ppt_pdf_indexed_six();
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
    if std::env::args().any(|arg| arg == "--epub-two") {
        println!("export-docs epub two-item byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_epub_two();
        return;
    }
    if std::env::args().any(|arg| arg == "--epub-three") {
        println!("export-docs epub three-item byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_epub_three();
        return;
    }
    if std::env::args().any(|arg| arg == "--epub-four") {
        println!("export-docs epub four-item byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_epub_four();
        return;
    }
    if std::env::args().any(|arg| arg == "--epub-five") {
        println!("export-docs epub five-item byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_epub_five();
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
    if std::env::args().any(|arg| arg == "--doc-pdf") {
        println!("export-docs doc one-paragraph pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_doc_pdf();
        return;
    }
    if std::env::args().any(|arg| arg == "--doc-pdf-two") {
        println!("export-docs doc two-paragraph pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_doc_pdf_two();
        return;
    }
    if std::env::args().any(|arg| arg == "--doc-pdf-three") {
        println!("export-docs doc three-paragraph pdf byte counts");
        println!(
            "{:<32} {:>10} {:>10} {:>12} {:>12} {:>12} {:>10} {:>12} {:>12} status",
            "pair", "input_b", "output_b", "p50_us", "p99_us", "MB/s", "allocs_p50", "alloc_b_p50", "cold_us",
        );
        report_doc_pdf_three();
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
