import os
import shutil
import subprocess
import sys
import tarfile
import zipfile
from pathlib import Path

import pdf2image
import pillow_heif
import py7zr
import yt_dlp
import zstandard as zstd
from gallery_dl import config as gdl_config, job as gdl_job
from PIL import Image, ImageOps

pillow_heif.register_heif_opener()

# Detect curl_cffi at import — yt-dlp uses it for browser impersonation,
# which is the only thing that gets past TikTok's datacenter-IP block.
try:
    import curl_cffi  # noqa: F401
    _HAS_IMPERSONATE = True
except ImportError:
    _HAS_IMPERSONATE = False

jobs: dict[str, dict] = {}


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
    if "http error 404" in e or "not found" in e:
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

# Spreadsheets via openpyxl + stdlib csv/json
SPREADSHEET_FORMATS = {"csv", "tsv", "xlsx", "json"}
SPREADSHEET_SUFFIXES = {f".{f}" for f in SPREADSHEET_FORMATS}

# Subtitles via pysubs2
SUBTITLE_FORMATS = {"srt", "vtt", "ass", "ssa", "sub", "sbv"}
SUBTITLE_SUFFIXES = {f".{f}" for f in SUBTITLE_FORMATS}

# Fonts via fonttools (TTF/OTF ↔ WOFF/WOFF2; TTF↔OTF is a no-op container swap)
FONT_FORMATS = {"ttf", "otf", "woff", "woff2"}
FONT_SUFFIXES = {f".{f}" for f in FONT_FORMATS}

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

SHRINK_LEVELS = {"low", "medium", "high"}
# Pillow-readable image inputs. PSD/TGA/PCX/EPS/ICNS rely on Pillow's broader
# format support; they get the same dispatch path as JPEG/PNG.
IMAGE_SUFFIXES = {
    ".jpg", ".jpeg", ".png", ".webp", ".avif", ".heic", ".heif",
    ".tiff", ".bmp", ".gif", ".ico", ".jp2",
    ".psd", ".tga", ".pcx", ".eps", ".icns",
}
VIDEO_SUFFIXES = {".mp4", ".mov", ".webm", ".mkv", ".avi", ".m4v",
                  ".flv", ".wmv", ".3gp", ".ts", ".mts", ".m2ts", ".ogv"}
AUDIO_SUFFIXES = {".mp3", ".m4a", ".wav", ".flac", ".ogg", ".opus", ".aac",
                  ".aiff", ".aif", ".wma"}
PDF_SUFFIXES = {".pdf"}
OFFICE_SUFFIXES = {".docx", ".doc", ".pptx", ".ppt", ".xlsx", ".xls", ".odp", ".ods", ".odt"}

# Audio transcoding (ffmpeg)
AUDIO_FORMATS = {"mp3", "m4a", "wav", "flac", "ogg", "opus", "aac"}
AUDIO_FFMPEG_OPTS: dict[str, list[str]] = {
    "mp3":  ["-c:a", "libmp3lame", "-b:a", "192k"],
    "m4a":  ["-c:a", "aac", "-b:a", "192k"],
    "wav":  ["-c:a", "pcm_s16le"],
    "flac": ["-c:a", "flac"],
    "ogg":  ["-c:a", "libvorbis", "-q:a", "5"],
    "opus": ["-c:a", "libopus", "-b:a", "128k"],
    "aac":  ["-c:a", "aac", "-b:a", "192k"],
}

# Office to PDF via LibreOffice headless
OFFICE_TO_PDF = {"pdf"}

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
# Default model: distil-large-v3 (~6x faster than whisper-large-v3, ≤1% WER drop, multilingual)
WHISPER_OUTPUTS = {"srt", "vtt", "notes"}
# Note: "txt" is handled here too when input is audio/video, but we don't add it
# to the set since the dispatch checks input type first; pdf→txt and image→txt
# stay routed to their own handlers.

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
    # yt-dlp's Python API needs an ImpersonateTarget object, not a bare string.
    if _HAS_IMPERSONATE:
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


def _convert_image(input_path: Path, target_fmt: str, out_dir: Path, quality: int | None = None) -> Path:
    img = Image.open(input_path)
    img = ImageOps.exif_transpose(img)  # honor camera-rotation EXIF tag
    img.load()
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


