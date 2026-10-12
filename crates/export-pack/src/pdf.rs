//! CBZ (a store zip of images) to a minimal multi-page PDF.
//! One image per page. The zip reader checks each payload CRC before the page is written.

use crate::error::{Error, Result};
use crate::png::{decode_png, is_jpeg, is_png, jpeg_size, RgbImage};
use crate::zip::ZipReader;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct PdfInfo {
    pub pages: usize,
    pub pixel_bytes: u64,
}

/// Read `cbz` with the zip reader and publish a PDF by renaming `pdf.tmp` onto `pdf`.
pub fn cbz_to_pdf(cbz: impl AsRef<Path>, pdf: impl AsRef<Path>) -> Result<PdfInfo> {
    let cbz = cbz.as_ref();
    let pdf = pdf.as_ref();
    let mut zip = ZipReader::open(cbz)?;
    let mut pages = Vec::new();
    for index in 0..zip.len() {
        let name = zip.entries()[index].name().to_owned();
        if name.ends_with('/') || name.split('/').next_back().unwrap_or("").starts_with('.') {
            continue;
        }
        if name.split('/').any(|part| part == "__MACOSX") {
            continue;
        }
        let size = zip.entries()[index].size();
        if size > crate::png::MAX_PIXELS * 4 + 1_048_576 {
            return Err(Error::Limit("image larger than the pixel cap"));
        }
        let mut bytes = Vec::new();
        zip.copy_entry(index, &mut bytes)?;
        if is_png(&bytes) {
            let image = decode_png(&bytes)?;
            let pixel_bytes = image.rgb.len() as u64;
            pages.push(Page::Rgb(image, pixel_bytes));
        } else if is_jpeg(&bytes) {
            let (width, height, gray) = jpeg_size(&bytes)?;
            let pixel_bytes = bytes.len() as u64;
            pages.push(Page::Jpeg {
                width,
                height,
                gray,
                bytes,
                pixel_bytes,
            });
        }
    }
    if pages.is_empty() {
        return Err(Error::Format("cbz contained no images"));
    }
    write_pdf(pdf, &pages)
}

enum Page {
    Rgb(RgbImage, u64),
    Jpeg {
        width: u32,
        height: u32,
        gray: bool,
        bytes: Vec<u8>,
        pixel_bytes: u64,
    },
}

impl Page {
    fn width(&self) -> u32 {
        match self {
            Page::Rgb(image, _) => image.width,
            Page::Jpeg { width, .. } => *width,
        }
    }
    fn height(&self) -> u32 {
        match self {
            Page::Rgb(image, _) => image.height,
            Page::Jpeg { height, .. } => *height,
        }
    }
    fn pixel_bytes(&self) -> u64 {
        match self {
            Page::Rgb(_, n) | Page::Jpeg { pixel_bytes: n, .. } => *n,
        }
    }
}

