import os
import shutil
import subprocess
import sys
import tarfile
import uuid
import zipfile
from pathlib import Path
from urllib.parse import urlparse

import pdf2image
import pillow_heif
import py7zr
import yt_dlp
import zstandard as zstd
from gallery_dl import config as gdl_config, job as gdl_job
from PIL import Image, ImageOps

Image.MAX_IMAGE_PIXELS = 64_000_000

from .language import (
    default_whisper_model,
    detect_language_code,
    dialect_stt_dir,
    dialect_stt_model,
    is_offline,
    is_ph_dialect,
    is_whisper_english_only,
    language_name,
    osd_script_from_image,
    pick_piper_voice,
    pick_say_voice,
    pick_tess_lang,
    record_on_job,
    requested_language_code,
    requested_translate_to,
    use_language,
    use_translate_to,
    whisper_language_arg,
)
from .translate import maybe_translate_segment, maybe_translate_text
from .budget import pad_to_byte, predicted_byte, video_bit_rate

pillow_heif.register_heif_opener()

try:
    import pillow_jxl  # noqa: F401  # registers JPEG XL with Pillow on import
    _HAS_JXL = True
except ImportError:
    _HAS_JXL = False

try:
    import rawpy  # noqa: F401
    _HAS_RAW = True
except ImportError:
    _HAS_RAW = False

try:
    import cairosvg  # noqa: F401
    _HAS_SVG = True
except ImportError:
    _HAS_SVG = False

# Detect curl_cffi at import — yt-dlp uses it for browser impersonation,
# which is the only thing that gets past TikTok's datacenter-IP block.
try:
    import curl_cffi  # noqa: F401
    _HAS_IMPERSONATE = True
except ImportError:
    _HAS_IMPERSONATE = False

IMPERSONATE_HOST = ("tiktok.com", "instagram.com", "threads.net")
TRUTHY_VALUE = {"1", "true", "yes", "on", "always", "chrome"}
FALSY_VALUE = {"0", "false", "no", "off", "never", "none", "disable", "disabled"}

jobs: dict[str, dict] = {}


def mark_ready(job: dict, target: Path) -> None:
    """Job is downloadable. Mint a short-lived token (job TTL); never ?pw=."""
    job["filename"] = target.name
    job["output_size"] = target.stat().st_size
    job["token"] = uuid.uuid4().hex
    job["progress"] = 100
    job["status"] = "ready"


def _host_match(host: str, domain: str) -> bool:
    return host == domain or host.endswith(f".{domain}")


def _should_impersonate_url(url: str) -> bool:
    if not _HAS_IMPERSONATE:
        return False
    mode = os.environ.get("EXPORT_YTDLP_IMPERSONATE", "auto").strip().lower()
    if mode in TRUTHY_VALUE:
        return True
    if mode in FALSY_VALUE:
        return False
    host = (urlparse(url).hostname or "").lower()
    return any(_host_match(host, domain) for domain in IMPERSONATE_HOST)


def _humanize_extraction_error(err_text: str) -> str:
    """Translate yt-dlp/gallery-dl error spew into one-line actionable hints."""
    if not err_text or not err_text.strip():
        return ("Extraction failed silently. Most likely an IP block (try "
                "EXPORT_HTTP_PROXY) or a login-walled URL (try "
                "EXPORT_YTDLP_BROWSER=chrome).")
    e = err_text.lower()
    if "ip address is blocked" in e or "ip blocked" in e:
        return ("This platform blocks your IP. Set EXPORT_HTTP_PROXY to a "
                "residential/Tor proxy (e.g. socks5://127.0.0.1:9050).")
    if "no longer available" in e or "video has been removed" in e or "is deleted" in e or "track not found" in e:
        return "This content has been deleted by its uploader or platform."
    if "private video" in e or "members-only" in e or "members only" in e or "subscribers only" in e:
        return ("This content is private/members-only. Set EXPORT_YTDLP_COOKIES or "
                "EXPORT_YTDLP_BROWSER (chrome/firefox/safari) so we can use your login.")
    if ("log in" in e or "sign in" in e or "logged in" in e or "login required" in e
            or "api is not granting access" in e or "empty media response" in e):
        return ("Login required. Set EXPORT_YTDLP_BROWSER=chrome (or firefox/safari) "
                "so we can read cookies from your logged-in browser.")
    if "no video could be found" in e:
        return "No video on this page — check the URL points to a video post."
    if "geographically restricted" in e or "geo-restricted" in e or "not available in your country" in e:
        return ("Geo-restricted. Set EXPORT_HTTP_PROXY to a server in an allowed "
                "region.")
    # "not found" is too broad — was catching ffmpeg's "moov atom not found"
    # in shrink/convert paths. Match only phrases that genuinely scope to URL extraction.
    if "http error 404" in e or "page not found" in e or "video not found" in e:
        return "Page not found — the URL may be wrong, expired, or deleted."
    if "http error 403" in e or "forbidden" in e:
        return ("Access forbidden — try setting EXPORT_YTDLP_BROWSER=chrome (login) "
                "or EXPORT_HTTP_PROXY (IP block).")
    return err_text  # fall through with the original yt-dlp message

IMAGE_SAVE_OPTS = {
    "jpeg": {"quality": 90, "optimize": True, "progressive": True},
    "png":  {"optimize": True},
    "webp": {"quality": 90, "method": 6},
    "avif": {"quality": 70},
    "tiff": {"compression": "tiff_lzw"},
    "ico":  {"sizes": [(256, 256), (128, 128), (64, 64), (32, 32), (16, 16)]},
    "bmp":  {},
    "gif":  {"optimize": True},
    "icns": {"sizes": [(1024, 1024), (512, 512), (256, 256), (128, 128), (64, 64), (32, 32), (16, 16)]},
    "jp2":  {"quality_mode": "rates", "quality_layers": [10]},  # JPEG 2000
}
if _HAS_JXL:
    IMAGE_SAVE_OPTS["jxl"] = {"quality": 80}
IMAGE_EXT_OVERRIDE = {"jpeg": "jpg"}
IMAGE_FORMATS = set(IMAGE_SAVE_OPTS.keys())

# Bridge: image-as-input → pdf
IMAGE_TO_PDF = {"pdf"}

# Bridge: pdf-as-input → image formats + plain text
PDF_TO_IMAGE = {"png", "jpeg", "webp"}
PDF_TO_TEXT = {"txt"}

# Bridge: any pandoc-readable doc → pdf via WeasyPrint
DOC_TO_PDF = {"pdf"}

# Bridge: video → gif via ffmpeg palette
VIDEO_TO_GIF = {"gif"}

# Video → video format conversion (with optional quality knob).
# Codec defaults: mp4/mov/mkv → libx264 (universal playback); webm → libvp9.
VIDEO_TO_VIDEO = {"mp4", "mov", "webm", "mkv"}

# Pandoc-handled document formats. Pandoc auto-detects input by extension.
DOC_FORMATS = {"md", "html", "docx", "rtf", "epub", "txt", "org", "rst", "odt", "tex", "pptx"}
PANDOC_WRITER = {"md": "markdown", "txt": "plain", "pptx": "pptx"}  # most map 1:1 to extension
# Inputs Pandoc can read (broader than what we generate)
DOC_INPUT_SUFFIXES = {
    ".md", ".markdown", ".html", ".htm", ".rtf", ".epub", ".txt",
    ".tex", ".org", ".rst", ".odt", ".ipynb", ".adoc", ".asciidoc",
    ".textile", ".docbook", ".fb2", ".docx", ".pptx",
}

# Spreadsheets via openpyxl + stdlib csv/json. yaml/toml/xml ride the same
# flatten-to-table path so a config file can become a sheet / json / html.
SPREADSHEET_FORMATS = {"csv", "tsv", "xlsx", "json", "yaml", "toml"}
SPREADSHEET_SUFFIXES = {f".{f}" for f in SPREADSHEET_FORMATS} | {".yml", ".xml"}
# Spreadsheet inputs can also degrade to plain-text outputs — useful when the
# JSON is nested and you just want a readable dump rather than a rectangular sheet.
SPREADSHEET_TO_TEXT = {"txt", "md", "html"}
# Heuristic: when rendering JSON to HTML, string values under these keys are
# treated as raw HTML (rendered inline) rather than HTML-escaped. Useful for
# nested content like {"reviewer": {"subtopics": [{"html": "<p>..."}]}}.
_JSON_HTML_KEYS = {"html", "body", "content", "rendered", "markup"}

# Subtitles via pysubs2
SUBTITLE_FORMATS = {"srt", "vtt", "ass", "ssa", "sub", "sbv"}
SUBTITLE_SUFFIXES = {f".{f}" for f in SUBTITLE_FORMATS}
# Subtitle → readable transcript outputs. pysubs2 already loads the events;
# we just dump events[].text in different shapes (plain / markdown / NoteGPT).
SUBTITLE_TO_TEXT = {"txt", "md", "notes"}
# Subtitle → printable transcript via pandoc (loads our markdown intermediate).
SUBTITLE_TO_DOC = {"docx", "rtf", "epub", "odt", "tex", "html", "pdf"}

# Fonts via fonttools (TTF/OTF ↔ WOFF/WOFF2; TTF↔OTF is a no-op container swap)
FONT_FORMATS = {"ttf", "otf", "woff", "woff2"}
FONT_SUFFIXES = {f".{f}" for f in FONT_FORMATS}
# Font → specimen image (sample text rendered in the font). Useful when picking
# from a folder of fonts without opening each one in a viewer.
FONT_TO_IMAGE = {"png", "jpeg", "webp"}
# PDF → markdown / NoteGPT-style notes (paragraph-grouped from extracted text).
PDF_TO_MD = {"md"}
PDF_TO_NOTES = {"notes"}
# PDF → other doc formats via markdown + pandoc (ebook / print / web).
PDF_TO_DOC = {"html", "epub", "rtf", "odt", "tex"}
# Spreadsheet → paginated PDF via HTML + WeasyPrint.
SPREADSHEET_TO_PDF = {"pdf"}
# Comic archives (zip of images).
COMIC_SUFFIXES = {".cbz"}
COMIC_TO_PDF = {"pdf"}
# Audio/video "kit": one STT pass → zip of txt + srt + vtt + notes.
TRANSCRIPT_KIT = {"kit"}
# Any text-bearing input → translated plain text (uses translate_to, default en).
TRANSLATE_OUTPUT = {"translate"}
# Video → mp4 with burned-in captions (transcribe then ffmpeg subtitles filter).
VIDEO_CAPTIONED = {"captioned"}
# Video → zip of sampled PNG frames.
VIDEO_FRAMES = {"frames"}
# Video → extract embedded subs, or fall back to Whisper srt.
VIDEO_SUBS = {"subs"}
# image → editable doc via OCR + pandoc wrap (digitize a scanned page).
IMAGE_TO_DOC = {"docx", "md", "html", "rtf", "epub", "odt", "tex"}

ARCHIVE_FORMATS = {"zip", "7z", "tar.gz", "tar.zst", "tar.bz2", "tar.xz"}

# Single-file compression — wraps one file, no archive structure.
# Streams via stdlib gzip/bz2/lzma + zstandard.
SINGLE_FILE_COMPRESS = {"gz", "bz2", "xz", "zst"}

PDF_OPS = {"split", "rotate", "watermark", "encrypt", "decrypt"}

# Image: ML background removal (rembg) and raster→vector (vtracer)
IMAGE_TO_NOBG = {"nobg"}
IMAGE_TO_SVG = {"svg"}

# Audio AI: speech denoise (noisereduce) and music source separation (Demucs)
AUDIO_DENOISE = {"denoise"}
AUDIO_STEMS = {"stems"}
# audio → mp4 with animated waveform on a dark bg. Useful for podcast clip
# sharing on platforms that need a video (Twitter/IG/LinkedIn/etc).
AUDIO_TO_VIDEO = {"audiogram"}

SHRINK_LEVELS = {"low", "medium", "high", "resize"}
# Pillow-readable image inputs. PSD/TGA/PCX/EPS/ICNS rely on Pillow's broader
# format support; they get the same dispatch path as JPEG/PNG.
IMAGE_SUFFIXES = {
    ".jpg", ".jpeg", ".png", ".webp", ".avif", ".heic", ".heif",
    ".tiff", ".bmp", ".gif", ".ico", ".jp2",
    ".psd", ".tga", ".pcx", ".eps", ".icns",
}
if _HAS_JXL:
    IMAGE_SUFFIXES.add(".jxl")
if _HAS_SVG:
    IMAGE_SUFFIXES.add(".svg")
RAW_SUFFIXES = {".dng", ".cr2", ".cr3", ".nef", ".arw", ".raf", ".orf", ".rw2", ".pef", ".srw"}
if _HAS_RAW:
    IMAGE_SUFFIXES |= RAW_SUFFIXES
# .gif is also in IMAGE_SUFFIXES, but ffmpeg reads it fine as a video source —
# included here so gif → mp4/mov/webm/mkv works. The dispatch checks image
# bridges first (so gif → png is still an image op), and only falls through to
# the video branch when the target is a video format.
VIDEO_SUFFIXES = {".mp4", ".mov", ".webm", ".mkv", ".avi", ".m4v",
                  ".flv", ".wmv", ".3gp", ".ts", ".mts", ".m2ts", ".ogv", ".gif",
                  ".mpg", ".mpeg", ".m2v", ".vob", ".asf", ".mxf"}
AUDIO_SUFFIXES = {".mp3", ".m4a", ".wav", ".flac", ".ogg", ".opus", ".aac",
                  ".aiff", ".aif", ".wma", ".amr", ".3ga", ".m4b", ".ac3", ".caf"}
PDF_SUFFIXES = {".pdf"}
OFFICE_SUFFIXES = {".docx", ".doc", ".pptx", ".ppt", ".xlsx", ".xls", ".odp", ".ods", ".odt"}

# Audio transcoding (ffmpeg)
AUDIO_FORMATS = {"mp3", "m4a", "wav", "flac", "ogg", "opus", "aac", "m4b"}
AUDIO_FFMPEG_OPTS: dict[str, list[str]] = {
    "mp3":  ["-c:a", "libmp3lame", "-b:a", "192k"],
    "m4a":  ["-c:a", "aac", "-b:a", "192k"],
    "m4b":  ["-c:a", "aac", "-b:a", "64k"],  # audiobook: lower bitrate, .m4b container
    "wav":  ["-c:a", "pcm_s16le"],
    "flac": ["-c:a", "flac"],
    "ogg":  ["-c:a", "libvorbis", "-q:a", "5"],
    "opus": ["-c:a", "libopus", "-b:a", "128k"],
    "aac":  ["-c:a", "aac", "-b:a", "192k"],
}

# Office to PDF via LibreOffice headless
OFFICE_TO_PDF = {"pdf"}
# Office (pptx/docx/odp/etc.) → raster images: each page/slide rendered.
# Implemented as office→pdf→images (reuses LibreOffice + pdf2image),
# so the rasterization quality matches PDF_TO_IMAGE.
OFFICE_TO_IMAGE = {"png", "jpeg", "webp"}
# Office formats pandoc can read directly. For these, we route through
# _convert_doc (pandoc) for doc-format outputs — much higher fidelity than
# office→pdf→text. For other office formats (doc, ppt, xls/xlsx, ods, odp)
# pandoc doesn't have native readers, so we chain through pdf instead.
OFFICE_PANDOC_SOURCES = {".docx", ".odt", ".pptx"}
# Universal office → txt fallback via pdf. Works for ALL office formats,
# including the ones pandoc can't read. Composes _office_to_pdf + _pdf_to_text.
OFFICE_TO_TEXT = {"txt"}

# OCR: image → text via Tesseract
IMAGE_TO_TEXT = {"txt"}

# PDF → DOCX via pdf2docx (pure Python)
PDF_TO_DOCX = {"docx"}

# Text-to-speech: doc/text → audio.
#   macOS: native `say` (built-in)
#   Linux: piper-tts (set EXPORT_PIPER_VOICE to a .onnx voice path)
TTS_AUDIO_FORMATS = {"mp3", "m4a", "wav"}
TTS_INPUT_SUFFIXES = {".txt", ".md", ".html", ".htm", ".rtf"}

# PDF → searchable PDF/A via OCRmyPDF
PDF_OCR_OUTPUTS = {"pdfa"}

# Whisper STT: audio/video → transcript text/srt/vtt + NoteGPT-style markdown notes
# Default model: large-v3-turbo (multilingual). distil-large-v3 is English-only.
WHISPER_OUTPUTS = {"srt", "vtt", "notes"}
# Note: "txt" is handled here too when input is audio/video, but we don't add it
# to the set since the dispatch checks input type first; pdf→txt and image→txt
# stay routed to their own handlers.
# Word-level transcript: {stem}_words.json = { language, word: [{ text, start,
# end, confidence }] } from faster-whisper word_timestamps=True (or Groq's word
# granularity). For editors that time captions per spoken word. Never translated.
WHISPER_WORDS = {"words"}

_whisper_state: dict = {"model": None, "name": None}

VIDEO_FORMAT_PICK = "bestvideo[ext=mp4]+bestaudio[ext=m4a]/bestvideo+bestaudio/best[ext=mp4]/best"


def _pick_single_file(out_dir: Path, prefer_ext: str | None) -> Path | None:
    files = sorted(
        (p for p in out_dir.rglob("*") if p.is_file()),
        key=lambda p: p.stat().st_mtime,
        reverse=True,
    )
    if not files:
        return None
    if prefer_ext:
        for f in files:
            if f.suffix == f".{prefer_ext}":
                return f
    return files[0]


def _zip_images(out_dir: Path, title: str) -> Path:
    files = [p for p in out_dir.rglob("*") if p.is_file()]
    zip_path = out_dir / f"{title or 'images'}.zip"
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_STORED) as zf:
        for f in files:
            zf.write(f, arcname=f.name)
    # remove originals so _pick picks the zip
    for f in files:
        if f != zip_path:
            f.unlink(missing_ok=True)
    return zip_path