def _convert_spreadsheet(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """Cross-convert tabular data: csv ↔ tsv ↔ xlsx ↔ json. Reads first sheet
    of multi-sheet xlsx. JSON must be a list of dicts (header row) or list of
    lists (raw rows). Numbers and strings preserved; dates → ISO strings."""
    import csv as csv_mod
    import json as json_mod
    import openpyxl
    from datetime import date, datetime
    src = input_path.suffix.lower()
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"

    # --- read input into rows: list[list[Any]] ---
    if src in (".csv", ".tsv"):
        delim = "\t" if src == ".tsv" else ","
        with input_path.open("r", encoding="utf-8-sig", newline="") as f:
            rows = list(csv_mod.reader(f, delimiter=delim))
    elif src == ".xlsx":
        wb = openpyxl.load_workbook(input_path, read_only=True, data_only=True)
        ws = wb.active
        rows = [[cell.value for cell in row] for row in ws.iter_rows()]
    elif src == ".json":
        data = json_mod.loads(input_path.read_text(encoding="utf-8"))
        if isinstance(data, list) and data and isinstance(data[0], dict):
            keys = list(data[0].keys())
            rows = [keys] + [[d.get(k, "") for k in keys] for d in data]
        elif isinstance(data, list):
            rows = data if not data or isinstance(data[0], list) else [[v] for v in data]
        else:
            raise RuntimeError("JSON must be a list of dicts or list of lists for tabular conversion")
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
        if rows and len(rows) > 1:
            keys = [str(k) for k in rows[0]]
            data = [{k: _stringify(v) for k, v in zip(keys, r)} for r in rows[1:]]
        else:
            data = [list(r) for r in rows]
        out_path.write_text(json_mod.dumps(data, indent=2, default=str), encoding="utf-8")
    return out_path


def _convert_subtitle(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    """Cross-convert subtitle formats: srt ↔ vtt ↔ ass ↔ ssa ↔ sub ↔ sbv."""
    import pysubs2
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"
    subs = pysubs2.load(str(input_path))
    subs.save(str(out_path), format_=target_fmt)
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
    writer = PANDOC_WRITER.get(target_fmt, target_fmt)
    cmd = ["pandoc", str(input_path), "-o", str(out_path), "-t", writer]
    if target_fmt == "epub":
        cmd.extend(["--metadata", f"title={stem}"])
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=120)
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


def _shrink_image(input_path: Path, level: str, out_dir: Path, quality: int | None = None) -> Path:
    if quality is None:
        quality = {"low": 50, "medium": 70, "high": 85}[level]
    quality = max(1, min(100, quality))
    img = Image.open(input_path)
    src_fmt = (img.format or "JPEG").lower()  # capture before exif_transpose strips it
    img = ImageOps.exif_transpose(img)
    img.load()
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
    img.save(out_path, format=out_fmt.upper(), **save_kwargs)
    return out_path


def _shrink_video(input_path: Path, level: str, out_dir: Path, quality: int | None = None) -> Path:
    # SVT-AV1 yields ~30-40% smaller files than x264 at matched quality.
    # CRF: lower = higher quality. AV1 needs slightly higher CRF than x264 for
    # equivalent perceived quality.
    if quality is not None:
        # Map UI's 1-100 quality slider to SVT-AV1's CRF (0-63, lower = better).
        # quality=100 → CRF 18 (visually lossless), quality=1 → CRF 60.
        q = max(1, min(100, quality))
        crf = str(int(round(60 - (q / 100) * 42)))
        settings = {"crf": crf, "preset": "6"}
    else:
        settings = {
            "low":    {"crf": "45", "preset": "8"},   # smallest, fastest encode
            "medium": {"crf": "35", "preset": "6"},   # SVT-AV1's recommended balance
            "high":   {"crf": "28", "preset": "4"},   # best quality, slower encode
        }[level]
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_small.mp4"
    cmd = [
        "ffmpeg", "-y", "-i", str(input_path),
        "-c:v", "libsvtav1", "-crf", settings["crf"], "-preset", settings["preset"],
        "-c:a", "aac", "-b:a", "96k",
        "-movflags", "+faststart",
        str(out_path),
    ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=900)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg failed: {r.stderr.strip()[-500:]}")
    return out_path


def _shrink_pdf(input_path: Path, level: str, out_dir: Path, quality: int | None = None) -> Path:
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
    return out_path


def run_shrink_job(job_id: str, input_path: Path, level: str, downloads_dir: Path, quality: int | None = None) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "shrinking"
        suffix = input_path.suffix.lower()
        if suffix in IMAGE_SUFFIXES:
            target = _shrink_image(input_path, level, out_dir, quality=quality)
        elif suffix in VIDEO_SUFFIXES:
            target = _shrink_video(input_path, level, out_dir, quality=quality)
        elif suffix in PDF_SUFFIXES:
            target = _shrink_pdf(input_path, level, out_dir, quality=quality)
        else:
            raise ValueError(f"unsupported file type for shrink: {suffix or '(no extension)'}")
        input_path.unlink(missing_ok=True)
        job["filename"] = target.name
        job["output_size"] = target.stat().st_size
        job["progress"] = 100
        job["status"] = "ready"
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
        job["filename"] = target.name
        job["output_size"] = target.stat().st_size
        job["progress"] = 100
        job["status"] = "ready"
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)