fn write_pdf(path: &Path, pages: &[Page]) -> Result<PdfInfo> {
    let tmp = sibling_tmp(path);
    let file = File::create(&tmp)?;
    let mut pdf = Pdf {
        file: Some(file),
        tmp_path: tmp,
        final_path: path.to_path_buf(),
        pos: 0,
        offsets: vec![0; 3 + pages.len() * 3],
        committed: false,
    };
    pdf.emit(b"%PDF-1.4\n")?;
    for (i, page) in pages.iter().enumerate() {
        let page_id = 3 + i * 3;
        let content_id = page_id + 1;
        let image_id = page_id + 2;
        pdf.object(
            page_id as u32,
            page_dict(page, content_id, image_id).as_bytes(),
        )?;
        let content = content_stream(page);
        let content_dict = format!("<< /Length {} >>", content.len());
        pdf.stream(content_id as u32, content_dict.as_bytes(), &content)?;
        match page {
            Page::Rgb(image, _) => {
                let dict = format!(
                    "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Length {} >>",
                    image.width,
                    image.height,
                    image.rgb.len()
                );
                pdf.stream(image_id as u32, dict.as_bytes(), &image.rgb)?;
            }
            Page::Jpeg {
                width,
                height,
                gray,
                bytes,
                ..
            } => {
                let space = if *gray { "/DeviceGray" } else { "/DeviceRGB" };
                let dict = format!(
                    "<< /Type /XObject /Subtype /Image /Width {width} /Height {height} /ColorSpace {space} /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>",
                    bytes.len()
                );
                pdf.stream(image_id as u32, dict.as_bytes(), bytes)?;
            }
        }
    }
    let kids: Vec<String> = (0..pages.len())
        .map(|i| format!("{} 0 R", 3 + i * 3))
        .collect();
    let pages_dict = format!(
        "<< /Type /Pages /Kids [{}] /Count {} >>",
        kids.join(" "),
        pages.len()
    );
    pdf.object(2, pages_dict.as_bytes())?;
    pdf.object(1, b"<< /Type /Catalog /Pages 2 0 R >>")?;
    let startxref = pdf.pos;
    let mut xref = format!("xref\n0 {}\n", pdf.offsets.len());
    xref.push_str("0000000000 65535 f \n");
    for offset in pdf.offsets.iter().skip(1) {
        xref.push_str(&format!("{offset:010} 00000 n \n"));
    }
    pdf.emit(xref.as_bytes())?;
    let trailer = format!(
        "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{startxref}\n%%EOF\n",
        pdf.offsets.len()
    );
    pdf.emit(trailer.as_bytes())?;
    let info = PdfInfo {
        pages: pages.len(),
        pixel_bytes: pages.iter().map(Page::pixel_bytes).sum(),
    };
    pdf.finish()?;
    Ok(info)
}

fn page_dict(page: &Page, content_id: usize, image_id: usize) -> String {
    format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {} {}] /Contents {content_id} 0 R /Resources << /XObject << /Im0 {image_id} 0 R >> >> >>",
        page.width(),
        page.height()
    )
}

fn content_stream(page: &Page) -> Vec<u8> {
    format!(
        "q\n{} 0 0 {} 0 0 cm\n/Im0 Do\nQ\n",
        page.width(),
        page.height()
    )
    .into_bytes()
}

struct Pdf {
    file: Option<File>,
    tmp_path: PathBuf,
    final_path: PathBuf,
    pos: u64,
    offsets: Vec<u64>,
    committed: bool,
}

impl Pdf {
    fn object(&mut self, id: u32, body: &[u8]) -> Result<()> {
        self.offsets[id as usize] = self.pos;
        let head = format!("{id} 0 obj\n");
        self.emit(head.as_bytes())?;
        self.emit(body)?;
        self.emit(b"\nendobj\n")
    }

    fn stream(&mut self, id: u32, dict: &[u8], data: &[u8]) -> Result<()> {
        self.offsets[id as usize] = self.pos;
        let head = format!("{id} 0 obj\n");
        self.emit(head.as_bytes())?;
        self.emit(dict)?;
        self.emit(b"\nstream\n")?;
        self.emit(data)?;
        self.emit(b"\nendstream\nendobj\n")
    }

    fn emit(&mut self, bytes: &[u8]) -> Result<()> {
        let file = self.file.as_mut().ok_or(Error::Format("pdf closed"))?;
        file.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        let file = self.file.take().ok_or(Error::Format("pdf closed"))?;
        file.sync_all()?;
        drop(file);
        fs::rename(&self.tmp_path, &self.final_path)?;
        sync_parent(&self.final_path);
        self.committed = true;
        Ok(())
    }
}

impl Drop for Pdf {
    fn drop(&mut self) {
        if self.committed {
            return;
        }
        self.file.take();
        let _ = fs::remove_file(&self.tmp_path);
    }
}

fn sibling_tmp(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".tmp");
    PathBuf::from(os)
}

fn sync_parent(path: &Path) {
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => Path::new("."),
    };
    if let Ok(dir) = File::open(parent) {
        let _ = dir.sync_all();
    }
}