def _run_ytdlp(job: dict, url: str, fmt: str, out_dir: Path) -> str:
    def hook(d: dict) -> None:
        status = d.get("status")
        if status == "downloading":
            job["status"] = "downloading"
            total = d.get("total_bytes") or d.get("total_bytes_estimate") or 0
            done = d.get("downloaded_bytes", 0)
            job["progress"] = int(100 * done / total) if total else 0
        elif status == "finished":
            job["status"] = "converting"
            job["progress"] = 100

    opts: dict = {
        "outtmpl": str(out_dir / "%(title).200B.%(ext)s"),
        "quiet": True,
        "no_warnings": True,
        "noplaylist": True,
        "progress_hooks": [hook],
    }

    # Cookies for login-walled sites (Instagram, private FB, locked X tweets).
    cookies = os.environ.get("EXPORT_YTDLP_COOKIES")
    if cookies and Path(cookies).exists():
        opts["cookiefile"] = cookies

    # Or pull cookies live from a logged-in browser (much friendlier than exporting
    # cookies.txt). Accepts: chrome, firefox, safari, edge, brave, opera, vivaldi.
    browser = os.environ.get("EXPORT_YTDLP_BROWSER")
    if browser:
        opts["cookiesfrombrowser"] = (browser,)

    # HTTP/SOCKS proxy — only thing that beats hard IP blocks (TikTok, Rumble, geo).
    # Examples: http://user:pw@host:8000 / socks5://127.0.0.1:9050 (Tor) / http://192.168.1.5:3128
    proxy = os.environ.get("EXPORT_HTTP_PROXY")
    if proxy:
        opts["proxy"] = proxy

    # Chrome impersonation — needed for TikTok / some IG endpoints' TLS fingerprint check.
    # Keep it off for normal YouTube: curl_cffi has crashed during cleanup on
    # Python 3.14, and yt-dlp's default transport works for public YouTube URLs.
    # yt-dlp's Python API needs an ImpersonateTarget object, not a bare string.
    if _should_impersonate_url(url):
        try:
            from yt_dlp.networking.impersonate import ImpersonateTarget
            opts["impersonate"] = ImpersonateTarget(client="chrome")
        except Exception:
            pass  # silent degradation; CLI still works without impersonation for most sites

    if fmt == "mp3":
        opts["format"] = "bestaudio/best"
        opts["postprocessors"] = [{
            "key": "FFmpegExtractAudio",
            "preferredcodec": "mp3",
            "preferredquality": "192",
        }]
    elif fmt == "mp4":
        opts["format"] = VIDEO_FORMAT_PICK
        opts["merge_output_format"] = "mp4"
    else:
        raise ValueError(f"unsupported format for yt-dlp: {fmt}")

    with yt_dlp.YoutubeDL(opts) as ydl:
        info = ydl.extract_info(url, download=True)
    return info.get("title", "")


def _run_gallerydl(job: dict, url: str, out_dir: Path) -> str:
    job["status"] = "downloading"
    gdl_config.clear()
    gdl_config.set(("extractor",), "base-directory", str(out_dir))
    gdl_config.set(("extractor",), "directory", [])
    gdl_config.set(("extractor",), "filename", "{num}_{filename}.{extension}")

    cookies = os.environ.get("EXPORT_YTDLP_COOKIES")  # same file works for both tools
    if cookies and Path(cookies).exists():
        gdl_config.set(("extractor",), "cookies", cookies)

    browser = os.environ.get("EXPORT_YTDLP_BROWSER")
    if browser:
        gdl_config.set(("extractor",), "cookies-from-browser", browser)

    proxy = os.environ.get("EXPORT_HTTP_PROXY")
    if proxy:
        gdl_config.set(("extractor",), "proxy", proxy)

    dj = gdl_job.DownloadJob(url)
    rc = dj.run()
    if rc != 0:
        raise RuntimeError(f"gallery-dl exit code {rc}")

    job["status"] = "converting"
    job["progress"] = 100
    return Path(url.rstrip("/").split("/")[-1] or "gallery").stem


def _svg_to_pil(input_path: Path) -> Image.Image:
    if not _HAS_SVG:
        raise RuntimeError("SVG input needs cairosvg (pip install cairosvg)")
    import cairosvg
    from io import BytesIO
    buf = BytesIO()
    cairosvg.svg2png(url=str(input_path), write_to=buf, output_width=2048)
    buf.seek(0)
    img = Image.open(buf)
    img.load()
    return img.convert("RGBA")


def _raw_to_pil(input_path: Path) -> Image.Image:
    if not _HAS_RAW:
        raise RuntimeError("Camera RAW input needs rawpy (pip install rawpy) and libraw")
    import rawpy as rawpy_mod
    with rawpy_mod.imread(str(input_path)) as raw:
        rgb = raw.postprocess(use_camera_wb=True, no_auto_bright=False)
    return Image.fromarray(rgb)


def _open_raster(path: Path) -> Image.Image:
    """Open any image-like input (including SVG / camera RAW / JXL) as a PIL image.
    Applies EXIF orientation for camera JPEGs/HEICs."""
    suffix = path.suffix.lower()
    if suffix == ".svg":
        return _svg_to_pil(path)
    if suffix in RAW_SUFFIXES:
        return _raw_to_pil(path)
    img = Image.open(path)
    img = ImageOps.exif_transpose(img)
    img.load()
    return img


def _convert_image(input_path: Path, target_fmt: str, out_dir: Path, quality: int | None = None) -> Path:
    img = _open_raster(input_path)
    # Formats that can't carry alpha (or look bad with it): JPEG, ICO often
    # benefits from RGBA, TIFF is fine either way.
    if target_fmt == "jpeg" and img.mode in ("RGBA", "LA", "P"):
        bg = Image.new("RGB", img.size, (255, 255, 255))
        bg.paste(img.convert("RGBA"), mask=img.convert("RGBA").split()[3])
        img = bg
    elif target_fmt == "ico" and img.mode != "RGBA":
        img = img.convert("RGBA")
    ext = IMAGE_EXT_OVERRIDE.get(target_fmt, target_fmt)
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{ext}"
    save_opts = dict(IMAGE_SAVE_OPTS.get(target_fmt, {}))
    if quality is not None and "quality" in save_opts:
        save_opts["quality"] = max(1, min(100, quality))
    img.save(out_path, format=target_fmt.upper(), **save_opts)
    return out_path


def _html_page(title: str, body: str) -> str:
    """Wrap body in a minimal styled HTML page (no external CSS/JS)."""
    safe_title = title.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    return (
        "<!doctype html><html><head><meta charset=\"utf-8\">"
        f"<title>{safe_title}</title>"
        "<style>"
        "body{font:14px/1.55 -apple-system,system-ui,sans-serif;max-width:880px;margin:2rem auto;padding:0 1rem;color:#222;background:#fafafa}"
        "h1{font-size:1.4rem;margin-bottom:1.5rem;color:#000}"
        "dl{margin:0 0 0 0.75rem;border-left:2px solid #e5e5e5;padding-left:0.75rem}"
        "dt{font-weight:600;color:#555;margin-top:0.4rem}"
        "dd{margin:0 0 0.4rem 0.75rem}"
        "ol,ul{margin:0.25rem 0 0.5rem 1.25rem;padding:0}"
        "li{margin:0.15rem 0}"
        ".raw-html{border:1px solid #e0e0e0;border-radius:4px;padding:0.5rem 0.75rem;background:#fff;margin:0.25rem 0}"
        "table{border-collapse:collapse;width:100%;background:#fff}"
        "th,td{border:1px solid #ddd;padding:0.4rem 0.6rem;text-align:left;vertical-align:top}"
        "th{background:#f0f0f0}"
        ".scalar{color:#0a4}"
        ".key{color:#06c}"
        "</style></head><body>"
        f"<h1>{safe_title}</h1>{body}</body></html>"
    )


def _json_to_html(obj, key_hint: str | None = None) -> str:
    """Recursively render a JSON value as styled HTML.
    String values under keys in _JSON_HTML_KEYS are rendered raw (not escaped),
    which makes nested {"html": "<h2>..."} content actually display."""
    esc = lambda s: str(s).replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")
    if isinstance(obj, dict):
        if not obj:
            return "<em>(empty object)</em>"
        parts = ["<dl>"]
        for k, v in obj.items():
            parts.append(f"<dt class=\"key\">{esc(k)}</dt><dd>{_json_to_html(v, key_hint=str(k).lower())}</dd>")
        parts.append("</dl>")
        return "".join(parts)
    if isinstance(obj, list):
        if not obj:
            return "<em>(empty list)</em>"
        items = "".join(f"<li>{_json_to_html(x, key_hint=key_hint)}</li>" for x in obj)
        return f"<ol>{items}</ol>"
    if obj is None:
        return "<span class=\"scalar\">null</span>"
    if isinstance(obj, bool):
        return f"<span class=\"scalar\">{str(obj).lower()}</span>"
    if isinstance(obj, (int, float)):
        return f"<span class=\"scalar\">{obj}</span>"
    s = str(obj)
    if key_hint in _JSON_HTML_KEYS and "<" in s and ">" in s:
        # Field key signals raw HTML — render it inline so users see formatted content.
        return f"<div class=\"raw-html\">{s}</div>"
    return esc(s)


def _flatten_json_for_table(obj):
    """Coerce arbitrary JSON into rows: list[list[Any]] with header row.
    Tries (in order):
      1. list[dict] → keys as header, values as rows
      2. list[list] → raw rows
      3. list of scalars → one column
      4. dict with a nested list[dict] one level deep → use that
      5. dict (anything else) → 2-col [key, value] sheet (values JSON-stringified)
    Always returns rows; never raises."""
    import json as json_mod
    if isinstance(obj, list):
        if not obj:
            return [[]]
        if isinstance(obj[0], dict):
            keys = sorted({k for d in obj if isinstance(d, dict) for k in d.keys()},
                          key=lambda k: list(obj[0].keys()).index(k) if k in obj[0] else 1e9)
            return [keys] + [[d.get(k, "") if isinstance(d, dict) else "" for k in keys] for d in obj]
        if isinstance(obj[0], list):
            return obj
        return [[v] for v in obj]
    if isinstance(obj, dict):
        # Look one level deep for a nested list of dicts (e.g. {"reviewer": {"subtopics": [...]}}).
        def _walk(d, depth=0):
            if depth > 3:
                return None
            if isinstance(d, list) and d and isinstance(d[0], dict):
                return d
            if isinstance(d, dict):
                for v in d.values():
                    found = _walk(v, depth + 1)
                    if found:
                        return found
            return None
        nested = _walk(obj)
        if nested:
            return _flatten_json_for_table(nested)
        # Fallback: dump key/value as 2-column sheet, JSON-encode complex values.
        rows = [["key", "value"]]
        for k, v in obj.items():
            v_str = v if isinstance(v, (str, int, float, bool)) or v is None \
                    else json_mod.dumps(v, ensure_ascii=False)
            rows.append([k, v_str])
        return rows
    return [["value"], [obj]]


def _load_structured(input_path: Path):
    """Load json / yaml / toml / xml into a Python object (dict/list/scalar)."""
    import json as json_mod
    src = input_path.suffix.lower()
    text = input_path.read_text(encoding="utf-8")
    if src == ".json":
        return json_mod.loads(text)
    if src in {".yaml", ".yml"}:
        import yaml
        return yaml.safe_load(text)
    if src == ".toml":
        import tomllib
        return tomllib.loads(text)
    if src == ".xml":
        import xml.etree.ElementTree as ET
        root = ET.fromstring(text)

        def _elem(el):
            children = list(el)
            payload = {f"@{k}": v for k, v in el.attrib.items()}
            if children:
                grouped: dict[str, list] = {}
                for child in children:
                    grouped.setdefault(child.tag, []).append(_elem(child))
                for tag, items in grouped.items():
                    payload[tag] = items[0] if len(items) == 1 else items
            text_val = (el.text or "").strip()
            if text_val:
                payload["#text"] = text_val
            return payload or (el.text or "")
        return {root.tag: _elem(root)}
    raise RuntimeError(f"unsupported structured input: {src}")