BATCH_IMAGE_FORMATS = {"pdf", "jpeg", "png", "webp", "avif", "gif", "mp4"}


def _open_oriented(path: Path, rotations: dict[str, int] | None = None) -> Image.Image:
    """Open image, apply EXIF orientation, then user-applied rotation."""
    img = Image.open(path)
    img = ImageOps.exif_transpose(img)
    img.load()
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
        job["filename"] = target.name
        job["output_size"] = target.stat().st_size
        job["progress"] = 100
        job["status"] = "ready"
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
        img = Image.open(p)
        img = ImageOps.exif_transpose(img)
        img.load()
        # User-applied rotation. PIL rotates CCW by default; negate for CW intent.
        original_name = p.name.removeprefix("input_")
        deg = int(rotations.get(original_name, 0)) % 360
        if deg:
            img = img.rotate(-deg, expand=True)
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
        job["filename"] = target.name
        job["output_size"] = target.stat().st_size
        job["progress"] = 100
        job["status"] = "ready"
    except Exception as e:
        job["status"] = "error"
        job["error"] = str(e)


def _image_to_pdf(input_path: Path, out_dir: Path) -> Path:
    img = Image.open(input_path)
    img = ImageOps.exif_transpose(img)
    img.load()
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
        out_path.write_text(text, encoding="utf-8")
        return out_path
    # Empty result → likely a scanned PDF. OCR each page.
    import pytesseract
    pages = pdf2image.convert_from_path(str(input_path), dpi=300)
    ocr_parts = [pytesseract.image_to_string(pg).strip() for pg in pages]
    out_path.write_text("\n\n".join(ocr_parts), encoding="utf-8")
    return out_path


def _image_remove_bg(input_path: Path, out_dir: Path) -> Path:
    from rembg import remove
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_nobg.png"
    in_bytes = input_path.read_bytes()
    out_path.write_bytes(remove(in_bytes))
    return out_path


def _image_to_svg(input_path: Path, out_dir: Path) -> Path:
    import vtracer
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.svg"
    vtracer.convert_image_to_svg_py(str(input_path), str(out_path))
    return out_path