def _convert_spreadsheet(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """Cross-convert tabular data: csv ↔ tsv ↔ xlsx ↔ json, plus txt/md dump.
    Reads first sheet of multi-sheet xlsx. Nested JSON is auto-tabularized:
    looks for an inner list[dict], falls back to a 2-col key/value sheet."""
    import csv as csv_mod
    import json as json_mod
    import openpyxl
    from datetime import date, datetime
    src = input_path.suffix.lower()
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"

    structured_src = src in {".json", ".yaml", ".yml", ".toml", ".xml"}

    # --- read input into rows: list[list[Any]] ---
    if src in (".csv", ".tsv"):
        delim = "\t" if src == ".tsv" else ","
        with input_path.open("r", encoding="utf-8-sig", newline="") as f:
            rows = list(csv_mod.reader(f, delimiter=delim))
    elif src == ".xlsx":
        wb = openpyxl.load_workbook(input_path, read_only=True, data_only=True)
        ws = wb.active
        rows = [[cell.value for cell in row] for row in ws.iter_rows()]
    elif structured_src:
        data = _load_structured(input_path)
        rows = _flatten_json_for_table(data)
    else:
        raise RuntimeError(f"unsupported spreadsheet input: {src}")

    # --- write output ---
    def _stringify(v):
        if isinstance(v, (date, datetime)):
            return v.isoformat()
        return "" if v is None else v

    if target_fmt in ("csv", "tsv"):
        delim = "\t" if target_fmt == "tsv" else ","
        with out_path.open("w", encoding="utf-8", newline="") as f:
            w = csv_mod.writer(f, delimiter=delim)
            w.writerows([[_stringify(c) for c in r] for r in rows])
    elif target_fmt == "xlsx":
        wb = openpyxl.Workbook()
        ws = wb.active
        for r in rows:
            ws.append([_stringify(c) for c in r])
        wb.save(out_path)
    elif target_fmt == "json":
        if structured_src:
            # Structured → JSON: preserve nesting (yaml/toml/xml/json).
            data = _load_structured(input_path)
        elif rows and len(rows) > 1:
            keys = [str(k) for k in rows[0]]
            data = [{k: _stringify(v) for k, v in zip(keys, r)} for r in rows[1:]]
        else:
            data = [list(r) for r in rows]
        out_path.write_text(json_mod.dumps(data, indent=2, default=str, ensure_ascii=False), encoding="utf-8")
    elif target_fmt in ("txt", "md"):
        # Plain-text dump. For JSON inputs we pretty-print the original (preserves
        # nested structure & is the most useful "read me as text" form). For
        # csv/tsv/xlsx we write a tab-aligned table.
        if structured_src:
            data = _load_structured(input_path)
            body = json_mod.dumps(data, indent=2, ensure_ascii=False)
            if target_fmt == "md":
                fence = "yaml" if src in {".yaml", ".yml"} else "json"
                body = f"```{fence}\n{body}\n```\n"
        else:
            lines = ["\t".join(str(_stringify(c)) for c in r) for r in rows]
            body = "\n".join(lines) + "\n"
            if target_fmt == "md":
                # Render as a real markdown table when there's a header + data.
                if len(rows) >= 2:
                    hdr = [str(_stringify(c)) for c in rows[0]]
                    sep = ["---"] * len(hdr)
                    md_rows = ["| " + " | ".join(hdr) + " |",
                               "| " + " | ".join(sep) + " |"]
                    for r in rows[1:]:
                        md_rows.append("| " + " | ".join(str(_stringify(c)).replace("|", "\\|") for c in r) + " |")
                    body = "\n".join(md_rows) + "\n"
        out_path.write_text(maybe_translate_text(body), encoding="utf-8")
    elif target_fmt == "html":
        # Render as a styled, readable HTML page. JSON walks the tree (with
        # _JSON_HTML_KEYS rendered raw); csv/tsv/xlsx render as a real <table>.
        if structured_src:
            data = _load_structured(input_path)
            body = _json_to_html(data)
        else:
            esc = lambda v: ("" if v is None else str(_stringify(v))).replace("&","&amp;").replace("<","&lt;").replace(">","&gt;")
            if rows and len(rows) >= 2:
                hdr = "".join(f"<th>{esc(c)}</th>" for c in rows[0])
                body_rows = "".join(
                    "<tr>" + "".join(f"<td>{esc(c)}</td>" for c in r) + "</tr>"
                    for r in rows[1:]
                )
                body = f"<table><thead><tr>{hdr}</tr></thead><tbody>{body_rows}</tbody></table>"
            else:
                body = "<p><em>(empty)</em></p>"
        title = stem.replace("_", " ")
        out_path.write_text(_html_page(title, body), encoding="utf-8")
    elif target_fmt == "yaml":
        import yaml
        if structured_src:
            data = _load_structured(input_path)
        elif rows and len(rows) > 1:
            keys = [str(k) for k in rows[0]]
            data = [{k: _stringify(v) for k, v in zip(keys, r)} for r in rows[1:]]
        else:
            data = [list(r) for r in rows]
        out_path.write_text(yaml.safe_dump(data, allow_unicode=True, sort_keys=False), encoding="utf-8")
    elif target_fmt == "toml":
        import tomli_w
        if structured_src:
            data = _load_structured(input_path)
        elif rows and len(rows) > 1:
            keys = [str(k) for k in rows[0]]
            data = {"row": [{k: _stringify(v) for k, v in zip(keys, r)} for r in rows[1:]]}
        else:
            data = {"row": [list(r) for r in rows]}
        if not isinstance(data, dict):
            data = {"item": data}
        out_path.write_text(tomli_w.dumps(data), encoding="utf-8")
    elif target_fmt == "pdf":
        html_path = _convert_spreadsheet(input_path, "html", out_dir)
        try:
            return _doc_to_pdf(html_path, out_dir)
        finally:
            html_path.unlink(missing_ok=True)
    else:
        raise RuntimeError(f"unsupported spreadsheet output: {target_fmt}")
    return out_path


def _translate_subtitle_event(subs) -> None:
    """In-place: translate each cue's text when translate_to is set."""
    if not requested_translate_to() or not subs.events:
        return
    from .translate import translate_batch
    sample = " ".join((e.plaintext or e.text or "") for e in subs.events[:8])
    source = detect_language_code(sample)
    target = requested_translate_to()
    if not target or source == target:
        return
    translated = translate_batch(
        [(e.plaintext or e.text or "") for e in subs.events],
        source, target,
    )
    for event, text in zip(subs.events, translated):
        event.text = text


def _convert_subtitle(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """Cross-convert subtitle formats: srt ↔ vtt ↔ ass ↔ ssa ↔ sub ↔ sbv."""
    import pysubs2
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"
    subs = pysubs2.load(str(input_path))
    _translate_subtitle_event(subs)
    subs.save(str(out_path), format_=target_fmt)
    return out_path


def _subtitle_to_text(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """Dump a subtitle file's text as plain/markdown/NoteGPT-style transcript.
    txt   → one line per event
    md    → '# title' + blank-line-separated events
    notes → paragraphs grouped by ≥1.5 s gap between adjacent events (mirrors
            the local Whisper notes formatter for consistency)"""
    import pysubs2
    stem = input_path.stem.removeprefix("input_")
    title = stem.replace("_", " ").replace("-", " ").strip()
    subs = pysubs2.load(str(input_path))
    events = sorted(subs.events, key=lambda e: e.start)
    _translate_subtitle_event(subs)
    lines = [e.plaintext or e.text for e in events]
    if target_fmt == "txt":
        out_path = out_dir / f"{stem}.txt"
        out_path.write_text("\n".join(lines).strip() + "\n", encoding="utf-8")
    elif target_fmt == "md":
        out_path = out_dir / f"{stem}.md"
        out_path.write_text(f"# {title}\n\n" + "\n\n".join(lines).strip() + "\n", encoding="utf-8")
    elif target_fmt == "notes":
        paragraphs: list[list[str]] = [[]]
        last_end = 0
        for e, line in zip(events, lines):
            # pysubs2 timestamps are in milliseconds; 1500 ms ≈ paragraph break.
            if e.start - last_end > 1500 and paragraphs[-1]:
                paragraphs.append([])
            paragraphs[-1].append(line.strip())
            last_end = e.end
        body = "\n\n".join(" ".join(p) for p in paragraphs if p)
        out_path = out_dir / f"{stem}_notes.md"
        out_path.write_text(f"# {title}\n\n## Transcript\n\n{body}\n", encoding="utf-8")
    else:
        raise ValueError(f"unsupported subtitle→text target: {target_fmt}")
    return out_path


def _subtitle_to_doc(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """Subtitle → printable transcript via the markdown intermediate.
    subtitle → md (our own dumper) → pandoc → {docx, rtf, epub, odt, tex, html, pdf}.
    Reuses _convert_doc + _doc_to_pdf so the output styling matches what
    a normal markdown document conversion would produce."""
    md_path = _subtitle_to_text(input_path, "md", out_dir)
    try:
        if target_fmt == "pdf":
            out_path = _doc_to_pdf(md_path, out_dir)
        else:
            out_path = _convert_doc(md_path, target_fmt, out_dir)
    finally:
        md_path.unlink(missing_ok=True)
    return out_path


def _convert_font(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """Convert between font containers: ttf/otf ↔ woff/woff2.
    Note: ttf↔otf is essentially a no-op (same container, different glyph
    table type). True ttf↔otf glyph re-encoding is a fontographer-tier
    operation we don't attempt."""
    from fontTools.ttLib import TTFont
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"
    font = TTFont(str(input_path))
    if target_fmt in ("woff", "woff2"):
        font.flavor = target_fmt
    else:
        # ttf or otf — strip any WOFF wrapping
        font.flavor = None
    font.save(str(out_path))
    return out_path


def _convert_doc(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"
    work = input_path
    tmp = None
    if requested_translate_to() and input_path.suffix.lower() in {".txt", ".md", ".markdown"}:
        translated = maybe_translate_text(
            input_path.read_text(encoding="utf-8", errors="replace")
        )
        tmp = out_dir / f"_translated{input_path.suffix.lower() or '.txt'}"
        tmp.write_text(translated, encoding="utf-8")
        work = tmp
    writer = PANDOC_WRITER.get(target_fmt, target_fmt)
    cmd = ["pandoc", str(work), "-o", str(out_path), "-t", writer]
    if target_fmt == "epub":
        cmd.extend(["--metadata", f"title={stem}"])
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
    if tmp:
        tmp.unlink(missing_ok=True)
    if r.returncode != 0:
        raise RuntimeError(f"pandoc failed: {r.stderr.strip()[:500]}")
    return out_path


def _make_archive(files: list[Path], fmt: str, out_dir: Path, base_name: str) -> Path:
    out_path = out_dir / f"{base_name}.{fmt}"
    if fmt == "zip":
        with zipfile.ZipFile(out_path, "w", zipfile.ZIP_DEFLATED, compresslevel=6) as zf:
            for f in files:
                zf.write(f, arcname=f.name.removeprefix("input_"))
    elif fmt == "7z":
        with py7zr.SevenZipFile(out_path, "w") as sz:
            for f in files:
                sz.write(f, arcname=f.name.removeprefix("input_"))
    elif fmt == "tar.gz":
        with tarfile.open(out_path, "w:gz", compresslevel=6) as tf:
            for f in files:
                tf.add(f, arcname=f.name.removeprefix("input_"))
    elif fmt == "tar.zst":
        cctx = zstd.ZstdCompressor(level=10, threads=-1)
        with out_path.open("wb") as f_out, cctx.stream_writer(f_out) as compressor:
            with tarfile.open(fileobj=compressor, mode="w|") as tf:
                for f in files:
                    tf.add(f, arcname=f.name.removeprefix("input_"))
    elif fmt == "tar.bz2":
        with tarfile.open(out_path, "w:bz2", compresslevel=9) as tf:
            for f in files:
                tf.add(f, arcname=f.name.removeprefix("input_"))
    elif fmt == "tar.xz":
        with tarfile.open(out_path, "w:xz", preset=6) as tf:
            for f in files:
                tf.add(f, arcname=f.name.removeprefix("input_"))
    else:
        raise ValueError(f"unsupported archive format: {fmt}")
    return out_path


def _compress_single(input_path: Path, fmt: str, out_dir: Path) -> Path:
    """Single-file compression (gz/bz2/xz/zst) — wraps one file, no archive."""
    import bz2
    import gzip
    import lzma
    base = input_path.name.removeprefix("input_")
    out_path = out_dir / f"{base}.{fmt}"
    if fmt == "gz":
        with input_path.open("rb") as f_in, gzip.open(out_path, "wb", compresslevel=6) as f_out:
            shutil.copyfileobj(f_in, f_out)
    elif fmt == "bz2":
        with input_path.open("rb") as f_in, bz2.open(out_path, "wb", compresslevel=9) as f_out:
            shutil.copyfileobj(f_in, f_out)
    elif fmt == "xz":
        with input_path.open("rb") as f_in, lzma.open(out_path, "wb", preset=6) as f_out:
            shutil.copyfileobj(f_in, f_out)
    elif fmt == "zst":
        cctx = zstd.ZstdCompressor(level=10, threads=-1)
        with input_path.open("rb") as f_in, out_path.open("wb") as f_out:
            cctx.copy_stream(f_in, f_out)
    else:
        raise ValueError(f"unsupported single-file format: {fmt}")
    return out_path


def _shrink_image(input_path: Path, level: str, out_dir: Path,
                  quality: int | None = None, max_dim: int | None = None,
                  scale_percent: float | None = None,
                  target_byte: int | None = None) -> Path:
    """Image shrink. Quality slider tunes JPEG/WebP/AVIF encoder quality.
    Optional resize via either max_dim (longest-side cap in px) OR
    scale_percent (e.g. 50 = half on both axes). scale_percent wins when both
    set. Level "resize" uses high quality and requires one of the two.
    `target_byte` lands on that exact file size (encode-under + pad). A source
    already at or under the cap is copied and never grown."""
    if level == "resize":
        if max_dim is None and scale_percent is None and not target_byte:
            raise RuntimeError("level=resize requires max_dim or scale_percent")
        quality = quality if quality is not None else 95
    elif quality is None:
        quality = {"low": 50, "medium": 70, "high": 85}[level]
    quality = max(1, min(100, quality))
    src_fmt = input_path.suffix.lower().lstrip(".") or "jpeg"
    img = _open_raster(input_path)
    src_fmt = {"jpg": "jpeg", "jxl": "jxl", "heic": "jpeg", "heif": "jpeg",
               "svg": "png", "dng": "jpeg", "cr2": "jpeg", "cr3": "jpeg",
               "nef": "jpeg", "arw": "jpeg", "raf": "jpeg"}.get(src_fmt, src_fmt)
    if scale_percent and scale_percent > 0 and scale_percent != 100:
        new_w = max(1, int(round(img.size[0] * scale_percent / 100)))
        new_h = max(1, int(round(img.size[1] * scale_percent / 100)))
        img = img.resize((new_w, new_h), Image.LANCZOS)
    elif max_dim and max(img.size) > max_dim:
        # thumbnail() is in-place, preserves aspect, uses LANCZOS by default.
        img.thumbnail((max_dim, max_dim))
    # PNG quality is meaningless; route lossless inputs through webp for actual savings.
    out_fmt = "webp" if src_fmt in ("png", "tiff", "bmp") else src_fmt
    if out_fmt not in ("jpeg", "webp", "avif"):
        out_fmt = "jpeg"
    ext = "jpg" if out_fmt == "jpeg" else out_fmt
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_small.{ext}"
    if out_fmt == "jpeg" and img.mode in ("RGBA", "LA", "P"):
        bg = Image.new("RGB", img.size, (255, 255, 255))
        bg.paste(img.convert("RGBA"), mask=img.convert("RGBA").split()[3])
        img = bg
    save_kwargs = {"quality": quality}
    if out_fmt == "jpeg":
        save_kwargs.update({"optimize": True, "progressive": True})
    elif out_fmt == "webp":
        save_kwargs["method"] = 6

    if target_byte and target_byte > 0:
        source_byte = input_path.stat().st_size
        if source_byte <= target_byte:
            dest = out_dir / f"{stem}_small{input_path.suffix.lower() or '.jpg'}"
            shutil.copy2(input_path, dest)
            return dest
        if img.mode in ("RGBA", "LA", "P"):
            bg = Image.new("RGB", img.size, (255, 255, 255))
            bg.paste(img.convert("RGBA"), mask=img.convert("RGBA").split()[3])
            img = bg
        elif img.mode != "RGB":
            img = img.convert("RGB")
        out_path = out_dir / f"{stem}_small.jpg"
        work = img
        best_q = 1
        for _ in range(6):
            lo, hi = 1, 95
            best_q = 1
            while lo <= hi:
                mid = (lo + hi) // 2
                work.save(out_path, format="JPEG", quality=mid, optimize=True, progressive=True)
                sz = out_path.stat().st_size
                if sz <= target_byte:
                    best_q = mid
                    lo = mid + 1
                else:
                    hi = mid - 1
            work.save(out_path, format="JPEG", quality=best_q, optimize=True, progressive=True)
            if out_path.stat().st_size <= target_byte:
                break
            nw = max(2, int(work.size[0] * 0.7))
            nh = max(2, int(work.size[1] * 0.7))
            work = work.resize((nw, nh), Image.LANCZOS)
        if out_path.stat().st_size > target_byte:
            raise RuntimeError(
                f"cannot fit image under {target_byte} bytes (landed {out_path.stat().st_size})"
            )
        if out_path.stat().st_size < target_byte:
            pad_to_byte(out_path, target_byte)
        return out_path

    img.save(out_path, format=out_fmt.upper(), **save_kwargs)
    return out_path


def _video_scale_filter(input_path: Path, max_dim: int | None = None,
                        scale_percent: float | None = None) -> str | None:
    """Build an ffmpeg `-vf scale=...` filter. Either max_dim (longest-side
    cap in px) or scale_percent (relative percentage) drives the target. When
    both are given, scale_percent wins. Probes the source via ffprobe so a
    concrete W:H can be passed (avoids ffmpeg expression escaping). Output
    dimensions are rounded to multiples of 2 (x264/SVT-AV1 requirement).
    Returns None when no scaling is needed."""
    if not max_dim and not scale_percent:
        return None
    r = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=width,height",
         "-of", "csv=p=0:s=x", str(input_path)],
        capture_output=True, text=True, timeout=10,
    )
    if r.returncode != 0 or not r.stdout.strip():
        # Probe failed — fall back to a percent-naive max_dim filter if we have one.
        if max_dim:
            return (f"scale={max_dim}:{max_dim}:force_original_aspect_ratio=decrease,"
                    f"scale=trunc(iw/2)*2:trunc(ih/2)*2")
        return None
    try:
        w_str, h_str = r.stdout.strip().split("x", 1)
        w, h = int(w_str), int(h_str)
    except ValueError:
        return None
    if scale_percent and scale_percent > 0 and scale_percent != 100:
        scale_factor = scale_percent / 100
    elif max_dim:
        longest = max(w, h)
        if longest <= max_dim:
            return None  # source already within cap
        scale_factor = max_dim / longest
    else:
        return None
    new_w = max(2, int(round(w * scale_factor / 2)) * 2)
    new_h = max(2, int(round(h * scale_factor / 2)) * 2)
    if new_w == w and new_h == h:
        return None
    return f"scale={new_w}:{new_h}:flags=lanczos"


def _probe_duration_second(input_path: Path) -> float:
    r = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration",
         "-of", "csv=p=0", str(input_path)],
        capture_output=True, text=True, timeout=15,
    )
    try:
        return max(float((r.stdout or "0").strip() or 0), 0.05)
    except ValueError:
        return 1.0


def _probe_has_audio(input_path: Path) -> bool:
    r = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "a:0",
         "-show_entries", "stream=codec_type", "-of", "csv=p=0", str(input_path)],
        capture_output=True, text=True, timeout=15,
    )
    return bool((r.stdout or "").strip())


def _shrink_video(input_path: Path, level: str, out_dir: Path,
                  quality: int | None = None, max_dim: int | None = None,
                  scale_percent: float | None = None,
                  target_byte: int | None = None) -> Path:
    """Video shrink. Default codec: libx264 (h.264) for universal playback —
    QuickTime, Preview, every browser, every messaging app, every NLE. About
    20-30% larger than SVT-AV1 at matched quality, but AV1-in-MP4 breaks on
    plenty of consumer players (Preview seeks, Discord embeds, older Safari),
    which is the failure pattern users actually hit.

    Set EXPORT_SHRINK_VIDEO_CODEC=av1 for the maximum-shrink path when you
    know the playback chain supports AV1 (recent Chrome/Firefox, VLC, mpv).

    Either max_dim (longest-side cap in px) or scale_percent (relative %)
    controls the downscale; scale_percent wins when both are set. Resolution
    downscale is the single biggest compression lever — a 4K → 1080p downscale
    alone gives ~4× smaller file at matched CRF. Level "resize" uses high-
    quality CRF and requires one of the two."""
    codec = os.environ.get("EXPORT_SHRINK_VIDEO_CODEC", "x264").lower()
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_small.mp4"

    if target_byte and target_byte > 0:
        source_byte = input_path.stat().st_size
        if source_byte <= target_byte:
            dest = out_dir / f"{stem}_small{input_path.suffix.lower() or '.mp4'}"
            shutil.copy2(input_path, dest)
            return dest
        duration_second = _probe_duration_second(input_path)
        has_audio = _probe_has_audio(input_path)
        audio_bit_rate = 96_000 if has_audio else 0
        scale_filter = _video_scale_filter(input_path, max_dim=max_dim, scale_percent=scale_percent) \
            if (max_dim or scale_percent) else None
        bit_rate = video_bit_rate(target_byte, duration_second, audio_bit_rate)
        last_err = ""
        for _ in range(6):
            vf_args = ["-vf", scale_filter] if scale_filter else []
            audio_args = ["-c:a", "aac", "-b:a", "96k"] if has_audio else ["-an"]
            cmd = [
                "ffmpeg", "-y", "-i", str(input_path),
                *vf_args,
                "-c:v", "libx264",
                "-b:v", str(bit_rate), "-maxrate", str(bit_rate),
                "-bufsize", str(max(bit_rate * 2, 16_000)),
                "-profile:v", "high", "-level:v", "4.0", "-pix_fmt", "yuv420p",
                "-preset", "fast",
                *audio_args,
                "-movflags", "+faststart", str(out_path),
            ]
            r = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
            if r.returncode != 0:
                last_err = r.stderr.strip()[-400:]
                bit_rate = max(8_000, int(bit_rate * 0.7))
                continue
            sz = out_path.stat().st_size
            if sz == target_byte:
                return out_path
            if sz < target_byte and (target_byte - sz) >= 8:
                pad_to_byte(out_path, target_byte)
                return out_path
            # Over cap, or under by <8 (can't write a free box): squeeze more.
            bit_rate = max(8_000, int(bit_rate * 0.7))
            if scale_filter is None:
                scale_filter = "scale=trunc(iw*3/8)*2:trunc(ih*3/8)*2"
            else:
                scale_filter = "scale=trunc(iw*0.7/2)*2:trunc(ih*0.7/2)*2"
        if out_path.exists() and out_path.stat().st_size <= target_byte:
            if out_path.stat().st_size < target_byte:
                pad_to_byte(out_path, target_byte)
            return out_path
        raise RuntimeError(
            f"cannot fit video under {target_byte} bytes"
            + (f": {last_err}" if last_err else "")
        )

    # Pure-resize mode: keep visual quality high, only scale dimensions.
    if level == "resize":
        if max_dim is None and scale_percent is None and not target_byte:
            raise RuntimeError("level=resize requires max_dim or scale_percent")
        quality = quality if quality is not None else 95
        level = "high"  # fall through to high-quality CRF mapping below

    scale_filter = _video_scale_filter(input_path, max_dim=max_dim, scale_percent=scale_percent) \
        if (max_dim or scale_percent) else None

    # Build the optional -vf scale=... arg pair once (empty list = no scaling).
    vf_args = ["-vf", scale_filter] if scale_filter else []

    if codec == "av1":
        # SVT-AV1: CRF 0-63 (lower = better). preset 0=slow/best, 13=fast/worst.
        if quality is not None:
            q = max(1, min(100, quality))
            crf = str(int(round(60 - (q / 100) * 42)))
            settings = {"crf": crf, "preset": "6"}
        else:
            settings = {"low":    {"crf": "45", "preset": "8"},
                        "medium": {"crf": "35", "preset": "6"},
                        "high":   {"crf": "28", "preset": "4"}}[level]
        cmd = ["ffmpeg", "-y", "-i", str(input_path),
               *vf_args,
               "-c:v", "libsvtav1", "-crf", settings["crf"], "-preset", settings["preset"],
               "-pix_fmt", "yuv420p",
               "-c:a", "aac", "-b:a", "96k",
               "-movflags", "+faststart", str(out_path)]
    else:
        # libx264: CRF 0-51 (18-28 is the typical range; 23 default). yuv420p
        # + high@4.0 profile + faststart = plays everywhere including iOS
        # Safari and old Smart TVs. Preset balances size vs encode time.
        if quality is not None:
            q = max(1, min(100, quality))
            # quality 100 → CRF 18 (visually lossless), quality 1 → CRF 35.
            crf = str(int(round(35 - (q / 100) * 17)))
            settings = {"crf": crf, "preset": "medium"}
        else:
            settings = {"low":    {"crf": "30", "preset": "fast"},
                        "medium": {"crf": "26", "preset": "medium"},
                        "high":   {"crf": "22", "preset": "slow"}}[level]
        cmd = ["ffmpeg", "-y", "-i", str(input_path),
               *vf_args,
               "-c:v", "libx264", "-crf", settings["crf"], "-preset", settings["preset"],
               "-profile:v", "high", "-level:v", "4.0", "-pix_fmt", "yuv420p",
               "-c:a", "aac", "-b:a", "96k", "-movflags", "+faststart", str(out_path)]

    r = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg failed: {r.stderr.strip()[-500:]}")
    return out_path


def _shrink_pdf(input_path: Path, level: str, out_dir: Path, quality: int | None = None,
                target_byte: int | None = None) -> Path:
    # Ghostscript only has 4 quality presets. Map quality slider to nearest preset.
    if quality is not None:
        q = max(1, min(100, quality))
        if q < 35:   setting = "/screen"
        elif q < 65: setting = "/ebook"
        elif q < 85: setting = "/printer"
        else:        setting = "/prepress"
    else:
        setting = {"low": "/screen", "medium": "/ebook", "high": "/printer"}[level]
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_small.pdf"
    cmd = [
        "gs", "-sDEVICE=pdfwrite", "-dCompatibilityLevel=1.4",
        f"-dPDFSETTINGS={setting}",
        "-dNOPAUSE", "-dQUIET", "-dBATCH",
        f"-sOutputFile={out_path}",
        str(input_path),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if r.returncode != 0:
        raise RuntimeError(f"ghostscript failed: {r.stderr.strip()[-500:]}")
    if target_byte and target_byte > 0:
        source_byte = input_path.stat().st_size
        if source_byte <= target_byte:
            dest = out_dir / f"{stem}_small.pdf"
            if dest != out_path:
                shutil.copy2(input_path, dest)
                out_path.unlink(missing_ok=True)
                return dest
            return out_path
        if out_path.stat().st_size > target_byte:
            raise RuntimeError(
                f"cannot fit pdf under {target_byte} bytes (landed {out_path.stat().st_size})"
            )
        if out_path.stat().st_size < target_byte:
            pad_to_byte(out_path, target_byte)
    return out_path


def run_shrink_job(job_id: str, input_path: Path, level: str, downloads_dir: Path,
                   quality: int | None = None, max_dim: int | None = None,
                   scale_percent: float | None = None,
                   target_byte: int | None = None) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "shrinking"
        source_byte = input_path.stat().st_size
        job["predicted_byte"] = predicted_byte(source_byte, target_byte)
        suffix = input_path.suffix.lower()
        if suffix in IMAGE_SUFFIXES:
            target = _shrink_image(input_path, level, out_dir, quality=quality,
                                   max_dim=max_dim, scale_percent=scale_percent,
                                   target_byte=target_byte)
        elif suffix in VIDEO_SUFFIXES:
            target = _shrink_video(input_path, level, out_dir, quality=quality,
                                   max_dim=max_dim, scale_percent=scale_percent,
                                   target_byte=target_byte)
        elif suffix in PDF_SUFFIXES:
            # PDF doesn't currently honor max_dim/scale_percent (would need
            # Ghostscript -dColorImageResolution tuning, separate concern).
            target = _shrink_pdf(input_path, level, out_dir, quality=quality,
                                 target_byte=target_byte)
        else:
            raise ValueError(f"unsupported file type for shrink: {suffix or '(no extension)'}")
        input_path.unlink(missing_ok=True)
        mark_ready(job, target)
        if job.get("predicted_byte") is not None:
            job["predicted_byte"] = job["output_size"]
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)


def run_compress_job(job_id: str, input_paths: list[Path], fmt: str, downloads_dir: Path) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "compressing"
        if not input_paths:
            raise RuntimeError("no input files")
        if fmt in SINGLE_FILE_COMPRESS:
            if len(input_paths) != 1:
                raise ValueError(
                    f".{fmt} compresses one file at a time. "
                    f"For multiple files use tar.{fmt} or zip / 7z."
                )
            target = _compress_single(input_paths[0], fmt, out_dir)
        else:
            base_name = (input_paths[0].stem.removeprefix("input_")
                         if len(input_paths) == 1 else "archive")
            target = _make_archive(input_paths, fmt, out_dir, base_name)
        for p in input_paths:
            p.unlink(missing_ok=True)
        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)


BATCH_IMAGE_FORMATS = {"pdf", "jpeg", "png", "webp", "avif", "gif", "mp4"}


def _open_oriented(path: Path, rotations: dict[str, int] | None = None) -> Image.Image:
    """Open image, apply EXIF orientation, then user-applied rotation."""
    img = _open_raster(path)
    if rotations:
        original_name = path.name.removeprefix("input_")
        deg = int(rotations.get(original_name, 0)) % 360
        if deg:
            img = img.rotate(-deg, expand=True)  # PIL rotates CCW; negate for CW
    return img


def _batch_image_convert(input_paths: list[Path], target_fmt: str, out_dir: Path,
                          rotations: dict[str, int] | None = None) -> Path:
    """Convert each input image to `target_fmt` (jpeg/png/webp/avif), zip the
    results. Honors EXIF + user rotations."""
    converted_dir = out_dir / "_batch"
    converted_dir.mkdir(exist_ok=True)
    failures = []
    for p in input_paths:
        try:
            img = _open_oriented(p, rotations)
            ext = IMAGE_EXT_OVERRIDE.get(target_fmt, target_fmt)
            stem = p.stem.removeprefix("input_")
            out_one = converted_dir / f"{stem}.{ext}"
            if target_fmt == "jpeg" and img.mode in ("RGBA", "LA", "P"):
                bg = Image.new("RGB", img.size, (255, 255, 255))
                bg.paste(img.convert("RGBA"), mask=img.convert("RGBA").split()[3])
                img = bg
            img.save(out_one, format=target_fmt.upper(), **IMAGE_SAVE_OPTS.get(target_fmt, {}))
        except Exception as e:
            failures.append(f"{p.name}: {e}")
    if failures and not any(converted_dir.iterdir()):
        shutil.rmtree(converted_dir, ignore_errors=True)
        raise RuntimeError("all images failed: " + "; ".join(failures[:3]))
    out_path = out_dir / f"images_{target_fmt}.zip"
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_STORED) as zf:
        for f in sorted(converted_dir.iterdir()):
            zf.write(f, arcname=f.name)
    shutil.rmtree(converted_dir, ignore_errors=True)
    return out_path


def _images_to_animated_gif(input_paths: list[Path], out_dir: Path,
                             rotations: dict[str, int] | None = None,
                             frame_seconds: float = 2.0) -> Path:
    """Combine N images into a single animated GIF (slideshow). Frames are
    capped to 800px max edge to keep file sizes sane."""
    if not input_paths:
        raise RuntimeError("no images")
    frames: list[Image.Image] = []
    for p in input_paths:
        img = _open_oriented(p, rotations)
        img.thumbnail((800, 800), Image.Resampling.LANCZOS)
        if img.mode != "RGB":
            img = img.convert("RGB")
        frames.append(img)
    base = input_paths[0].stem.removeprefix("input_")
    out_path = out_dir / (f"{base}.gif" if len(input_paths) == 1
                          else f"{base}_and_{len(input_paths) - 1}_more.gif")
    frames[0].save(out_path, save_all=True, append_images=frames[1:],
                   duration=int(frame_seconds * 1000), loop=0, optimize=True)
    return out_path


def _images_to_video(input_paths: list[Path], out_dir: Path,
                      rotations: dict[str, int] | None = None,
                      frame_seconds: float = 3.0) -> Path:
    """Combine N images into an MP4 slideshow. Each image normalized to RGB
    JPEG first (for ffmpeg input), then concatenated at one frame per
    `frame_seconds` seconds. Cap width at 1920."""
    if not input_paths:
        raise RuntimeError("no images")
    work = out_dir / "_slideshow"
    work.mkdir(exist_ok=True)
    for i, p in enumerate(input_paths):
        img = _open_oriented(p, rotations)
        if img.mode != "RGB":
            img = img.convert("RGB")
        img.save(work / f"frame_{i:04d}.jpg", quality=88)
    base = input_paths[0].stem.removeprefix("input_")
    out_path = out_dir / (f"{base}.mp4" if len(input_paths) == 1
                          else f"{base}_and_{len(input_paths) - 1}_more.mp4")
    cmd = [
        "ffmpeg", "-y",
        "-framerate", f"{1.0 / frame_seconds:.4f}",
        "-pattern_type", "glob", "-i", str(work / "frame_*.jpg"),
        "-c:v", "libx264", "-pix_fmt", "yuv420p",
        "-vf", "scale='min(1920,iw)':-2,format=yuv420p",
        "-movflags", "+faststart",
        str(out_path),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    shutil.rmtree(work, ignore_errors=True)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg slideshow failed: {r.stderr.strip()[-500:]}")
    return out_path


def run_image_batch_job(job_id: str, input_paths: list[Path], fmt: str,
                          downloads_dir: Path,
                          rotations: dict[str, int] | None = None) -> None:
    """Dispatch batch image conversion based on output format."""
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "converting"
        if fmt == "pdf":
            target = _images_to_pdf(input_paths, out_dir, rotations)
        elif fmt in {"jpeg", "png", "webp", "avif"}:
            target = _batch_image_convert(input_paths, fmt, out_dir, rotations)
        elif fmt == "gif":
            target = _images_to_animated_gif(input_paths, out_dir, rotations)
        elif fmt == "mp4":
            target = _images_to_video(input_paths, out_dir, rotations)
        else:
            raise ValueError(f"unsupported batch format: {fmt}")
        for p in input_paths:
            p.unlink(missing_ok=True)
        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = str(e)


def _images_to_pdf(input_paths: list[Path], out_dir: Path,
                   rotations: dict[str, int] | None = None) -> Path:
    """Combine N images into a single multi-page PDF. EXIF-rotated first
    (camera orientation), then user-rotated per `rotations` (degrees CW
    keyed by original filename, e.g. {"IMG_1234.HEIC": 90})."""
    if not input_paths:
        raise RuntimeError("no images to combine")
    rotations = rotations or {}
    pages: list[Image.Image] = []
    for p in input_paths:
        img = _open_oriented(p, rotations)
        if img.mode in ("RGBA", "LA", "P"):
            bg = Image.new("RGB", img.size, (255, 255, 255))
            bg.paste(img.convert("RGBA"), mask=img.convert("RGBA").split()[3])
            img = bg
        elif img.mode != "RGB":
            img = img.convert("RGB")
        pages.append(img)
    base = input_paths[0].stem.removeprefix("input_")
    out_path = out_dir / (f"{base}.pdf" if len(input_paths) == 1
                          else f"{base}_and_{len(input_paths) - 1}_more.pdf")
    pages[0].save(out_path, "PDF", resolution=150,
                  save_all=True, append_images=pages[1:])
    return out_path


def run_images_to_pdf_job(job_id: str, input_paths: list[Path],
                          downloads_dir: Path,
                          rotations: dict[str, int] | None = None) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "converting"
        target = _images_to_pdf(input_paths, out_dir, rotations)
        for p in input_paths:
            p.unlink(missing_ok=True)
        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = str(e)


def _image_to_pdf(input_path: Path, out_dir: Path) -> Path:
    img = _open_raster(input_path)
    if img.mode in ("RGBA", "LA", "P"):
        bg = Image.new("RGB", img.size, (255, 255, 255))
        bg.paste(img.convert("RGBA"), mask=img.convert("RGBA").split()[3])
        img = bg
    elif img.mode != "RGB":
        img = img.convert("RGB")
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.pdf"
    img.save(out_path, format="PDF", resolution=300)
    return out_path


def _pdf_to_images(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    stem = input_path.stem.removeprefix("input_")
    pages = pdf2image.convert_from_path(str(input_path), dpi=200)
    if not pages:
        raise RuntimeError("pdf has no pages")
    ext = IMAGE_EXT_OVERRIDE.get(target_fmt, target_fmt)
    save_kwargs = {"jpeg": {"quality": 92}, "webp": {"quality": 90}}.get(target_fmt, {})
    if len(pages) == 1:
        out_path = out_dir / f"{stem}.{ext}"
        pages[0].save(out_path, format=target_fmt.upper(), **save_kwargs)
        return out_path
    # Multi-page: save each then zip
    page_dir = out_dir / "_pages"
    page_dir.mkdir(exist_ok=True)
    for i, pg in enumerate(pages, 1):
        pg.save(page_dir / f"{stem}_p{i:03d}.{ext}", format=target_fmt.upper(), **save_kwargs)
    out_path = out_dir / f"{stem}.zip"
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_STORED) as zf:
        for f in sorted(page_dir.iterdir()):
            zf.write(f, arcname=f.name)
    shutil.rmtree(page_dir, ignore_errors=True)
    return out_path


def _convert_video(input_path: Path, target_fmt: str, out_dir: Path, quality: int | None = None) -> Path:
    """Re-mux/transcode video into a different container. quality is the same
    1-100 scale used by the advanced slider — gets mapped to per-codec CRF."""
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"

    if target_fmt == "webm":
        vcodec, acodec = "libvpx-vp9", "libopus"
        # vp9 CRF 0-63; q=100 → 18 (visually lossless), q=1 → 60.
        crf = str(int(round(60 - (quality / 100) * 42))) if quality is not None else "30"
    else:
        # mp4 / mov / mkv: x264 + aac for universal device playback.
        vcodec, acodec = "libx264", "aac"
        # x264 CRF 0-51; q=100 → 18, q=1 → 35.
        crf = str(int(round(35 - (quality / 100) * 17))) if quality is not None else "23"

    cmd = [
        "ffmpeg", "-y", "-i", str(input_path),
        "-c:v", vcodec, "-crf", crf, "-preset", "medium",
        "-c:a", acodec, "-b:a", "128k",
        "-movflags", "+faststart",
        str(out_path),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=1800)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg failed: {r.stderr.strip()[-500:]}")
    return out_path


def _video_to_gif(input_path: Path, out_dir: Path) -> Path:
    stem = input_path.stem.removeprefix("input_")
    palette = out_dir / "_palette.png"
    out_path = out_dir / f"{stem}.gif"
    p1 = subprocess.run(
        ["ffmpeg", "-y", "-i", str(input_path),
         "-vf", "fps=12,scale=480:-1:flags=lanczos,palettegen",
         str(palette)],
        capture_output=True, text=True, timeout=600,
    )
    if p1.returncode != 0:
        raise RuntimeError(f"palette failed: {p1.stderr.strip()[-500:]}")
    p2 = subprocess.run(
        ["ffmpeg", "-y", "-i", str(input_path), "-i", str(palette),
         "-filter_complex", "[0:v]fps=12,scale=480:-1:flags=lanczos[x];[x][1:v]paletteuse",
         str(out_path)],
        capture_output=True, text=True, timeout=600,
    )
    if p2.returncode != 0:
        raise RuntimeError(f"gif convert failed: {p2.stderr.strip()[-500:]}")
    palette.unlink(missing_ok=True)
    return out_path


def _pdf_to_text(input_path: Path, out_dir: Path) -> Path:
    import pdfplumber
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.txt"
    parts: list[str] = []
    with pdfplumber.open(input_path) as pdf:
        for page in pdf.pages:
            parts.append((page.extract_text() or "").strip())
    text = "\n\n".join(parts).strip()
    if text:
        out_path.write_text(maybe_translate_text(text), encoding="utf-8")
        return out_path
    # Empty result → likely a scanned PDF. OCR each page with auto language.
    import pytesseract
    pages = pdf2image.convert_from_path(str(input_path), dpi=300)
    if not pages:
        out_path.write_text("", encoding="utf-8")
        return out_path
    tess_lang = pick_tess_lang(osd_script=osd_script_from_image(pages[0]))
    ocr_parts = [pytesseract.image_to_string(pg, lang=tess_lang).strip() for pg in pages]
    body = maybe_translate_text("\n\n".join(ocr_parts))
    out_path.write_text(body, encoding="utf-8")
    return out_path


def _image_remove_bg(input_path: Path, out_dir: Path) -> Path:
    if is_offline():
        # rembg downloads u2net on first use. Fail closed rather than hanging
        # on huggingface when EXPORT_OFFLINE=1 and the model isn't cached.
        u2_home = Path(os.environ.get("U2NET_HOME", Path.home() / ".u2net"))
        if not any(u2_home.glob("*.onnx")):
            raise RuntimeError(
                "Offline mode — rembg model is not cached. Run once online "
                "or unset EXPORT_OFFLINE."
            )
    from rembg import remove
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_nobg.png"
    img = _open_raster(input_path)
    from io import BytesIO
    buf = BytesIO()
    img.save(buf, format="PNG")
    out_path.write_bytes(remove(buf.getvalue()))
    return out_path


def _image_to_svg(input_path: Path, out_dir: Path) -> Path:
    import vtracer
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.svg"
    work = input_path
    tmp = None
    if input_path.suffix.lower() in ({".svg"} | RAW_SUFFIXES | {".jxl", ".heic", ".heif", ".avif"}):
        img = _open_raster(input_path)
        tmp = out_dir / "_raster_for_svg.png"
        img.convert("RGBA").save(tmp, "PNG")
        work = tmp
    try:
        vtracer.convert_image_to_svg_py(str(work), str(out_path))
        return out_path
    finally:
        if tmp:
            tmp.unlink(missing_ok=True)


def _image_to_text(input_path: Path, out_dir: Path, job: dict | None = None) -> Path:
    import pytesseract
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.txt"
    img = _open_raster(input_path)
    tess_lang = pick_tess_lang(osd_script=osd_script_from_image(img))
    text = pytesseract.image_to_string(img, lang=tess_lang)
    record_on_job(job, detect_language_code(text))
    text = maybe_translate_text(text, job=job)
    out_path.write_text(text, encoding="utf-8")
    return out_path


def _pdf_merge(input_paths: list[Path], out_dir: Path) -> Path:
    from pypdf import PdfReader, PdfWriter
    writer = PdfWriter()
    for p in input_paths:
        for page in PdfReader(p).pages:
            writer.add_page(page)
    out_path = out_dir / "merged.pdf"
    with out_path.open("wb") as f:
        writer.write(f)
    return out_path


def _pdf_split(input_path: Path, out_dir: Path) -> Path:
    from pypdf import PdfReader, PdfWriter
    reader = PdfReader(input_path)
    stem = input_path.stem.removeprefix("input_")
    if len(reader.pages) <= 1:
        out_path = out_dir / f"{stem}_p001.pdf"
        w = PdfWriter()
        w.add_page(reader.pages[0])
        with out_path.open("wb") as f:
            w.write(f)
        return out_path
    pages_dir = out_dir / "_pages"
    pages_dir.mkdir(exist_ok=True)
    for i, page in enumerate(reader.pages, 1):
        w = PdfWriter()
        w.add_page(page)
        with (pages_dir / f"{stem}_p{i:03d}.pdf").open("wb") as f:
            w.write(f)
    out_path = out_dir / f"{stem}_pages.zip"
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_STORED) as zf:
        for f in sorted(pages_dir.iterdir()):
            zf.write(f, arcname=f.name)
    shutil.rmtree(pages_dir, ignore_errors=True)
    return out_path