def _image_to_text(input_path: Path, out_dir: Path) -> Path:
    import pytesseract
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.txt"
    img = Image.open(input_path)
    img = ImageOps.exif_transpose(img)  # OCR also benefits from rotation correction
    img.load()
    text = pytesseract.image_to_string(img)
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
        job["filename"] = target.name
        job["progress"] = 100
        job["status"] = "ready"
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
        job["filename"] = target.name
        job["progress"] = 100
        job["status"] = "ready"
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

    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"

    # Choose synthesizer: macOS `say` if available, else piper-tts.
    if sys.platform == "darwin" and shutil.which("say"):
        raw = out_dir / "_tts.aiff"
        r = subprocess.run(
            ["say", "-o", str(raw), text],
            capture_output=True, text=True, timeout=600,
        )
        if r.returncode != 0:
            raise RuntimeError(f"say failed: {r.stderr.strip()[-500:]}")
    else:
        voice = os.environ.get("EXPORT_PIPER_VOICE", "")
        if not voice or not Path(voice).exists():
            raise RuntimeError(
                "TTS unavailable: install piper-tts and set "
                "EXPORT_PIPER_VOICE to a .onnx voice path"
            )
        if not shutil.which("piper"):
            raise RuntimeError("piper binary not in PATH (pip install piper-tts)")
        raw = out_dir / "_tts.wav"
        r = subprocess.run(
            ["piper", "-m", voice, "-f", str(raw)],
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


def _audio_video_to_transcript(input_path: Path, target_fmt: str, out_dir: Path) -> Path:
    from faster_whisper import WhisperModel
    name = os.environ.get("EXPORT_WHISPER_MODEL", "distil-large-v3")
    if _whisper_state["model"] is None or _whisper_state["name"] != name:
        download_root = os.environ.get("EXPORT_WHISPER_MODEL_DIR") or None
        _whisper_state["model"] = WhisperModel(
            name, device="cpu", compute_type="int8", download_root=download_root,
        )
        _whisper_state["name"] = name
    model = _whisper_state["model"]

    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}.{target_fmt}"
    segments_iter, _info = model.transcribe(str(input_path), beam_size=1)
    segments = list(segments_iter)

    if target_fmt == "txt":
        out_path.write_text(
            "\n".join(s.text.strip() for s in segments) + "\n",
            encoding="utf-8",
        )
    elif target_fmt == "notes":
        # Group segments into paragraphs by pause length (>1.5s gap = new paragraph).
        # NoteGPT-style: clean readable markdown, no timestamps cluttering the text.
        paragraphs: list[list[str]] = [[]]
        last_end = 0.0
        for s in segments:
            if s.start - last_end > 1.5 and paragraphs[-1]:
                paragraphs.append([])
            paragraphs[-1].append(s.text.strip())
            last_end = s.end
        body = "\n\n".join(" ".join(p) for p in paragraphs if p)
        title = stem.replace("_", " ").replace("-", " ").strip()
        # Out path uses .md extension since it's clean markdown.
        out_path = out_dir / f"{stem}_notes.md"
        out_path.write_text(
            f"# {title}\n\n## Transcript\n\n{body}\n",
            encoding="utf-8",
        )
    elif target_fmt == "srt":
        lines: list[str] = []
        for i, s in enumerate(segments, 1):
            lines += [str(i),
                      f"{_format_ts(s.start, True)} --> {_format_ts(s.end, True)}",
                      s.text.strip(), ""]
        out_path.write_text("\n".join(lines), encoding="utf-8")
    elif target_fmt == "vtt":
        lines = ["WEBVTT", ""]
        for s in segments:
            lines += [f"{_format_ts(s.start, False)} --> {_format_ts(s.end, False)}",
                      s.text.strip(), ""]
        out_path.write_text("\n".join(lines), encoding="utf-8")
    else:
        raise ValueError(f"unsupported transcript format: {target_fmt}")
    return out_path


def _audio_denoise(input_path: Path, out_dir: Path) -> Path:
    import noisereduce as nr
    import soundfile as sf
    stem = input_path.stem.removeprefix("input_")
    out_path = out_dir / f"{stem}_denoised.wav"
    data, sr = sf.read(str(input_path))
    cleaned = nr.reduce_noise(y=data, sr=sr, stationary=True)
    sf.write(str(out_path), cleaned, sr)
    return out_path


def _audio_separate_stems(input_path: Path, out_dir: Path) -> Path:
    """Run Demucs htdemucs to split audio into vocals/drums/bass/other, then zip the stems."""
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
    cmd = ["ffmpeg", "-y", "-i", str(input_path), "-vn"] + AUDIO_FFMPEG_OPTS[target_fmt] + [str(out_path)]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=600)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg audio failed: {r.stderr.strip()[-500:]}")
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