def _pdf_rotate(input_path: Path, degrees: int, out_dir: Path) -> Path:
    from pypdf import PdfReader, PdfWriter
    if degrees not in (90, 180, 270):
        raise ValueError("degrees must be 90, 180, or 270")
    reader = PdfReader(input_path)
    writer = PdfWriter()
    for page in reader.pages:
        page.rotate(degrees)
        writer.add_page(page)
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_rotated.pdf"
    with out_path.open("wb") as f:
        writer.write(f)
    return out_path


def _pdf_watermark(input_path: Path, text: str, out_dir: Path) -> Path:
    from io import BytesIO
    from pypdf import PdfReader, PdfWriter
    from reportlab.lib.pagesizes import letter
    from reportlab.pdfgen import canvas

    reader = PdfReader(input_path)
    if not reader.pages:
        raise RuntimeError("pdf has no pages")
    first = reader.pages[0]
    width, height = float(first.mediabox.width), float(first.mediabox.height)

    wm_buf = BytesIO()
    c = canvas.Canvas(wm_buf, pagesize=(width, height))
    c.setFont("Helvetica-Bold", min(width, height) // 8)
    c.setFillColorRGB(0.5, 0.5, 0.5, alpha=0.25)
    c.saveState()
    c.translate(width / 2, height / 2)
    c.rotate(45)
    c.drawCentredString(0, 0, text)
    c.restoreState()
    c.save()
    wm_buf.seek(0)
    wm_page = PdfReader(wm_buf).pages[0]

    writer = PdfWriter()
    for page in reader.pages:
        page.merge_page(wm_page)
        writer.add_page(page)
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_watermarked.pdf"
    with out_path.open("wb") as f:
        writer.write(f)
    return out_path


def _pdf_encrypt(input_path: Path, password: str, out_dir: Path) -> Path:
    import pikepdf
    if not password:
        raise ValueError("password is required")
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_protected.pdf"
    with pikepdf.open(input_path) as pdf:
        pdf.save(out_path, encryption=pikepdf.Encryption(owner=password, user=password, R=6))
    return out_path


def _pdf_decrypt(input_path: Path, password: str, out_dir: Path) -> Path:
    import pikepdf
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_unlocked.pdf"
    with pikepdf.open(input_path, password=password) as pdf:
        pdf.save(out_path)
    return out_path


def run_pdf_merge_job(job_id: str, input_paths: list[Path], downloads_dir: Path) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "merging"
        if len(input_paths) < 2:
            raise ValueError("need at least 2 PDFs to merge")
        target = _pdf_merge(input_paths, out_dir)
        for p in input_paths:
            p.unlink(missing_ok=True)
        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)


def run_pdf_op_job(job_id: str, input_path: Path, op: str, value: str, downloads_dir: Path) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "processing"
        if op == "split":
            target = _pdf_split(input_path, out_dir)
        elif op == "rotate":
            target = _pdf_rotate(input_path, int(value or "90"), out_dir)
        elif op == "watermark":
            target = _pdf_watermark(input_path, value or "WATERMARK", out_dir)
        elif op == "encrypt":
            target = _pdf_encrypt(input_path, value, out_dir)
        elif op == "decrypt":
            target = _pdf_decrypt(input_path, value, out_dir)
        else:
            raise ValueError(f"unknown PDF op: {op}")
        input_path.unlink(missing_ok=True)
        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)


def _pdf_to_pdfa(input_path: Path, out_dir: Path) -> Path:
    """Run OCRmyPDF: skip-text mode means scanned pages get an OCR layer added,
    digital-born pages pass through untouched. Output is PDF/A for archival."""
    import ocrmypdf
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_searchable.pdf"
    ocrmypdf.ocr(
        input_path, out_path,
        output_type="pdfa",
        skip_text=True,
        deskew=True,
        language=pick_tess_lang(),
        progress_bar=False,
    )
    return out_path


def _pdf_to_docx(input_path: Path, out_dir: Path) -> Path:
    from pdf2docx import Converter
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.docx"
    cv = Converter(str(input_path))
    try:
        cv.convert(str(out_path), start=0, end=None)
    finally:
        cv.close()
    return out_path


def _pdf_to_markdown(input_path: Path, out_dir: Path) -> Path:
    """pdf → md: extract text (pdfplumber + OCR fallback) then prepend a title."""
    txt_path = _pdf_to_text(input_path, out_dir)
    text = txt_path.read_text(encoding="utf-8", errors="replace")
    txt_path.unlink(missing_ok=True)
    stem = input_path.stem.removeprefix("input_")
    title = stem.replace("_", " ").replace("-", " ").strip()
    out_path = out_dir / f"{stem}.md"
    out_path.write_text(f"# {title}\n\n{text}", encoding="utf-8")
    return out_path


def _pdf_to_notes(input_path: Path, out_dir: Path) -> Path:
    """pdf → NoteGPT-style notes. Groups extracted text into paragraphs at
    blank-line boundaries (one of the few structural signals that survives
    pdfplumber extraction). For scanned PDFs the OCR fallback in _pdf_to_text
    preserves enough whitespace for this to work."""
    import re
    txt_path = _pdf_to_text(input_path, out_dir)
    text = txt_path.read_text(encoding="utf-8", errors="replace")
    txt_path.unlink(missing_ok=True)
    paragraphs = [p.strip() for p in re.split(r"\n\s*\n+", text) if p.strip()]
    stem = input_path.stem.removeprefix("input_")
    title = stem.replace("_", " ").replace("-", " ").strip()
    body = "\n\n".join(paragraphs)
    out_path = out_dir / f"{stem}_notes.md"
    out_path.write_text(f"# {title}\n\n## Notes\n\n{body}\n", encoding="utf-8")
    return out_path


def _image_to_doc(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """image → docx/md/html/rtf/epub/odt/tex: OCR the image (Tesseract via
    _image_to_text) then wrap the extracted text in the requested doc format
    via pandoc. Useful for digitizing scanned pages where you want an editable
    file, not a flat .txt."""
    txt_path = _image_to_text(input_path, out_dir)
    try:
        return _convert_doc(txt_path, target_fmt, out_dir)
    finally:
        txt_path.unlink(missing_ok=True)


def _font_to_image(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """font → png/jpeg/webp: render a sample specimen of the font (name at top,
    then a large AaBbCc preview, then two pangrams). Falls back to a fontTools
    TTF intermediate for woff/woff2 since some Pillow builds can't decode them."""
    from PIL import Image, ImageDraw, ImageFont
    stem = input_path.stem.removeprefix("input_")
    work_font = input_path
    intermediate: Path | None = None
    if input_path.suffix.lower() in (".woff", ".woff2"):
        from fontTools.ttLib import TTFont
        intermediate = out_dir / "_font_preview.ttf"
        f = TTFont(str(input_path))
        f.flavor = None
        f.save(str(intermediate))
        work_font = intermediate
    try:
        canvas = Image.new("RGB", (1200, 540), "white")
        d = ImageDraw.Draw(canvas)
        d.text((40, 30), stem, fill="#111", font=ImageFont.truetype(str(work_font), 28))
        d.text((40, 90), "AaBbCcDd 0123 !?", fill="#000",
               font=ImageFont.truetype(str(work_font), 92))
        d.text((40, 240), "The quick brown fox jumps over the lazy dog.",
               fill="#222", font=ImageFont.truetype(str(work_font), 32))
        d.text((40, 320), "Pack my box with five dozen liquor jugs.",
               fill="#444", font=ImageFont.truetype(str(work_font), 22))
        d.text((40, 370), "Sphinx of black quartz, judge my vow.",
               fill="#444", font=ImageFont.truetype(str(work_font), 22))
        d.text((40, 470), "0123456789 ABCDEFGHIJKLMNOPQRSTUVWXYZ",
               fill="#666", font=ImageFont.truetype(str(work_font), 18))
        ext = IMAGE_EXT_OVERRIDE.get(target_fmt, target_fmt)
        save_kwargs = {"jpeg": {"quality": 92, "optimize": True},
                       "webp": {"quality": 92}}.get(target_fmt, {})
        out_path = out_dir / f"{stem}_specimen.{ext}"
        canvas.save(out_path, format=target_fmt.upper(), **save_kwargs)
        return out_path
    finally:
        if intermediate:
            intermediate.unlink(missing_ok=True)


def _office_to_audio(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """office (any) → mp3/m4a/wav audiobook via office → pdf → text → TTS."""
    txt_path = _office_to_text(input_path, out_dir)
    try:
        return _text_to_audio(txt_path, target_fmt, out_dir)
    finally:
        txt_path.unlink(missing_ok=True)


def _pdf_to_audio(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """pdf → mp3/m4a/wav audiobook via pdf → text → TTS."""
    txt_path = _pdf_to_text(input_path, out_dir)
    try:
        return _text_to_audio(txt_path, target_fmt, out_dir)
    finally:
        txt_path.unlink(missing_ok=True)


def _subtitle_to_audio(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """subtitle → mp3/m4a/wav via subtitle → text → TTS. Useful when you want
    the caption text read aloud (e.g. accessibility, dub-overs)."""
    txt_path = _subtitle_to_text(input_path, "txt", out_dir)
    try:
        return _text_to_audio(txt_path, target_fmt, out_dir)
    finally:
        txt_path.unlink(missing_ok=True)


def _text_source_to_audiogram(input_path: Path, out_dir: Path, make_audio) -> Path:
    """Compose: text-source → mp3 (via TTS) → audiogram (waveform mp4).
    `make_audio` is whichever wrapper produces the mp3 (office/pdf/subtitle)."""
    mp3_path = make_audio(input_path, "mp3", out_dir)
    try:
        return _audio_to_audiogram(mp3_path, out_dir)
    finally:
        mp3_path.unlink(missing_ok=True)


def _text_to_audio(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    suffix = input_path.suffix.lower()
    if suffix == ".txt":
        text = input_path.read_text(encoding="utf-8", errors="replace")
    else:
        r = subprocess.run(
            ["pandoc", str(input_path), "-t", "plain"],
            capture_output=True, text=True, timeout=120,
        )
        if r.returncode != 0:
            raise RuntimeError(f"pandoc -> plain failed: {r.stderr.strip()[-500:]}")
        text = r.stdout
    if not text.strip():
        raise RuntimeError("input contains no readable text")

    text = maybe_translate_text(text)
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"

    lang = requested_translate_to() or detect_language_code(text)
    # Choose synthesizer: macOS `say` if available, else piper-tts.
    if sys.platform == "darwin" and shutil.which("say"):
        raw = out_dir / "_tts.aiff"
        cmd = ["say", "-o", str(raw)]
        voice = pick_say_voice(lang)
        if voice:
            cmd.extend(["-v", voice])
        # stdin, not argv — long documents blow the argument list otherwise.
        r = subprocess.run(
            cmd, input=text, capture_output=True, text=True, timeout=600,
        )
        if r.returncode != 0:
            raise RuntimeError(f"say failed: {r.stderr.strip()[-500:]}")
    else:
        voice_path = pick_piper_voice(lang)
        if voice_path is None or not voice_path.exists():
            raise RuntimeError(
                "TTS unavailable: install piper-tts and set "
                "EXPORT_PIPER_VOICE to a .onnx voice path (or drop matching "
                "voices into ./voices/{lang}_*.onnx)"
            )
        if not shutil.which("piper"):
            raise RuntimeError("piper binary not in PATH (pip install piper-tts)")
        raw = out_dir / "_tts.wav"
        r = subprocess.run(
            ["piper", "-m", str(voice_path), "-f", str(raw)],
            input=text, capture_output=True, text=True, timeout=600,
        )
        if r.returncode != 0:
            raise RuntimeError(f"piper failed: {r.stderr.strip()[-500:]}")

    cmd = ["ffmpeg", "-y", "-i", str(raw)] + AUDIO_FFMPEG_OPTS[target_fmt] + [str(out_path)]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    raw.unlink(missing_ok=True)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg tts failed: {r.stderr.strip()[-500:]}")
    return out_path


def _format_ts(seconds: float, comma: bool) -> str:
    h = int(seconds // 3600)
    m = int((seconds % 3600) // 60)
    s = seconds % 60
    out = f"{h:02d}:{m:02d}:{s:06.3f}"
    return out.replace(".", ",") if comma else out


def _write_transcript(segments: list, target_fmt: str, stem: str, out_dir: Path,
                      language_code: str | None = None) -> Path:
    """Write a transcript in the requested format from a list of segments.
    Each segment is a dict-like with .start, .end, .text (or ['start'], ['end'], ['text']).
    Shared by both the local Whisper path and the Groq cloud path."""
    def g(s, k):
        return s[k] if isinstance(s, dict) else getattr(s, k)
    segments = maybe_translate_segment(segments)
    if requested_translate_to():
        language_code = requested_translate_to()
    out_path = out_dir / f"{stem}.{target_fmt}"
    if target_fmt == "txt":
        out_path.write_text("\n".join(g(s, "text").strip() for s in segments) + "\n", encoding="utf-8")
    elif target_fmt == "notes":
        # NoteGPT-style: paragraphs at >1.5s pause gap, no timestamp clutter.
        paragraphs: list[list[str]] = [[]]
        last_end = 0.0
        for s in segments:
            if g(s, "start") - last_end > 1.5 and paragraphs[-1]:
                paragraphs.append([])
            paragraphs[-1].append(g(s, "text").strip())
            last_end = g(s, "end")
        body = "\n\n".join(" ".join(p) for p in paragraphs if p)
        title = stem.replace("_", " ").replace("-", " ").strip()
        out_path = out_dir / f"{stem}_notes.md"
        lang_line = ""
        if language_code:
            lang_line = f"Detected language: {language_name(language_code)} (`{language_code}`)\n\n"
        out_path.write_text(f"# {title}\n\n{lang_line}## Transcript\n\n{body}\n", encoding="utf-8")
    elif target_fmt == "srt":
        lines: list[str] = []
        for i, s in enumerate(segments, 1):
            lines += [str(i),
                      f"{_format_ts(g(s, 'start'), True)} --> {_format_ts(g(s, 'end'), True)}",
                      g(s, "text").strip(), ""]
        out_path.write_text("\n".join(lines), encoding="utf-8")
    elif target_fmt == "vtt":
        lines = ["WEBVTT", ""]
        for s in segments:
            lines += [f"{_format_ts(g(s, 'start'), False)} --> {_format_ts(g(s, 'end'), False)}",
                      g(s, "text").strip(), ""]
        out_path.write_text("\n".join(lines), encoding="utf-8")
    else:
        raise ValueError(f"unsupported transcript format: {target_fmt}")
    return out_path


def _spread_word(text: str, start: float, end: float) -> list[dict]:
    """Fallback word times for a segment with none: split on whitespace and share
    the segment's span by character length. Confidence is unknown (None)."""
    token = text.split()
    if not token:
        return []
    total = sum(len(t) for t in token)
    span = max(end - start, 0.0)
    out: list[dict] = []
    t = start
    for w in token:
        dur = span * len(w) / total
        out.append({"text": w, "start": t, "end": t + dur, "confidence": None})
        t += dur
    return out


def _segment_word(s) -> list[dict]:
    """Words of one segment as dicts. faster-whisper Segment.words carry
    .word/.start/.end/.probability; Groq segment dicts carry a `words` list we
    built as {text, start, end, confidence}. Missing → even spread fallback."""
    if isinstance(s, dict):
        raw = s.get("words")
        text, start, end = s["text"], s["start"], s["end"]
    else:
        raw = getattr(s, "words", None)
        text, start, end = s.text, s.start, s.end
    if not raw:
        return _spread_word(text, float(start), float(end))
    out: list[dict] = []
    for w in raw:
        if isinstance(w, dict):
            word, ws, we, p = w.get("text", ""), w["start"], w["end"], w.get("confidence")
        else:
            word, ws, we, p = w.word, w.start, w.end, getattr(w, "probability", None)
        word = (word or "").strip()
        if word:
            out.append({"text": word, "start": float(ws), "end": float(we),
                        "confidence": None if p is None else float(p)})
    return out


def _write_words(segments: list, stem: str, out_dir: Path,
                 language_code: str | None = None) -> Path:
    """words.json — { language, word: [{ text, start, end, confidence }] }.
    Seconds from the start of the input, 3 decimals. Not translated: a
    translation has no spoken time per word."""
    import json as json_mod
    word: list[dict] = []
    for s in segments:
        for w in _segment_word(s):
            word.append({
                "text": w["text"],
                "start": round(w["start"], 3),
                "end": round(max(w["end"], w["start"]), 3),
                "confidence": None if w["confidence"] is None else round(w["confidence"], 3),
            })
    out_path = out_dir / f"{stem}_words.json"
    out_path.write_text(
        json_mod.dumps({"language": language_code, "word": word}, ensure_ascii=False),
        encoding="utf-8",
    )
    return out_path


def _audio_video_to_transcript_groq(input_path: Path, out_dir: Path,
                                     job: dict | None = None,
                                     is_word: bool = False) -> tuple[list, str | None]:
    """Transcribe via Groq's Whisper API. Free tier: 25 MB upload limit, dev tier: 100 MB.
    We pre-process to flac mono 16 kHz (Whisper's native format) for max compression,
    then segment into ~20-min chunks (well under both limits even for music). Each
    chunk's segment timestamps are offset so the final transcript stitches cleanly."""
    import requests
    api_key = os.environ["EXPORT_GROQ_API_KEY"]
    model_id = os.environ.get("EXPORT_GROQ_MODEL", "whisper-large-v3-turbo")
    chunk_seconds = int(os.environ.get("EXPORT_GROQ_CHUNK_SECONDS", "1200"))  # 20 min default

    # Pass 1: re-encode to mp3 mono 16 kHz @ 64 kbps. mp3 is plenty for Whisper
    # speech recognition (it's downsampled to 16 kHz internally anyway), and
    # segments cleanly with -c copy because mp3 has independent frames.
    # 20 min @ 64 kbps mono ≈ 9.6 MB — safely under Groq's 25 MB free-tier cap.
    pre_path = out_dir / "_groq_input.mp3"
    r = subprocess.run(
        ["ffmpeg", "-y", "-i", str(input_path), "-vn", "-ac", "1", "-ar", "16000",
         "-c:a", "libmp3lame", "-b:a", "64k", str(pre_path)],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg preprocess for groq failed: {r.stderr.strip()[-500:]}")

    # Pass 2: segment into N chunks of `chunk_seconds` each. mp3 segments cleanly.
    chunk_dir = out_dir / "_groq_chunks"
    chunk_dir.mkdir(exist_ok=True)
    r = subprocess.run(
        ["ffmpeg", "-y", "-i", str(pre_path),
         "-f", "segment", "-segment_time", str(chunk_seconds),
         "-c", "copy", str(chunk_dir / "chunk_%03d.mp3")],
        capture_output=True, text=True,
    )
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg segment for groq failed: {r.stderr.strip()[-500:]}")
    chunks = sorted(chunk_dir.glob("chunk_*.mp3"))
    if not chunks:
        raise RuntimeError("groq: no chunks produced")

    # Transcribe each chunk, shifting timestamps by the cumulative offset.
    all_segments: list[dict] = []
    detected_language: str | None = whisper_language_arg(None)
    groq_data: dict = {"model": model_id, "response_format": "verbose_json"}
    if detected_language:
        groq_data["language"] = detected_language
    if is_word:
        # OpenAI-compatible word granularity: top-level `words` [{word, start, end}]
        # with no per-word probability, so confidence is None. If a response has
        # no `words`, _segment_word spreads each segment's span by length.
        groq_data["timestamp_granularities[]"] = ["word", "segment"]
    for i, chunk in enumerate(chunks):
        offset = i * chunk_seconds
        with chunk.open("rb") as f:
            resp = requests.post(
                "https://api.groq.com/openai/v1/audio/transcriptions",
                headers={"Authorization": f"Bearer {api_key}"},
                files={"file": (chunk.name, f, "audio/mpeg")},
                data=groq_data,
                timeout=300,
            )
        if resp.status_code != 200:
            raise RuntimeError(f"groq API error {resp.status_code}: {resp.text[:300]}")
        result = resp.json()
        if not detected_language:
            detected_language = result.get("language") or None
        chunk_segments: list[dict] = []
        for seg in result.get("segments", []):
            chunk_segments.append({
                "start": seg["start"] + offset,
                "end":   seg["end"]   + offset,
                "text":  seg["text"],
            })
        if is_word and chunk_segments:
            # Bucket each word under the last segment that starts at or before it.
            for w in result.get("words") or []:
                ws = w["start"] + offset
                home = chunk_segments[0]
                for seg in chunk_segments:
                    if seg["start"] <= ws:
                        home = seg
                    else:
                        break
                home.setdefault("words", []).append({
                    "text": w.get("word", ""), "start": ws,
                    "end": w["end"] + offset, "confidence": None,
                })
        all_segments.extend(chunk_segments)
        if job is not None:
            job["progress"] = max(1, min(99, int(100 * (i + 1) / len(chunks))))

    pre_path.unlink(missing_ok=True)
    shutil.rmtree(chunk_dir, ignore_errors=True)
    return all_segments, detected_language


def _require_network(action: str) -> None:
    if is_offline():
        raise RuntimeError(
            f"Offline mode — {action} needs a network. Unset EXPORT_OFFLINE "
            "or drop a local file instead."
        )


def _ct2_ready(path: Path) -> bool:
    return path.is_dir() and (
        (path / "model.bin").exists() or any(path.glob("model*.bin"))
    )


def ensure_dialect_ct2() -> Path:
    """Local CTranslate2 copy of whisper-medium-ph. Fail closed if missing."""
    dest = dialect_stt_dir()
    name = dialect_stt_model()
    if _ct2_ready(dest):
        return dest
    if is_offline():
        raise RuntimeError(
            f"missing dialect STT model {name} at {dest} — "
            "unset EXPORT_OFFLINE or convert once online"
        )
    dest.mkdir(parents=True, exist_ok=True)
    try:
        from ctranslate2.converters.transformers import TransformersConverter
        conv = TransformersConverter(
            name,
            copy_files=["tokenizer.json", "preprocessor_config.json"],
        )
        conv.convert(str(dest), quantization="int8", force=True)
    except Exception as e:
        raise RuntimeError(f"missing dialect STT model {name}: {e}") from e
    if not _ct2_ready(dest):
        raise RuntimeError(f"missing dialect STT model {name} at {dest}")
    return dest


def _collect_transcript(input_path: Path, out_dir: Path,
                        job: dict | None = None,
                        is_word: bool = False) -> tuple[list, str | None]:
    """Run STT once. Returns (segments, language_code). Groq if keyed and online,
    else local faster-whisper. Default local model is large-v3-turbo (multilingual);
    distil-large-v3 is English-only and is no longer the default.
    Pinned PH dialects (Cebuano, Ilocano, …) load whisper-medium-ph, not turbo+tl.
    is_word asks for word timestamps (segment.words) on every path."""
    pinned = requested_language_code()
    use_dialect = is_ph_dialect(pinned)

    if os.environ.get("EXPORT_GROQ_API_KEY") and not is_offline() and not use_dialect:
        return _audio_video_to_transcript_groq(input_path, out_dir, job, is_word=is_word)

    from faster_whisper import WhisperModel
    if use_dialect:
        load_name = str(ensure_dialect_ct2())
        download_root = None
        local_only = True
    else:
        load_name = default_whisper_model()
        download_root = os.environ.get("EXPORT_WHISPER_MODEL_DIR") or None
        local_only = is_offline()
    if _whisper_state["model"] is None or _whisper_state["name"] != load_name:
        _whisper_state["model"] = WhisperModel(
            load_name, device="cpu", compute_type="int8",
            download_root=download_root,
            local_files_only=local_only,
        )
        _whisper_state["name"] = load_name
    model = _whisper_state["model"]

    transcribe_kw: dict = {"beam_size": 1}
    if is_word:
        transcribe_kw["word_timestamps"] = True
    lang_arg = None if use_dialect else whisper_language_arg(None)
    if lang_arg:
        transcribe_kw["language"] = lang_arg
    elif (not use_dialect) and is_whisper_english_only(load_name):
        transcribe_kw["language"] = "en"

    segments_iter, info = model.transcribe(str(input_path), **transcribe_kw)
    duration = max(float(getattr(info, "duration", 0.0) or 0.0), 0.001)
    segments = []
    for s in segments_iter:
        segments.append(s)
        if job is not None:
            job["progress"] = max(1, min(99, int(100 * s.end / duration)))
    if use_dialect:
        detected = pinned
    else:
        detected = getattr(info, "language", None) or lang_arg
    return segments, detected


def _audio_video_to_transcript(input_path: Path, target_fmt: str, out_dir: Path,
                                job: dict | None = None) -> Path:
    is_word = target_fmt in WHISPER_WORDS
    segments, detected = _collect_transcript(input_path, out_dir, job, is_word=is_word)
    record_on_job(job, detected)
    stem = input_path.stem.removeprefix("input_")
    if is_word:
        return _write_words(segments, stem, out_dir, language_code=detected)
    return _write_transcript(segments, target_fmt, stem, out_dir, language_code=detected)


def _audio_denoise(input_path: Path, out_dir: Path) -> Path:
    import noisereduce as nr
    import soundfile as sf
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_denoised.wav"
    data, sr = sf.read(str(input_path))
    cleaned = nr.reduce_noise(y=data, sr=sr, stationary=True)
    sf.write(str(out_path), cleaned, sr)
    return out_path


def _audio_to_audiogram(input_path: Path, out_dir: Path) -> Path:
    """Render audio as a 1080×1080 mp4 with a static waveform of the whole clip
    + an animated playhead that sweeps across as the audio plays.

    Two-pass: (1) showwavespic generates a high-quality static waveform image
    of the entire audio (filled-envelope mode, dynaudnorm normalized so quiet
    podcasts still show detail); (2) composite that image onto a black canvas
    and overlay a moving 8-px white playhead whose x position interpolates
    from 0 to canvas width over the audio's duration.

    Why waveform over spectrogram: spectrogram becomes unreadable noise when
    you compress 1+ hour of audio into 1080 pixels (each column = ~4 s of FFT
    averaging). Waveform compresses gracefully — the amplitude envelope still
    reads as loud/quiet patterns over time at any clip length."""
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_audiogram.mp4"

    # Probe duration so the playhead's drawbox knows how fast to sweep.
    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration",
         "-of", "default=noprint_wrappers=1:nokey=1", str(input_path)],
        capture_output=True, text=True,
    )
    try:
        duration = max(float(probe.stdout.strip()), 0.5)
    except (ValueError, AttributeError):
        duration = 60.0  # fallback; playhead just won't be perfectly accurate

    spec_png = out_dir / f"{stem}_wave.png"
    # Pass 1: full-audio waveform → static png. dynaudnorm normalizes amplitude
    # so quiet content still has visible peaks; showwavespic with draw=full
    # fills between the min/max envelope (much more visually substantial than
    # a thin line); split_channels=0 collapses stereo into one shape.
    p1 = subprocess.run([
        "ffmpeg", "-y", "-i", str(input_path),
        "-filter_complex",
        "aformat=channel_layouts=mono,dynaudnorm=p=0.9,"
        "showwavespic=s=1080x500:colors=0x58a6ff:split_channels=0:draw=full",
        str(spec_png),
    ], capture_output=True, text=True)
    if p1.returncode != 0:
        raise RuntimeError(f"ffmpeg audiogram (waveform) failed: {p1.stderr.strip()[-500:]}")

    # Pass 2: loop the spectrogram on a black canvas + sweeping playhead + audio.
    # Playhead is a separate lavfi color source (8×500 px) overlaid with
    # eval=frame so the x expression re-evaluates per frame. drawbox in this
    # ffmpeg lacks an eval option and silently bakes its expressions at init
    # time, so overlay is the only filter that can do the sweep correctly.
    cmd = [
        "ffmpeg", "-y",
        "-loop", "1", "-framerate", "30", "-i", str(spec_png),    # 0: spectrogram
        "-i", str(input_path),                                      # 1: audio
        "-f", "lavfi", "-i", "color=c=black:s=1080x1080:r=30",     # 2: canvas
        "-f", "lavfi", "-i", f"color=c=white@0.85:s=8x500:d={duration}:r=30",  # 3: playhead
        "-filter_complex",
        f"[2:v][0:v]overlay=x=0:y=(H-h)/2[withspec];"
        f"[withspec][3:v]overlay=x='min(t/{duration}*1080,1080-8)':y=290:"
        f"eval=frame:format=auto,format=yuv420p[v]",
        "-map", "[v]", "-map", "1:a",
        "-c:v", "libx264", "-preset", "fast", "-crf", "23",
        "-c:a", "aac", "-b:a", "192k",
        "-movflags", "+faststart", "-shortest",
        str(out_path),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True)
    spec_png.unlink(missing_ok=True)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg audiogram (mux) failed: {r.stderr.strip()[-500:]}")
    return out_path


def _audio_separate_stems(input_path: Path, out_dir: Path) -> Path:
    """Run Demucs htdemucs to split audio into vocals/drums/bass/other, then zip the stems."""
    if is_offline():
        hub = Path.home() / ".cache" / "torch" / "hub"
        if not any(hub.rglob("htdemucs*")):
            raise RuntimeError(
                "Offline mode — Demucs model is not cached. Run once online "
                "or unset EXPORT_OFFLINE."
            )
    stem = input_path.stem.removeprefix("input_")
    sep_root = out_dir / "_demucs"
    sep_root.mkdir(exist_ok=True)
    cmd = [
        sys.executable, "-m", "demucs.separate",
        "-o", str(sep_root),
        "-n", "htdemucs",
        str(input_path),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=1200)
    if r.returncode != 0:
        raise RuntimeError(f"demucs failed: {r.stderr.strip()[-500:]}")
    # Demucs writes to <out>/htdemucs/<input_stem>/{vocals,drums,bass,other}.wav
    candidates = list(sep_root.rglob("vocals.*"))
    if not candidates:
        raise RuntimeError("demucs produced no stems")
    track_dir = candidates[0].parent
    out_path = out_dir / f"{stem}_stems.zip"
    with zipfile.ZipFile(out_path, "w", zipfile.ZIP_STORED) as zf:
        for f in sorted(track_dir.iterdir()):
            zf.write(f, arcname=f.name)
    shutil.rmtree(sep_root, ignore_errors=True)
    return out_path


def _convert_audio(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"
    if target_fmt == "wav" and _flac_magic(input_path):
        _export_audio_wav(input_path, out_path)
        return out_path
    cmd = ["ffmpeg", "-y", "-i", str(input_path), "-vn"] + AUDIO_FFMPEG_OPTS[target_fmt] + [str(out_path)]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg audio failed: {r.stderr.strip()[-500:]}")
    return out_path


def _flac_magic(path: Path) -> bool:
    try:
        with path.open("rb") as handle:
            return handle.read(4) == b"fLaC"
    except OSError:
        return False


def _export_audio_wav_bin() -> Path:
    override = os.environ.get("EXPORT_AUDIO_WAV")
    if override:
        return Path(override)
    root = Path(__file__).resolve().parents[1]
    release = root / "target" / "release" / "export_audio_wav"
    if release.exists():
        return release
    return root / "target" / "debug" / "export_audio_wav"


def _export_audio_wav(input_path: Path, out_path: Path) -> None:
    bin_path = _export_audio_wav_bin()
    r = subprocess.run(
        [str(bin_path), str(input_path), str(out_path)],
        capture_output=True, text=True, timeout=600,
    )
    if r.returncode != 0 or not out_path.is_file():
        detail = (r.stderr or r.stdout or f"missing {bin_path}").strip()
        raise RuntimeError(f"export-audio wav failed: {detail[-500:]}")


def _office_to_text(input_path: Path, out_dir: Path) -> Path:
    """Universal office → txt via the office→pdf→text chain. Works for any
    office input (docx/pptx/xlsx/doc/ppt/xls/ods/odp/odt). Lower fidelity than
    direct pandoc for docx/odt/pptx but works on the formats pandoc can't read."""
    pdf_path = _office_to_pdf(input_path, out_dir)
    try:
        out_path = _pdf_to_text(pdf_path, out_dir)
    finally:
        pdf_path.unlink(missing_ok=True)
    return out_path


def _office_to_images(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """Render each slide/page of an office document as an image.
    Path: office → pdf (LibreOffice headless) → image(s) (pdf2image at 200 DPI).
    Single-page docs → one image; multi-page → zip. Intermediate pdf is deleted.
    Killer use case: pptx slide deck → png per slide for embedding in docs/web."""
    pdf_path = _office_to_pdf(input_path, out_dir)
    try:
        out_path = _pdf_to_images(pdf_path, target_fmt, out_dir)
    finally:
        pdf_path.unlink(missing_ok=True)
    return out_path


def _office_to_pdf(input_path: Path, out_dir: Path) -> Path:
    # Each conversion uses an isolated profile dir so concurrent jobs don't
    # collide on LibreOffice's user profile.
    profile_dir = out_dir / "_lo_profile"
    r = subprocess.run(
        ["soffice", "--headless",
         f"-env:UserInstallation=file://{profile_dir}",
         "--convert-to", "pdf",
         "--outdir", str(out_dir), str(input_path)],
        capture_output=True, text=True, timeout=300,
    )
    shutil.rmtree(profile_dir, ignore_errors=True)
    if r.returncode != 0:
        raise RuntimeError(f"soffice failed: {r.stderr.strip()[-500:]}")
    expected = out_dir / (input_path.stem + ".pdf")
    if not expected.exists():
        raise RuntimeError("soffice produced no output")
    final = out_dir / (input_path.stem.removeprefix("input_") + ".pdf")
    if expected != final:
        expected.rename(final)
    return final


def _doc_to_pdf(input_path: Path, out_dir: Path) -> Path:
    from weasyprint import HTML
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.pdf"
    suffix = input_path.suffix.lower()
    if suffix in (".html", ".htm"):
        html_str = input_path.read_text(encoding="utf-8")
    else:
        r = subprocess.run(
            ["pandoc", str(input_path), "-t", "html", "-s"],
            capture_output=True, text=True, timeout=120,
        )
        if r.returncode != 0:
            raise RuntimeError(f"pandoc->html failed: {r.stderr.strip()[-500:]}")
        html_str = r.stdout
    HTML(string=html_str).write_pdf(out_path)
    return out_path


def _escape_subtitles_path(path: Path) -> str:
    """Escape a filesystem path for ffmpeg's `subtitles=` filter."""
    return str(path).replace("\\", "\\\\").replace(":", "\\:").replace("'", "\\'")


def _ffmpeg_has_subtitles_filter() -> bool:
    r = subprocess.run(
        ["ffmpeg", "-hide_banner", "-filters"],
        capture_output=True, text=True, timeout=15,
    )
    blob = (r.stdout or "") + (r.stderr or "")
    return " subtitles " in blob or "\nsubtitles " in blob


def _srt_plain_text(path: Path) -> str:
    line_out: list[str] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        s = line.strip()
        if not s or s.isdigit() or "-->" in s:
            continue
        line_out.append(s)
    return " ".join(line_out)


def _caption_overlay_png(srt_path: Path, width: int, height: int, png_path: Path) -> None:
    """Rasterize srt text onto a transparent PNG (used when ffmpeg lacks libass)."""
    from PIL import Image, ImageDraw, ImageFont
    text = _srt_plain_text(srt_path) or " "
    img = Image.new("RGBA", (width, height), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)
    font_size = max(16, height // 16)
    font = None
    for candidate in (
        "/System/Library/Fonts/Supplemental/Arial.ttf",
        "/System/Library/Fonts/Helvetica.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ):
        if Path(candidate).exists():
            try:
                font = ImageFont.truetype(candidate, font_size)
                break
            except OSError:
                continue
    if font is None:
        font = ImageFont.load_default()
    max_w = int(width * 0.9)
    word = text.split()
    wrapped: list[str] = []
    cur = ""
    for w in word:
        trial = (cur + " " + w).strip()
        bbox = draw.textbbox((0, 0), trial, font=font)
        if bbox[2] - bbox[0] <= max_w:
            cur = trial
        else:
            if cur:
                wrapped.append(cur)
            cur = w
    if cur:
        wrapped.append(cur)
    wrapped = wrapped[:8] or [" "]
    line_h = font_size + 6
    block_h = line_h * len(wrapped) + 16
    top = height - block_h - 12
    draw.rectangle((0, top, width, height), fill=(0, 0, 0, 160))
    y = top + 8
    for line in wrapped:
        bbox = draw.textbbox((0, 0), line, font=font)
        tw = bbox[2] - bbox[0]
        draw.text(((width - tw) / 2, y), line, font=font, fill=(255, 255, 255, 255))
        y += line_h
    img.save(png_path, "PNG")


def _pdf_to_doc(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """pdf → html/epub/rtf/odt/tex via markdown + pandoc."""
    md_path = _pdf_to_markdown(input_path, out_dir)
    try:
        return _convert_doc(md_path, target_fmt, out_dir)
    finally:
        md_path.unlink(missing_ok=True)


def _cbz_to_pdf(input_path: Path, out_dir: Path) -> Path:
    """Comic .cbz (zip of images) → multi-page PDF."""
    extract_dir = out_dir / "_cbz"
    extract_dir.mkdir(exist_ok=True)
    with zipfile.ZipFile(input_path) as zf:
        zf.extractall(extract_dir)
    image_file = sorted(
        p for p in extract_dir.rglob("*")
        if p.is_file() and p.suffix.lower() in IMAGE_SUFFIXES
    )
    if not image_file:
        raise RuntimeError("cbz contained no images")
    try:
        return _images_to_pdf(image_file, out_dir)
    finally:
        shutil.rmtree(extract_dir, ignore_errors=True)


def _transcript_kit(input_path: Path, out_dir: Path, job: dict | None = None) -> Path:
    """One STT pass → zip of txt + srt + vtt + notes + language.txt."""
    segments, detected = _collect_transcript(input_path, out_dir, job)
    record_on_job(job, detected)
    stem = input_path.stem.removeprefix("input_")
    written: list[Path] = []
    for fmt in ("txt", "srt", "vtt", "notes"):
        written.append(_write_transcript(segments, fmt, stem, out_dir, language_code=detected))
    lang_path = out_dir / f"{stem}.language.txt"
    lang_path.write_text(
        f"{detected or 'unknown'}\n{language_name(detected)}\n",
        encoding="utf-8",
    )
    written.append(lang_path)
    zip_path = out_dir / f"{stem}_kit.zip"
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_STORED) as zf:
        for p in written:
            zf.write(p, arcname=p.name)
    for p in written:
        p.unlink(missing_ok=True)
    return zip_path


def _video_extract_subs(input_path: Path, out_dir: Path, job: dict | None = None) -> Path:
    """Pull the first embedded subtitle stream; Whisper srt if the file has none."""
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.srt"
    r = subprocess.run(
        ["ffmpeg", "-y", "-i", str(input_path), "-map", "0:s:0", str(out_path)],
        capture_output=True, text=True, timeout=120,
    )
    if r.returncode == 0 and out_path.exists() and out_path.stat().st_size > 0:
        return out_path
    out_path.unlink(missing_ok=True)
    return _audio_video_to_transcript(input_path, "srt", out_dir, job=job)


def _video_frames(input_path: Path, out_dir: Path) -> Path:
    """Sample up to 120 PNG frames at 1 fps (first 2 minutes) and zip them."""
    stem = input_path.stem.removeprefix("input_")
    frames_dir = out_dir / "_frames"
    frames_dir.mkdir(exist_ok=True)
    r = subprocess.run(
        ["ffmpeg", "-y", "-i", str(input_path),
         "-vf", "fps=1", "-frames:v", "120",
         str(frames_dir / "frame_%04d.png")],
        capture_output=True, text=True, timeout=300,
    )
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg frames failed: {r.stderr.strip()[-500:]}")
    frame = sorted(frames_dir.glob("frame_*.png"))
    if not frame:
        raise RuntimeError("no frames extracted")
    zip_path = out_dir / f"{stem}_frames.zip"
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_STORED) as zf:
        for p in frame:
            zf.write(p, arcname=p.name)
    shutil.rmtree(frames_dir, ignore_errors=True)
    return zip_path


def _video_captioned(input_path: Path, out_dir: Path, job: dict | None = None) -> Path:
    """Transcribe, then burn captions into an h.264 mp4.

    Prefers ffmpeg's `subtitles` filter (libass). Homebrew ffmpeg 8.x is often
    built without libass, so we fall back to a PIL overlay + `overlay` filter.
    """
    srt_path = _audio_video_to_transcript(input_path, "srt", out_dir, job=job)
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_captioned.mp4"
    burn = out_dir / "burn.srt"
    if srt_path.resolve() != burn.resolve():
        shutil.copy2(srt_path, burn)
        srt_path.unlink(missing_ok=True)

    if _ffmpeg_has_subtitles_filter():
        r = subprocess.run(
            ["ffmpeg", "-y", "-i", str(input_path.resolve()),
             "-vf", "subtitles=filename=burn.srt",
             "-c:v", "libx264", "-preset", "fast", "-crf", "23",
             "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart",
             str(out_path.resolve())],
            cwd=str(out_dir),
            capture_output=True, text=True, timeout=1200,
        )
        burn.unlink(missing_ok=True)
        if r.returncode != 0:
            raise RuntimeError(f"ffmpeg captioned failed: {r.stderr.strip()[-500:]}")
        return out_path

    probe = subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0",
         "-show_entries", "stream=width,height", "-of", "csv=p=0:s=x",
         str(input_path)],
        capture_output=True, text=True, timeout=30,
    )
    if probe.returncode != 0 or "x" not in (probe.stdout or ""):
        burn.unlink(missing_ok=True)
        raise RuntimeError(f"ffprobe failed: {probe.stderr.strip()[-300:]}")
    w_s, h_s = probe.stdout.strip().split("x", 1)
    overlay = out_dir / "caption.png"
    _caption_overlay_png(burn, int(w_s), int(h_s), overlay)
    r = subprocess.run(
        ["ffmpeg", "-y", "-i", str(input_path.resolve()), "-i", str(overlay),
         "-filter_complex", "overlay=0:0",
         "-c:v", "libx264", "-preset", "fast", "-crf", "23",
         "-c:a", "aac", "-b:a", "192k", "-movflags", "+faststart",
         str(out_path.resolve())],
        capture_output=True, text=True, timeout=1200,
    )
    burn.unlink(missing_ok=True)
    overlay.unlink(missing_ok=True)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg captioned overlay failed: {r.stderr.strip()[-500:]}")
    return out_path


def _notes_to_doc(input_path: Path, target_fmt: str, out_dir: Path,
                  job: dict | None = None) -> Path:
    """audio/video → notes.md → pdf/docx/epub via pandoc (or weasyprint for pdf)."""
    notes_path = _audio_video_to_transcript(input_path, "notes", out_dir, job=job)
    try:
        if target_fmt == "pdf":
            return _doc_to_pdf(notes_path, out_dir)
        return _convert_doc(notes_path, target_fmt, out_dir)
    finally:
        notes_path.unlink(missing_ok=True)


def _video_to_png(input_path: Path, out_dir: Path) -> Path:
    """One sampled still from a video (pictures of frames, not a transcript)."""
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.png"
    r = subprocess.run(
        ["ffmpeg", "-y", "-i", str(input_path), "-frames:v", "1", "-q:v", "2",
         str(out_path)],
        capture_output=True, text=True, timeout=120,
    )
    if r.returncode != 0 or not out_path.exists() or out_path.stat().st_size == 0:
        raise RuntimeError(f"ffmpeg still frame failed: {(r.stderr or '')[-400:]}")
    return out_path


def _to_translated_txt(input_path: Path, out_dir: Path, job: dict | None = None) -> Path:
    """Any text-bearing input → translated .txt. Defaults target to English
    when translate_to was not pinned (so the `translate` format always does work)."""
    if not requested_translate_to():
        with use_translate_to("en"):
            return _to_translated_txt(input_path, out_dir, job)
    txt_path = _convert_one(input_path, "txt", out_dir, job=job)
    target = requested_translate_to() or "en"
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target}.txt"
    if txt_path != out_path:
        txt_path.replace(out_path)
    return out_path


def _convert_one(input_path: Path, fmt: str, out_dir: Path,
                 quality: int | None = None, job: dict | None = None) -> Path:
    """Single-file convert dispatch. Shared by run_convert_job and run_batch_convert_job.
    Caller is responsible for cleanup of input_path after success."""
    suffix = input_path.suffix.lower()
    if fmt in TRANSLATE_OUTPUT:
        return _to_translated_txt(input_path, out_dir, job=job)
    is_image_in       = suffix in IMAGE_SUFFIXES
    is_pdf_in         = suffix in PDF_SUFFIXES
    is_video_in       = suffix in VIDEO_SUFFIXES
    is_audio_in       = suffix in AUDIO_SUFFIXES
    is_office_in      = suffix in OFFICE_SUFFIXES
    is_doc_in         = suffix in DOC_INPUT_SUFFIXES
    is_spreadsheet_in = suffix in SPREADSHEET_SUFFIXES
    is_subtitle_in    = suffix in SUBTITLE_SUFFIXES
    is_font_in        = suffix in FONT_SUFFIXES
    is_comic_in       = suffix in COMIC_SUFFIXES
    if is_comic_in and fmt in COMIC_TO_PDF:
        return _cbz_to_pdf(input_path, out_dir)
    if is_image_in and fmt in IMAGE_FORMATS:
        return _convert_image(input_path, fmt, out_dir, quality=quality)
    if is_image_in and fmt in IMAGE_TO_PDF:
        return _image_to_pdf(input_path, out_dir)
    if is_image_in and fmt in IMAGE_TO_TEXT:
        return _image_to_text(input_path, out_dir, job=job)
    if is_image_in and fmt in IMAGE_TO_NOBG:
        return _image_remove_bg(input_path, out_dir)
    if is_image_in and fmt in IMAGE_TO_SVG:
        return _image_to_svg(input_path, out_dir)
    if is_image_in and fmt in IMAGE_TO_DOC:
        return _image_to_doc(input_path, fmt, out_dir)
    if is_pdf_in and fmt in PDF_TO_IMAGE:
        return _pdf_to_images(input_path, fmt, out_dir)
    if is_pdf_in and fmt in PDF_TO_TEXT:
        return _pdf_to_text(input_path, out_dir)
    if is_pdf_in and fmt in PDF_TO_DOCX:
        return _pdf_to_docx(input_path, out_dir)
    if is_pdf_in and fmt in PDF_OCR_OUTPUTS:
        return _pdf_to_pdfa(input_path, out_dir)
    if is_pdf_in and fmt in PDF_TO_MD:
        return _pdf_to_markdown(input_path, out_dir)
    if is_pdf_in and fmt in PDF_TO_NOTES:
        return _pdf_to_notes(input_path, out_dir)
    if is_pdf_in and fmt in PDF_TO_DOC:
        return _pdf_to_doc(input_path, fmt, out_dir)
    if is_video_in and fmt == "png":
        return _video_to_png(input_path, out_dir)
    if is_video_in and fmt in VIDEO_TO_GIF:
        return _video_to_gif(input_path, out_dir)
    if is_video_in and fmt in VIDEO_TO_VIDEO:
        return _convert_video(input_path, fmt, out_dir, quality=quality)
    if (is_audio_in or is_video_in) and fmt in WHISPER_OUTPUTS:
        return _audio_video_to_transcript(input_path, fmt, out_dir, job=job)
    if (is_audio_in or is_video_in) and fmt == "txt":
        return _audio_video_to_transcript(input_path, "txt", out_dir, job=job)
    if (is_audio_in or is_video_in) and fmt in WHISPER_WORDS:
        return _audio_video_to_transcript(input_path, fmt, out_dir, job=job)
    if (is_audio_in or is_video_in) and fmt in TRANSCRIPT_KIT:
        return _transcript_kit(input_path, out_dir, job=job)
    if is_video_in and fmt in VIDEO_CAPTIONED:
        return _video_captioned(input_path, out_dir, job=job)
    if is_video_in and fmt in VIDEO_FRAMES:
        return _video_frames(input_path, out_dir)
    if is_video_in and fmt in VIDEO_SUBS:
        return _video_extract_subs(input_path, out_dir, job=job)
    if (is_audio_in or is_video_in) and fmt in {"pdf", "docx", "epub"}:
        return _notes_to_doc(input_path, fmt, out_dir, job=job)
    if is_audio_in and fmt in AUDIO_DENOISE:
        return _audio_denoise(input_path, out_dir)
    if is_audio_in and fmt in AUDIO_STEMS:
        return _audio_separate_stems(input_path, out_dir)
    # ffmpeg handles video containers transparently for audio extraction
    # (-vn) and waveform rendering, so video → audio/audiogram is the same call.
    if (is_audio_in or is_video_in) and fmt in AUDIO_TO_VIDEO:
        return _audio_to_audiogram(input_path, out_dir)
    if (is_audio_in or is_video_in) and fmt in AUDIO_FORMATS:
        return _convert_audio(input_path, fmt, out_dir)
    if is_office_in and fmt in OFFICE_TO_PDF:
        return _office_to_pdf(input_path, out_dir)
    if is_office_in and fmt in OFFICE_TO_IMAGE:
        return _office_to_images(input_path, fmt, out_dir)
    # docx/odt/pptx → md/html/rtf/epub/txt/odt/tex/etc. — pandoc reads these natively.
    if is_office_in and suffix in OFFICE_PANDOC_SOURCES and fmt in DOC_FORMATS:
        return _convert_doc(input_path, fmt, out_dir)
    # Any office (incl. xlsx/ppt/doc) → txt: chain office → pdf → text.
    if is_office_in and fmt in OFFICE_TO_TEXT:
        return _office_to_text(input_path, out_dir)
    # office → audiobook: chain office → pdf → text → TTS.
    if is_office_in and fmt in TTS_AUDIO_FORMATS:
        return _office_to_audio(input_path, fmt, out_dir)
    # pdf → audiobook: chain pdf → text → TTS.
    if is_pdf_in and fmt in TTS_AUDIO_FORMATS:
        return _pdf_to_audio(input_path, fmt, out_dir)
    # subtitle → audiobook: chain subtitle → text → TTS.
    if is_subtitle_in and fmt in TTS_AUDIO_FORMATS:
        return _subtitle_to_audio(input_path, fmt, out_dir)
    # office/pdf/subtitle → audiogram: chain through TTS, then waveform render.
    if is_office_in and fmt in AUDIO_TO_VIDEO:
        return _text_source_to_audiogram(input_path, out_dir, _office_to_audio)
    if is_pdf_in and fmt in AUDIO_TO_VIDEO:
        return _text_source_to_audiogram(input_path, out_dir, _pdf_to_audio)
    if is_subtitle_in and fmt in AUDIO_TO_VIDEO:
        return _text_source_to_audiogram(input_path, out_dir, _subtitle_to_audio)
    if suffix in TTS_INPUT_SUFFIXES and fmt in TTS_AUDIO_FORMATS:
        return _text_to_audio(input_path, fmt, out_dir)
    if is_doc_in and fmt in DOC_TO_PDF:
        return _doc_to_pdf(input_path, out_dir)
    if fmt in DOC_FORMATS and is_doc_in:
        return _convert_doc(input_path, fmt, out_dir)
    if is_spreadsheet_in and (fmt in SPREADSHEET_FORMATS or fmt in SPREADSHEET_TO_TEXT or fmt in SPREADSHEET_TO_PDF):
        return _convert_spreadsheet(input_path, fmt, out_dir)
    if is_subtitle_in and fmt in SUBTITLE_FORMATS:
        return _convert_subtitle(input_path, fmt, out_dir)
    if is_subtitle_in and fmt in SUBTITLE_TO_TEXT:
        return _subtitle_to_text(input_path, fmt, out_dir)
    if is_subtitle_in and fmt in SUBTITLE_TO_DOC:
        return _subtitle_to_doc(input_path, fmt, out_dir)
    if is_font_in and fmt in FONT_FORMATS:
        return _convert_font(input_path, fmt, out_dir)
    if is_font_in and fmt in FONT_TO_IMAGE:
        return _font_to_image(input_path, fmt, out_dir)
    from .graph import shortest_path
    hop = shortest_path(suffix, fmt)
    if hop and len(hop) > 1:
        current = input_path
        for hop_fmt in hop:
            current = _convert_one(current, hop_fmt, out_dir, quality=quality, job=job)
        return current
    raise ValueError(f"no bridge: {suffix or '(no ext)'} → {fmt}")


# Output formats that can be meaningfully merged into one file.
# Text-like → concat with separator. PDF → pypdf merge. Audio/video → ffmpeg concat.
# Subtitles → pysubs2 with timestamp shifting. xlsx → multi-sheet workbook.
# Images → tile grid into one larger image.
TEXT_MERGE_FORMATS = {"txt", "md", "markdown", "csv", "tsv", "json", "html", "htm",
                      "rtf", "tex", "org", "rst", "notes", "yaml", "yml", "toml"}
SUBTITLE_MERGE_FORMATS = {"srt", "vtt", "ass", "ssa", "sub", "sbv"}
AV_MERGE_FORMATS = {"mp3", "m4a", "wav", "flac", "ogg", "opus", "aac",
                    "mp4", "mov", "webm", "mkv"}
IMAGE_MERGE_FORMATS = {"jpeg", "jpg", "png", "webp", "avif", "tiff", "bmp"}
SHEET_MERGE_FORMATS = {"xlsx"}
MERGEABLE_FORMATS = (TEXT_MERGE_FORMATS | SUBTITLE_MERGE_FORMATS | AV_MERGE_FORMATS
                     | IMAGE_MERGE_FORMATS | SHEET_MERGE_FORMATS | {"pdf"})


def _merge_outputs(paths: list[Path], fmt: str, out_dir: Path) -> Path:
    """Merge N converted outputs of the same format into a single file."""
    if not paths:
        raise RuntimeError("nothing to merge")
    if fmt == "pdf":
        return _pdf_merge(paths, out_dir)
    if fmt == "json":
        # Always produce a valid JSON array. If each file is itself an array,
        # the natural merge is element-wise concat; otherwise wrap each as an
        # element. Mixed shapes degrade to wrapping.
        import json as json_mod
        out_path = out_dir / "merged.json"
        items: list = []
        for p in paths:
            obj = json_mod.loads(p.read_text(encoding="utf-8"))
            if isinstance(obj, list):
                items.extend(obj)
            else:
                items.append(obj)
        out_path.write_text(json_mod.dumps(items, indent=2, ensure_ascii=False), encoding="utf-8")
        return out_path
    if fmt == "xlsx":
        # Multi-sheet workbook: each input file becomes its own sheet.
        # Sheet names use the input filename (truncated to xlsx's 31-char cap).
        import openpyxl
        wb = openpyxl.Workbook()
        wb.remove(wb.active)  # drop default empty sheet
        seen: set[str] = set()
        for p in paths:
            src_wb = openpyxl.load_workbook(p, read_only=True, data_only=True)
            for ws_idx, src_ws in enumerate(src_wb.worksheets):
                base = p.stem.removeprefix("input_")
                if len(src_wb.worksheets) > 1:
                    base = f"{base}_{src_ws.title}"
                # xlsx sheet names: ≤31 chars, no []:*?/\
                name = "".join(c if c not in r"[]:*?/\\" else "_" for c in base)[:31] or "sheet"
                # Dedupe: append _2, _3 if collision
                final = name
                n = 2
                while final in seen:
                    suffix = f"_{n}"
                    final = name[:31 - len(suffix)] + suffix
                    n += 1
                seen.add(final)
                new_ws = wb.create_sheet(final)
                for row in src_ws.iter_rows(values_only=True):
                    new_ws.append(list(row))
        out_path = out_dir / "merged.xlsx"
        wb.save(out_path)
        return out_path
    if fmt in SUBTITLE_MERGE_FORMATS:
        # pysubs2 merge with cumulative time-shift so file 2's events start
        # after file 1's last cue ends. Otherwise concatenated subtitles all
        # collide at 00:00:00 and the last one wins.
        import pysubs2
        merged = pysubs2.SSAFile()
        offset_ms = 0
        for p in paths:
            subs = pysubs2.load(str(p))
            for ev in subs.events:
                ev2 = ev.copy()
                ev2.start += offset_ms
                ev2.end += offset_ms
                merged.events.append(ev2)
            if subs.events:
                offset_ms = max(offset_ms, max(ev.end for ev in merged.events))
        out_path = out_dir / f"merged.{fmt}"
        merged.save(str(out_path), format_=fmt)
        return out_path
    if fmt in IMAGE_MERGE_FORMATS:
        # Tile N images in a roughly-square grid: cols = ceil(sqrt(N)).
        # Each cell is sized to the max input dimension in each axis; smaller
        # images are letterboxed (centered with padding) so nothing is cropped.
        import math
        n = len(paths)
        cols = int(math.ceil(math.sqrt(n)))
        rows = int(math.ceil(n / cols))
        imgs = [_open_raster(p) for p in paths]
        # Convert to RGB once so we have a consistent mode (alpha not preserved
        # for jpeg/avif; png/webp keep transparency by switching to RGBA).
        keep_alpha = fmt in {"png", "webp", "tiff"}
        target_mode = "RGBA" if keep_alpha else "RGB"
        normalized = []
        for img in imgs:
            if img.mode != target_mode:
                if target_mode == "RGB" and img.mode in ("RGBA", "LA", "P"):
                    bg = Image.new("RGB", img.size, (255, 255, 255))
                    bg.paste(img.convert("RGBA"), mask=img.convert("RGBA").split()[3])
                    img = bg
                else:
                    img = img.convert(target_mode)
            normalized.append(img)
        cell_w = max(i.width for i in normalized)
        cell_h = max(i.height for i in normalized)
        bg_color = (0, 0, 0, 0) if keep_alpha else (255, 255, 255)
        canvas = Image.new(target_mode, (cell_w * cols, cell_h * rows), bg_color)
        for idx, img in enumerate(normalized):
            r, c = divmod(idx, cols)
            x = c * cell_w + (cell_w - img.width) // 2
            y = r * cell_h + (cell_h - img.height) // 2
            canvas.paste(img, (x, y))
        save_fmt = "JPEG" if fmt in ("jpeg", "jpg") else fmt.upper()
        out_path = out_dir / f"merged.{fmt}"
        save_kwargs = {}
        if fmt in ("jpeg", "jpg"):
            save_kwargs.update({"quality": 92, "optimize": True, "progressive": True})
        elif fmt == "webp":
            save_kwargs.update({"quality": 92, "method": 6})
        elif fmt == "avif":
            save_kwargs.update({"quality": 80})
        canvas.save(out_path, save_fmt, **save_kwargs)
        return out_path
    if fmt in {"csv", "tsv"}:
        # Keep the header from file 1, drop the header (first row) from files 2..N
        # if every file has the same column count — typical multi-export merge.
        import csv as csv_mod
        delim = "\t" if fmt == "tsv" else ","
        loaded = []
        for p in paths:
            with p.open("r", encoding="utf-8-sig", newline="") as f:
                loaded.append(list(csv_mod.reader(f, delimiter=delim)))
        same_width = len({len(rs[0]) for rs in loaded if rs}) == 1
        merged_rows: list[list] = []
        for i, rs in enumerate(loaded):
            if not rs:
                continue
            merged_rows.extend(rs if i == 0 or not same_width else rs[1:])
        out_path = out_dir / f"merged.{fmt}"
        with out_path.open("w", encoding="utf-8", newline="") as f:
            csv_mod.writer(f, delimiter=delim).writerows(merged_rows)
        return out_path
    if fmt in TEXT_MERGE_FORMATS:
        out_path = out_dir / f"merged.{fmt}"
        chunks = []
        for p in paths:
            label = p.name
            try:
                body = p.read_text(encoding="utf-8")
            except UnicodeDecodeError:
                body = p.read_text(encoding="utf-8", errors="replace")
            chunks.append(f"--- {label} ---\n{body.rstrip()}\n")
        out_path.write_text("\n".join(chunks), encoding="utf-8")
        return out_path
    if fmt in AV_MERGE_FORMATS:
        # ffmpeg concat demuxer requires same codec/sample rate. Since these
        # files just came from the same conversion target, that should hold.
        list_path = out_dir / "_concat.txt"
        list_path.write_text(
            "\n".join(f"file '{p.as_posix()}'" for p in paths),
            encoding="utf-8",
        )
        out_path = out_dir / f"merged.{fmt}"
        cmd = ["ffmpeg", "-y", "-f", "concat", "-safe", "0",
               "-i", str(list_path), "-c", "copy", str(out_path)]
        r = subprocess.run(cmd, capture_output=True, text=True)
        list_path.unlink(missing_ok=True)
        if r.returncode != 0:
            # Re-encode fallback for cases where -c copy fails (codec mismatch
            # at container level — rare here but possible with VBR audio).
            cmd_re = ["ffmpeg", "-y", "-f", "concat", "-safe", "0",
                      "-i", str(list_path), str(out_path)]
            list_path.write_text(
                "\n".join(f"file '{p.as_posix()}'" for p in paths),
                encoding="utf-8",
            )
            r = subprocess.run(cmd_re, capture_output=True, text=True)
            list_path.unlink(missing_ok=True)
            if r.returncode != 0:
                raise RuntimeError(f"ffmpeg concat failed: {r.stderr.strip()[-400:]}")
        return out_path
    raise ValueError(f"format not mergeable: {fmt}")


def _shrink_one(input_path: Path, level: str, out_dir: Path,
                quality: int | None = None, max_dim: int | None = None,
                scale_percent: float | None = None,
                target_byte: int | None = None) -> Path:
    """Single-file shrink dispatch. Mirrors _convert_one's role for the
    batch-shrink endpoint. PDF currently ignores max_dim/scale_percent
    (Ghostscript image-DPI tuning is a separate concern)."""
    suffix = input_path.suffix.lower()
    if suffix in IMAGE_SUFFIXES:
        return _shrink_image(input_path, level, out_dir, quality=quality,
                             max_dim=max_dim, scale_percent=scale_percent,
                             target_byte=target_byte)
    if suffix in VIDEO_SUFFIXES:
        return _shrink_video(input_path, level, out_dir, quality=quality,
                             max_dim=max_dim, scale_percent=scale_percent,
                             target_byte=target_byte)
    if suffix in PDF_SUFFIXES:
        return _shrink_pdf(input_path, level, out_dir, quality=quality,
                           target_byte=target_byte)
    raise ValueError(f"unsupported file type for shrink: {suffix or '(no extension)'}")


def run_batch_shrink_job(job_id: str, input_paths: list[Path], level: str,
                         downloads_dir: Path, quality: int | None = None,
                         max_dim: int | None = None,
                         scale_percent: float | None = None,
                         target_byte: int | None = None) -> None:
    """Apply the same shrink/resize params to N homogeneous files. Zips the
    outputs (no merge mode — combining N shrunk images/videos doesn't make
    sense). Per-file errors abort the whole job."""
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "shrinking"
        outputs: list[Path] = []
        total = len(input_paths)
        for i, p in enumerate(input_paths, 1):
            outputs.append(_shrink_one(p, level, out_dir, quality=quality,
                                       max_dim=max_dim, scale_percent=scale_percent,
                                       target_byte=target_byte))
            job["progress"] = max(1, min(99, int(100 * i / total)))
        zip_name = f"{'resized' if level == 'resize' else 'shrunk'}_{total}.zip"
        target = out_dir / zip_name
        with zipfile.ZipFile(target, "w", zipfile.ZIP_STORED) as zf:
            for o in outputs:
                zf.write(o, arcname=o.name)
        for o in outputs:
            o.unlink(missing_ok=True)
        for p in input_paths:
            p.unlink(missing_ok=True)
        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = str(e)


def run_batch_convert_job(job_id: str, input_paths: list[Path], fmt: str,
                          downloads_dir: Path, merge: bool = False,
                          quality: int | None = None, language: str | None = None,
                          translate_to: str | None = None) -> None:
    """Convert N homogeneous files to fmt. If merge=True, return one merged file;
    otherwise zip the N outputs. Per-file errors abort the whole job."""
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "converting"
        if translate_to:
            job["translate_to"] = translate_to
        outputs: list[Path] = []
        total = len(input_paths)
        with use_language(language), use_translate_to(translate_to):
            for i, p in enumerate(input_paths, 1):
                outputs.append(_convert_one(p, fmt, out_dir, quality=quality, job=job))
                job["progress"] = max(1, min(99, int(100 * i / total)))
        if merge:
            merged = _merge_outputs(outputs, fmt, out_dir)
            for o in outputs:
                if o != merged:
                    o.unlink(missing_ok=True)
            target = merged
        else:
            zip_name = f"converted_{total}_{fmt}.zip"
            target = out_dir / zip_name
            with zipfile.ZipFile(target, "w", zipfile.ZIP_STORED) as zf:
                for o in outputs:
                    zf.write(o, arcname=o.name)
            for o in outputs:
                o.unlink(missing_ok=True)
        for p in input_paths:
            p.unlink(missing_ok=True)
        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = str(e)


def run_convert_job(job_id: str, input_path: Path, fmt: str, downloads_dir: Path,
                    quality: int | None = None, language: str | None = None,
                    translate_to: str | None = None) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "converting"
        if translate_to:
            job["translate_to"] = translate_to
        with use_language(language), use_translate_to(translate_to):
            target = _convert_one(input_path, fmt, out_dir, quality=quality, job=job)
        input_path.unlink(missing_ok=True)
        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)


def run_job(job_id: str, url: str, fmt: str, downloads_dir: Path,
            language: str | None = None, translate_to: str | None = None) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    out_dir.mkdir(parents=True, exist_ok=True)

    try:
        from .ssrf import refuse_ssrf
        refuse_ssrf(url)
        _require_network("URL extraction")
        if translate_to:
            job["translate_to"] = translate_to
        with use_language(language), use_translate_to(translate_to):
            if fmt == "images":
                title = _run_gallerydl(job, url, out_dir)
                job["title"] = title
                image_files = [p for p in out_dir.rglob("*") if p.is_file()]
                if not image_files:
                    raise RuntimeError("gallery-dl produced no files")
                if len(image_files) == 1:
                    target = image_files[0]
                else:
                    target = _zip_images(out_dir, title)
            elif fmt in {"txt", "srt", "vtt", "notes", "words", "kit", "translate"}:
                # URL → audio (mp3) → transcript via Whisper.
                title = _run_ytdlp(job, url, "mp3", out_dir)
                job["title"] = title
                audio = _pick_single_file(out_dir, prefer_ext="mp3")
                if audio is None:
                    raise RuntimeError("yt-dlp produced no audio")
                job["progress"] = 0
                job["status"] = "transcribing"
                if fmt == "kit":
                    target = _transcript_kit(audio, out_dir, job=job)
                elif fmt == "translate":
                    target = _to_translated_txt(audio, out_dir, job=job)
                else:
                    target = _audio_video_to_transcript(audio, fmt, out_dir, job=job)
                audio.unlink(missing_ok=True)
            elif fmt in {"pdf", "docx", "epub"}:
                title = _run_ytdlp(job, url, "mp3", out_dir)
                job["title"] = title
                audio = _pick_single_file(out_dir, prefer_ext="mp3")
                if audio is None:
                    raise RuntimeError("yt-dlp produced no audio")
                job["progress"] = 0
                job["status"] = "transcribing"
                target = _notes_to_doc(audio, fmt, out_dir, job=job)
                audio.unlink(missing_ok=True)
            elif fmt in {"captioned", "frames", "subs"}:
                title = _run_ytdlp(job, url, "mp4", out_dir)
                job["title"] = title
                video = _pick_single_file(out_dir, prefer_ext="mp4")
                if video is None:
                    raise RuntimeError("yt-dlp produced no video")
                job["progress"] = 0
                job["status"] = "converting"
                if fmt == "captioned":
                    target = _video_captioned(video, out_dir, job=job)
                elif fmt == "frames":
                    target = _video_frames(video, out_dir)
                else:
                    target = _video_extract_subs(video, out_dir, job=job)
                video.unlink(missing_ok=True)
            else:
                title = _run_ytdlp(job, url, fmt, out_dir)
                job["title"] = title
                target = _pick_single_file(out_dir, prefer_ext=fmt)
                if target is None:
                    raise RuntimeError("yt-dlp produced no file")

        mark_ready(job, target)
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)
        shutil.rmtree(out_dir, ignore_errors=True)