def run_convert_job(job_id: str, input_path: Path, fmt: str, downloads_dir: Path, quality: int | None = None) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "converting"
        suffix = input_path.suffix.lower()
        is_image_in       = suffix in IMAGE_SUFFIXES
        is_pdf_in         = suffix in PDF_SUFFIXES
        is_video_in       = suffix in VIDEO_SUFFIXES
        is_audio_in       = suffix in AUDIO_SUFFIXES
        is_office_in      = suffix in OFFICE_SUFFIXES
        is_doc_in         = suffix in DOC_INPUT_SUFFIXES
        is_spreadsheet_in = suffix in SPREADSHEET_SUFFIXES
        is_subtitle_in    = suffix in SUBTITLE_SUFFIXES
        is_font_in        = suffix in FONT_SUFFIXES
        if is_image_in and fmt in IMAGE_FORMATS:
            target = _convert_image(input_path, fmt, out_dir, quality=quality)
        elif is_image_in and fmt in IMAGE_TO_PDF:
            target = _image_to_pdf(input_path, out_dir)
        elif is_image_in and fmt in IMAGE_TO_TEXT:
            target = _image_to_text(input_path, out_dir)
        elif is_image_in and fmt in IMAGE_TO_NOBG:
            target = _image_remove_bg(input_path, out_dir)
        elif is_image_in and fmt in IMAGE_TO_SVG:
            target = _image_to_svg(input_path, out_dir)
        elif is_pdf_in and fmt in PDF_TO_IMAGE:
            target = _pdf_to_images(input_path, fmt, out_dir)
        elif is_pdf_in and fmt in PDF_TO_TEXT:
            target = _pdf_to_text(input_path, out_dir)
        elif is_pdf_in and fmt in PDF_TO_DOCX:
            target = _pdf_to_docx(input_path, out_dir)
        elif is_pdf_in and fmt in PDF_OCR_OUTPUTS:
            target = _pdf_to_pdfa(input_path, out_dir)
        elif is_video_in and fmt in VIDEO_TO_GIF:
            target = _video_to_gif(input_path, out_dir)
        elif is_video_in and fmt in VIDEO_TO_VIDEO:
            target = _convert_video(input_path, fmt, out_dir, quality=quality)
        elif (is_audio_in or is_video_in) and fmt in WHISPER_OUTPUTS:
            target = _audio_video_to_transcript(input_path, fmt, out_dir)
        elif (is_audio_in or is_video_in) and fmt == "txt":
            target = _audio_video_to_transcript(input_path, "txt", out_dir)
        elif is_audio_in and fmt in AUDIO_DENOISE:
            target = _audio_denoise(input_path, out_dir)
        elif is_audio_in and fmt in AUDIO_STEMS:
            target = _audio_separate_stems(input_path, out_dir)
        elif is_audio_in and fmt in AUDIO_FORMATS:
            target = _convert_audio(input_path, fmt, out_dir)
        elif is_office_in and fmt in OFFICE_TO_PDF:
            target = _office_to_pdf(input_path, out_dir)
        elif suffix in TTS_INPUT_SUFFIXES and fmt in TTS_AUDIO_FORMATS:
            target = _text_to_audio(input_path, fmt, out_dir)
        elif is_doc_in and fmt in DOC_TO_PDF:
            target = _doc_to_pdf(input_path, out_dir)
        elif fmt in DOC_FORMATS and is_doc_in:
            target = _convert_doc(input_path, fmt, out_dir)
        elif is_spreadsheet_in and fmt in SPREADSHEET_FORMATS:
            target = _convert_spreadsheet(input_path, fmt, out_dir)
        elif is_subtitle_in and fmt in SUBTITLE_FORMATS:
            target = _convert_subtitle(input_path, fmt, out_dir)
        elif is_font_in and fmt in FONT_FORMATS:
            target = _convert_font(input_path, fmt, out_dir)
        else:
            raise ValueError(f"no bridge: {suffix or '(no ext)'} → {fmt}")
        input_path.unlink(missing_ok=True)
        job["filename"] = target.name
        job["progress"] = 100
        job["status"] = "ready"
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)


def run_job(job_id: str, url: str, fmt: str, downloads_dir: Path) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    out_dir.mkdir(parents=True, exist_ok=True)

    try:
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
        elif fmt in {"txt", "srt", "vtt", "notes"}:
            # URL → audio (mp3) → transcript via Whisper.
            title = _run_ytdlp(job, url, "mp3", out_dir)
            job["title"] = title
            audio = _pick_single_file(out_dir, prefer_ext="mp3")
            if audio is None:
                raise RuntimeError("yt-dlp produced no audio")
            # Reset progress so the bar doesn't look "done" during the slow Whisper pass
            job["progress"] = 0
            job["status"] = "transcribing"
            target = _audio_video_to_transcript(audio, fmt, out_dir)
            audio.unlink(missing_ok=True)  # drop intermediate mp3
        else:
            title = _run_ytdlp(job, url, fmt, out_dir)
            job["title"] = title
            target = _pick_single_file(out_dir, prefer_ext=fmt)
            if target is None:
                raise RuntimeError("yt-dlp produced no file")

        job["filename"] = target.name
        job["status"] = "ready"
    except Exception as e:
        job["status"] = "error"
        job["error"] = _humanize_extraction_error(str(e) or repr(e) or e.__class__.__name__)
        shutil.rmtree(out_dir, ignore_errors=True)
