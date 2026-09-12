import asyncio
import os
import shutil
import time
import uuid
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import BackgroundTasks, FastAPI, File, Form, Header, HTTPException, Request, UploadFile
from fastapi.responses import FileResponse, JSONResponse, Response
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

from .jobs import (
    ARCHIVE_FORMATS,
    AUDIO_DENOISE,
    AUDIO_FORMATS,
    AUDIO_STEMS,
    AUDIO_TO_VIDEO,
    COMIC_TO_PDF,
    DOC_FORMATS,
    DOC_TO_PDF,
    FONT_FORMATS,
    IMAGE_FORMATS,
    IMAGE_SUFFIXES,
    IMAGE_TO_NOBG,
    IMAGE_TO_PDF,
    IMAGE_TO_SVG,
    IMAGE_TO_TEXT,
    FONT_TO_IMAGE,
    IMAGE_TO_DOC,
    OFFICE_TO_IMAGE,
    OFFICE_TO_PDF,
    OFFICE_TO_TEXT,
    PDF_TO_DOC,
    PDF_TO_MD,
    PDF_TO_NOTES,
    SUBTITLE_TO_DOC,
    SUBTITLE_TO_TEXT,
    PDF_OCR_OUTPUTS,
    PDF_OPS,
    PDF_TO_DOCX,
    SINGLE_FILE_COMPRESS,
    PDF_TO_IMAGE,
    PDF_TO_TEXT,
    RAW_SUFFIXES,
    SHRINK_LEVELS,
    SPREADSHEET_FORMATS,
    SPREADSHEET_TO_PDF,
    SPREADSHEET_TO_TEXT,
    SUBTITLE_FORMATS,
    TRANSCRIPT_KIT,
    TRANSLATE_OUTPUT,
    TTS_AUDIO_FORMATS,
    VIDEO_CAPTIONED,
    VIDEO_FRAMES,
    VIDEO_SUBS,
    VIDEO_TO_GIF,
    VIDEO_TO_VIDEO,
    WHISPER_OUTPUTS,
    jobs,
    run_batch_convert_job,
    run_batch_shrink_job,
    run_compress_job,
    run_convert_job,
    run_job,
    run_image_batch_job,
    run_images_to_pdf_job,
    run_pdf_merge_job,
    run_pdf_op_job,
    run_shrink_job,
    MERGEABLE_FORMATS,
    _open_raster,
)
from .meta import META_OP, is_darwin, reveal, run_meta_job
from .language import (
    UI_LANGUAGE,
    default_whisper_model,
    installed_tess_lang,
    is_offline,
    is_whisper_english_only,
    language_name,
    tts_backend,
)

ALLOWED_UPLOAD_FORMATS = (
    IMAGE_FORMATS | DOC_FORMATS | IMAGE_TO_PDF | IMAGE_TO_TEXT
    | IMAGE_TO_NOBG | IMAGE_TO_SVG | IMAGE_TO_DOC
    | PDF_TO_IMAGE | PDF_TO_TEXT | PDF_TO_DOCX | PDF_OCR_OUTPUTS
    | PDF_TO_MD | PDF_TO_NOTES | PDF_TO_DOC
    | DOC_TO_PDF | VIDEO_TO_GIF | VIDEO_TO_VIDEO
    | AUDIO_FORMATS | AUDIO_DENOISE | AUDIO_STEMS | AUDIO_TO_VIDEO
    | OFFICE_TO_PDF | OFFICE_TO_IMAGE | OFFICE_TO_TEXT
    | TTS_AUDIO_FORMATS | WHISPER_OUTPUTS
    | SPREADSHEET_FORMATS | SPREADSHEET_TO_TEXT | SPREADSHEET_TO_PDF
    | SUBTITLE_FORMATS | SUBTITLE_TO_TEXT | SUBTITLE_TO_DOC
    | FONT_FORMATS | FONT_TO_IMAGE
    | TRANSCRIPT_KIT | VIDEO_CAPTIONED | VIDEO_FRAMES | VIDEO_SUBS
    | COMIC_TO_PDF | TRANSLATE_OUTPUT
    | {"pdf", "docx", "epub"}  # audio/video → notes-as-doc chains
)
# Upload caps — default UNLIMITED for local/personal use. Set
# EXPORT_MAX_UPLOAD_MB or EXPORT_MAX_COMPRESS_MB to a positive integer to
# enforce a soft cap (recommended only when exposing publicly — see SECURITY.md).
def _mb_to_bytes(env: str) -> int:
    raw = os.environ.get(env, "0").strip()
    try:
        return max(0, int(raw)) * 1024 * 1024
    except ValueError:
        return 0

MAX_UPLOAD_BYTES = _mb_to_bytes("EXPORT_MAX_UPLOAD_MB")              # 0 = unlimited
MAX_COMPRESS_TOTAL_BYTES = _mb_to_bytes("EXPORT_MAX_COMPRESS_MB")    # 0 = unlimited

def _env_int(name: str, default: int) -> int:
    try:
        return max(0, int(os.environ.get(name, default)))
    except ValueError:
        return default

CLEANUP_AGE_SECONDS = _env_int("EXPORT_CLEANUP_AGE_SECONDS", 21600)       # 6 h default
CLEANUP_INTERVAL_SECONDS = _env_int("EXPORT_CLEANUP_INTERVAL_SECONDS", 900)  # 15 min default
# When truthy (1/true/yes), delete the output file + job dir as soon as the
# user finishes their download. Maximum-ephemeral mode for VPS deployments.
DOWNLOAD_AUTODELETE = os.environ.get("EXPORT_DOWNLOAD_AUTODELETE", "").lower() in ("1", "true", "yes", "on")

PASSWORD = os.environ.get("EXPORT_PASSWORD", "1234")
DOWNLOADS_DIR = Path(os.environ.get("EXPORT_DOWNLOADS", "/tmp/export"))
STATIC_DIR = Path(__file__).resolve().parent.parent / "static"

DOWNLOADS_DIR.mkdir(parents=True, exist_ok=True)


async def _cleanup_loop() -> None:
    while True:
        try:
            now = time.time()
            for entry in DOWNLOADS_DIR.iterdir():
                if entry.is_dir() and (now - entry.stat().st_mtime) > CLEANUP_AGE_SECONDS:
                    shutil.rmtree(entry, ignore_errors=True)
                    jobs.pop(entry.name, None)
        except Exception:
            pass
        await asyncio.sleep(CLEANUP_INTERVAL_SECONDS)


@asynccontextmanager
async def lifespan(_app: FastAPI):
    task = asyncio.create_task(_cleanup_loop())

    # Optional folder-watch mode (Clop-style ambient compression).
    observer = None
    watch_dir = os.environ.get("EXPORT_WATCH_DIR")
    if watch_dir:
        try:
            from .watcher import start_watcher
            observer = start_watcher(Path(watch_dir))
        except Exception as e:  # don't crash the app if watcher fails
            print(f"watcher disabled: {e}")

    try:
        yield
    finally:
        task.cancel()
        if observer is not None:
            observer.stop()
            observer.join(timeout=2)


app = FastAPI(title="export", lifespan=lifespan)


def check(pw: str | None) -> None:
    if pw != PASSWORD:
        raise HTTPException(status_code=401, detail="bad password")


@app.middleware("http")
async def rate_limit_mutate(request: Request, call_next):
    if request.method == "POST" and request.url.path.startswith("/api/"):
        from .limit import RateLimited, client_ip, enforce
        try:
            enforce(client_ip(request))
        except RateLimited as e:
            return JSONResponse({"detail": str(e)}, status_code=429)
    return await call_next(request)


class CreateJob(BaseModel):
    url: str
    format: str = "mp3"
    language: str | None = None
    translate_to: str | None = None


@app.post("/api/jobs")
def create_job(body: CreateJob, bg: BackgroundTasks, x_password: str | None = Header(None)):
    check(x_password)
    if is_offline():
        raise HTTPException(
            status_code=503,
            detail="Offline mode — URL extraction needs a network. Unset EXPORT_OFFLINE or drop a local file.",
        )
    from .ssrf import SsrfRefused, refuse_ssrf
    try:
        refuse_ssrf(body.url)
    except SsrfRefused as e:
        raise HTTPException(status_code=400, detail=str(e)) from e
    job_id = uuid.uuid4().hex[:12]
    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": "",
        "error": None,
        "filename": None,
        "original_size": None,
        "output_size": None,
        "language": None,
        "language_name": None,
        "translate_to": None,
    }
    bg.add_task(run_job, job_id, body.url, body.format, DOWNLOADS_DIR, body.language, body.translate_to)
    return {"job_id": job_id}


@app.post("/api/upload")
async def upload_file(
    bg: BackgroundTasks,
    file: UploadFile = File(...),
    format: str = Form(...),
    quality: int | None = Form(None),
    language: str | None = Form(None),
    translate_to: str | None = Form(None),
    x_password: str | None = Header(None),
):
    check(x_password)
    if format not in ALLOWED_UPLOAD_FORMATS:
        raise HTTPException(status_code=400, detail=f"unsupported format: {format}")
    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)
    input_path = job_dir / f"input_{file.filename or 'upload'}"

    written = 0
    with input_path.open("wb") as f:
        while chunk := await file.read(1024 * 1024):
            written += len(chunk)
            if MAX_UPLOAD_BYTES and written > MAX_UPLOAD_BYTES:
                f.close()
                input_path.unlink(missing_ok=True)
                raise HTTPException(status_code=413, detail=f"file > {MAX_UPLOAD_BYTES // (1024*1024)} MB")
            f.write(chunk)

    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": Path(file.filename or "upload").stem,
        "error": None,
        "filename": None,
        "original_size": written,
        "output_size": None,
        "language": None,
        "language_name": None,
        "translate_to": None,
    }
    bg.add_task(run_convert_job, job_id, input_path, format, DOWNLOADS_DIR, quality, language, translate_to)
    return {"job_id": job_id}


@app.post("/api/compress")
async def compress_files(
    bg: BackgroundTasks,
    files: list[UploadFile] = File(...),
    format: str = Form(...),
    x_password: str | None = Header(None),
):
    check(x_password)
    if format not in (ARCHIVE_FORMATS | SINGLE_FILE_COMPRESS):
        raise HTTPException(status_code=400, detail=f"unsupported compression format: {format}")
    if format in SINGLE_FILE_COMPRESS and len(files) != 1:
        raise HTTPException(status_code=400, detail=f".{format} requires exactly 1 file; use tar.{format} or zip for multiple")
    if not files:
        raise HTTPException(status_code=400, detail="no files provided")

    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)

    saved: list[Path] = []
    total = 0
    for f in files:
        target = job_dir / f"input_{f.filename or f'file{len(saved)}'}"
        with target.open("wb") as out:
            while chunk := await f.read(1024 * 1024):
                total += len(chunk)
                if MAX_COMPRESS_TOTAL_BYTES and total > MAX_COMPRESS_TOTAL_BYTES:
                    out.close()
                    for p in saved + [target]:
                        p.unlink(missing_ok=True)
                    raise HTTPException(
                        status_code=413,
                        detail=f"total upload > {MAX_COMPRESS_TOTAL_BYTES // (1024*1024)} MB",
                    )
                out.write(chunk)
        saved.append(target)

    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": saved[0].stem.removeprefix("input_") if len(saved) == 1 else f"{len(saved)} files",
        "error": None,
        "filename": None,
    }
    bg.add_task(run_compress_job, job_id, saved, format, DOWNLOADS_DIR)
    return {"job_id": job_id}


@app.post("/api/shrink")
async def shrink_file(
    bg: BackgroundTasks,
    file: UploadFile = File(...),
    level: str = Form("medium"),
    quality: int | None = Form(None),
    max_dim: int | None = Form(None),
    scale_percent: float | None = Form(None),
    target_byte: int | None = Form(None),
    x_password: str | None = Header(None),
):
    check(x_password)
    if level not in SHRINK_LEVELS:
        raise HTTPException(status_code=400, detail=f"invalid level: {level}")
    if level == "resize" and not max_dim and not scale_percent and not target_byte:
        raise HTTPException(status_code=400, detail="level=resize requires max_dim or scale_percent")
    if max_dim is not None and (max_dim < 16 or max_dim > 16384):
        raise HTTPException(status_code=400, detail="max_dim must be 16-16384 px")
    if scale_percent is not None and (scale_percent <= 0 or scale_percent > 200):
        raise HTTPException(status_code=400, detail="scale_percent must be in (0, 200]")
    if target_byte is not None and (target_byte < 64 or target_byte > 2_000_000_000):
        raise HTTPException(status_code=400, detail="target_byte must be 64–2000000000")
    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)
    input_path = job_dir / f"input_{file.filename or 'upload'}"

    written = 0
    with input_path.open("wb") as f:
        while chunk := await file.read(1024 * 1024):
            written += len(chunk)
            if MAX_UPLOAD_BYTES and written > MAX_UPLOAD_BYTES:
                f.close()
                input_path.unlink(missing_ok=True)
                raise HTTPException(status_code=413, detail=f"file > {MAX_UPLOAD_BYTES // (1024*1024)} MB")
            f.write(chunk)

    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": Path(file.filename or "upload").stem,
        "error": None,
        "filename": None,
        "original_size": written,
        "output_size": None,
        "predicted_byte": None,
    }
    bg.add_task(run_shrink_job, job_id, input_path, level, DOWNLOADS_DIR,
                quality, max_dim, scale_percent, target_byte)
    return {"job_id": job_id}


_IMAGE_EXT_FOR_BATCH = {".jpg", ".jpeg", ".png", ".webp", ".avif", ".heic",
                        ".heif", ".tiff", ".bmp", ".gif"} | IMAGE_SUFFIXES
BATCH_IMAGE_OUT_FORMATS = {"pdf", "jpeg", "png", "webp", "avif", "gif", "mp4"}


async def _save_batch_uploads(files: list[UploadFile], job_dir: Path) -> tuple[list[Path], int]:
    """Shared upload-saving for /api/images/* endpoints. Returns (saved_paths, total_bytes)."""
    saved: list[Path] = []
    total = 0
    for f in files:
        ext = "." + (f.filename or "").rsplit(".", 1)[-1].lower()
        if ext not in _IMAGE_EXT_FOR_BATCH:
            for p in saved:
                p.unlink(missing_ok=True)
            raise HTTPException(
                status_code=400,
                detail=f"not an image: {f.filename} (need .jpg/.png/.heic/etc.)",
            )
        target = job_dir / f"input_{f.filename}"
        with target.open("wb") as out:
            while chunk := await f.read(1024 * 1024):
                total += len(chunk)
                if MAX_COMPRESS_TOTAL_BYTES and total > MAX_COMPRESS_TOTAL_BYTES:
                    out.close()
                    for p in saved + [target]:
                        p.unlink(missing_ok=True)
                    raise HTTPException(status_code=413, detail="upload too large")
                out.write(chunk)
        saved.append(target)
    return saved, total


@app.post("/api/images/batch")
async def images_batch(
    bg: BackgroundTasks,
    files: list[UploadFile] = File(...),
    format: str = Form(...),
    rotations: str = Form("{}"),
    x_password: str | None = Header(None),
):
    """Multi-image batch conversion. Output format determines the shape:
      pdf  → single multi-page PDF
      jpeg/png/webp/avif → each converted, then zipped
      gif  → single animated GIF (slideshow, 2s/frame)
      mp4  → single MP4 slideshow video (3s/frame, libx264)"""
    check(x_password)
    if format not in BATCH_IMAGE_OUT_FORMATS:
        raise HTTPException(
            status_code=400,
            detail=f"unsupported batch format: {format} (allowed: {sorted(BATCH_IMAGE_OUT_FORMATS)})",
        )
    if not files:
        raise HTTPException(status_code=400, detail="no images provided")
    import json as _json
    try:
        rot_map = _json.loads(rotations) if rotations else {}
        if not isinstance(rot_map, dict):
            rot_map = {}
    except _json.JSONDecodeError:
        rot_map = {}

    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)
    saved, total = await _save_batch_uploads(files, job_dir)

    label = {
        "pdf": "pdf", "gif": "animated GIF", "mp4": "slideshow video",
    }.get(format, f"zipped {format}s")
    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": f"{len(saved)} image{'s' if len(saved) != 1 else ''} → {label}",
        "error": None,
        "filename": None,
        "original_size": total,
        "output_size": None,
    }
    bg.add_task(run_image_batch_job, job_id, saved, format, DOWNLOADS_DIR, rot_map)
    return {"job_id": job_id}


@app.post("/api/images/to-pdf")
async def images_to_pdf(
    bg: BackgroundTasks,
    files: list[UploadFile] = File(...),
    rotations: str = Form("{}"),
    x_password: str | None = Header(None),
):
    """Combine N images into a single multi-page PDF.
    Killer use case: phone photos of a quiz/document → submittable PDF.
    Each photo is auto-rotated using its EXIF orientation tag, then user-
    rotated by the per-file degrees in `rotations` (JSON {filename: degrees})."""
    check(x_password)
    import json as _json
    try:
        rot_map = _json.loads(rotations) if rotations else {}
        if not isinstance(rot_map, dict):
            rot_map = {}
    except _json.JSONDecodeError:
        rot_map = {}
    if not files:
        raise HTTPException(status_code=400, detail="no images provided")
    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)

    saved: list[Path] = []
    total = 0
    for f in files:
        ext = "." + (f.filename or "").rsplit(".", 1)[-1].lower()
        if ext not in _IMAGE_EXT_FOR_BATCH:
            for p in saved:
                p.unlink(missing_ok=True)
            raise HTTPException(
                status_code=400,
                detail=f"not an image: {f.filename} (need .jpg/.png/.heic/etc.)",
            )
        target = job_dir / f"input_{f.filename}"
        with target.open("wb") as out:
            while chunk := await f.read(1024 * 1024):
                total += len(chunk)
                if MAX_COMPRESS_TOTAL_BYTES and total > MAX_COMPRESS_TOTAL_BYTES:
                    out.close()
                    for p in saved + [target]:
                        p.unlink(missing_ok=True)
                    raise HTTPException(status_code=413, detail="upload too large")
                out.write(chunk)
        saved.append(target)

    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": f"{len(saved)} image{'s' if len(saved) != 1 else ''} → pdf",
        "error": None,
        "filename": None,
        "original_size": total,
        "output_size": None,
    }
    bg.add_task(run_images_to_pdf_job, job_id, saved, DOWNLOADS_DIR, rot_map)
    return {"job_id": job_id}


@app.post("/api/batch")
async def batch_convert(
    bg: BackgroundTasks,
    files: list[UploadFile] = File(...),
    format: str = Form(...),
    merge: str = Form("0"),
    quality: int | None = Form(None),
    language: str | None = Form(None),
    translate_to: str | None = Form(None),
    x_password: str | None = Header(None),
):
    """Mass convert N homogeneous files to `format`. With merge=1, returns one
    merged file using the format-aware merger in jobs.py:_merge_outputs:
      pdf      → pypdf merge
      xlsx     → multi-sheet workbook (one input per sheet)
      json     → wrap as a single JSON array
      csv/tsv  → header-deduped concatenation
      srt/vtt/ass/ssa/sub/sbv → pysubs2 with cumulative timestamp shift
      mp3/m4a/wav/flac/ogg/opus/aac/mp4/mov/webm/mkv → ffmpeg concat
      jpeg/png/webp/avif/tiff/bmp → tile grid (cols=ceil(sqrt(N)))
      txt/md/html/rtf/...        → labeled text concat
    With merge=0 (or for non-mergeable formats), returns a zip of N converted files."""
    check(x_password)
    if not files or len(files) < 2:
        raise HTTPException(status_code=400, detail="batch needs at least 2 files")
    if format not in ALLOWED_UPLOAD_FORMATS:
        raise HTTPException(status_code=400, detail=f"unsupported format: {format}")
    do_merge = merge.lower() in ("1", "true", "yes", "on")
    if do_merge and format not in MERGEABLE_FORMATS:
        raise HTTPException(
            status_code=400,
            detail=f"format '{format}' is not mergeable (mergeable: {sorted(MERGEABLE_FORMATS)})",
        )

    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)

    saved: list[Path] = []
    total = 0
    for f in files:
        target = job_dir / f"input_{f.filename}"
        with target.open("wb") as out:
            while chunk := await f.read(1024 * 1024):
                total += len(chunk)
                if MAX_COMPRESS_TOTAL_BYTES and total > MAX_COMPRESS_TOTAL_BYTES:
                    out.close()
                    for p in saved + [target]:
                        p.unlink(missing_ok=True)
                    raise HTTPException(status_code=413, detail="upload too large")
                out.write(chunk)
        saved.append(target)

    label = f"merged {format}" if do_merge else f"{len(saved)} {format}s (zip)"
    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": f"{len(saved)} files → {label}",
        "error": None,
        "filename": None,
        "original_size": total,
        "output_size": None,
    }
    bg.add_task(run_batch_convert_job, job_id, saved, format, DOWNLOADS_DIR, do_merge, quality, language, translate_to)
    return {"job_id": job_id}


@app.post("/api/batch/shrink")
async def batch_shrink(
    bg: BackgroundTasks,
    files: list[UploadFile] = File(...),
    level: str = Form("medium"),
    quality: int | None = Form(None),
    max_dim: int | None = Form(None),
    scale_percent: float | None = Form(None),
    target_byte: int | None = Form(None),
    x_password: str | None = Header(None),
):
    """Apply the same shrink/resize params to N homogeneous files. Returns a
    zip of the N shrunk outputs. Mirrors /api/shrink but for batches.
    level=resize requires either max_dim or scale_percent."""
    check(x_password)
    if not files or len(files) < 2:
        raise HTTPException(status_code=400, detail="batch shrink needs at least 2 files")
    if level not in SHRINK_LEVELS:
        raise HTTPException(status_code=400, detail=f"invalid level: {level}")
    if level == "resize" and not max_dim and not scale_percent and not target_byte:
        raise HTTPException(status_code=400, detail="level=resize requires max_dim or scale_percent")
    if max_dim is not None and (max_dim < 16 or max_dim > 16384):
        raise HTTPException(status_code=400, detail="max_dim must be 16-16384 px")
    if scale_percent is not None and (scale_percent <= 0 or scale_percent > 200):
        raise HTTPException(status_code=400, detail="scale_percent must be in (0, 200]")
    if target_byte is not None and (target_byte < 64 or target_byte > 2_000_000_000):
        raise HTTPException(status_code=400, detail="target_byte must be 64–2000000000")

    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)

    saved: list[Path] = []
    total = 0
    for f in files:
        target = job_dir / f"input_{f.filename}"
        with target.open("wb") as out:
            while chunk := await f.read(1024 * 1024):
                total += len(chunk)
                if MAX_COMPRESS_TOTAL_BYTES and total > MAX_COMPRESS_TOTAL_BYTES:
                    out.close()
                    for p in saved + [target]:
                        p.unlink(missing_ok=True)
                    raise HTTPException(status_code=413, detail="upload too large")
                out.write(chunk)
        saved.append(target)

    label = "resized" if level == "resize" else f"shrunk ({level})"
    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": f"{len(saved)} files → {label}",
        "error": None,
        "filename": None,
        "original_size": total,
        "output_size": None,
    }
    bg.add_task(run_batch_shrink_job, job_id, saved, level, DOWNLOADS_DIR,
                quality, max_dim, scale_percent, target_byte)
    return {"job_id": job_id}


@app.post("/api/pdf/merge")
async def pdf_merge(
    bg: BackgroundTasks,
    files: list[UploadFile] = File(...),
    x_password: str | None = Header(None),
):
    check(x_password)
    if len(files) < 2:
        raise HTTPException(status_code=400, detail="merge needs at least 2 PDFs")

    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)

    saved: list[Path] = []
    total = 0
    for f in files:
        if not (f.filename or "").lower().endswith(".pdf"):
            for p in saved:
                p.unlink(missing_ok=True)
            raise HTTPException(status_code=400, detail=f"not a PDF: {f.filename}")
        target = job_dir / f"input_{f.filename}"
        with target.open("wb") as out:
            while chunk := await f.read(1024 * 1024):
                total += len(chunk)
                if MAX_COMPRESS_TOTAL_BYTES and total > MAX_COMPRESS_TOTAL_BYTES:
                    out.close()
                    for p in saved + [target]:
                        p.unlink(missing_ok=True)
                    raise HTTPException(status_code=413, detail="upload too large")
                out.write(chunk)
        saved.append(target)

    jobs[job_id] = {
        "status": "pending", "progress": 0,
        "title": f"merge {len(saved)} PDFs",
        "error": None, "filename": None,
    }
    bg.add_task(run_pdf_merge_job, job_id, saved, DOWNLOADS_DIR)
    return {"job_id": job_id}


@app.post("/api/pdf/op")
async def pdf_op(
    bg: BackgroundTasks,
    file: UploadFile = File(...),
    op: str = Form(...),
    value: str = Form(""),
    x_password: str | None = Header(None),
):
    check(x_password)
    if op not in PDF_OPS:
        raise HTTPException(status_code=400, detail=f"unknown PDF op: {op}")
    if not (file.filename or "").lower().endswith(".pdf"):
        raise HTTPException(status_code=400, detail="input must be a .pdf file")

    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)
    input_path = job_dir / f"input_{file.filename}"

    written = 0
    with input_path.open("wb") as f:
        while chunk := await file.read(1024 * 1024):
            written += len(chunk)
            if MAX_UPLOAD_BYTES and written > MAX_UPLOAD_BYTES:
                f.close()
                input_path.unlink(missing_ok=True)
                raise HTTPException(status_code=413, detail=f"file > {MAX_UPLOAD_BYTES // (1024*1024)} MB")
            f.write(chunk)

    jobs[job_id] = {
        "status": "pending", "progress": 0,
        "title": Path(file.filename or "upload").stem,
        "error": None, "filename": None,
    }
    bg.add_task(run_pdf_op_job, job_id, input_path, op, value, DOWNLOADS_DIR)
    return {"job_id": job_id}


@app.post("/api/meta")
async def meta_file(
    bg: BackgroundTasks,
    file: UploadFile = File(...),
    op: str = Form(...),
    modified_at: str = Form(""),
    tag: str = Form(""),
    password: str = Form(""),
    from_exif: str = Form(""),
    x_password: str | None = Header(None),
):
    check(x_password)
    if op not in META_OP:
        raise HTTPException(status_code=400, detail=f"unknown meta op: {op}")
    job_id = uuid.uuid4().hex[:12]
    job_dir = DOWNLOADS_DIR / job_id
    job_dir.mkdir(parents=True, exist_ok=True)
    input_path = job_dir / f"input_{file.filename or 'upload'}"
    written = 0
    with input_path.open("wb") as f:
        while chunk := await file.read(1024 * 1024):
            written += len(chunk)
            if MAX_UPLOAD_BYTES and written > MAX_UPLOAD_BYTES:
                f.close()
                input_path.unlink(missing_ok=True)
                raise HTTPException(status_code=413, detail=f"file > {MAX_UPLOAD_BYTES // (1024*1024)} MB")
            f.write(chunk)
    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": Path(file.filename or "upload").stem,
        "error": None,
        "filename": None,
        "original_size": written,
        "output_size": None,
    }
    is_from_exif = from_exif.strip().lower() in {"1", "true", "yes", "on"}
    bg.add_task(
        run_meta_job, job_id, input_path, op, DOWNLOADS_DIR,
        modified_at, tag, password, is_from_exif,
    )
    return {"job_id": job_id}


@app.post("/api/jobs/{job_id}/reveal")
def reveal_job(job_id: str, x_password: str | None = Header(None)):
    check(x_password)
    if not is_darwin():
        raise HTTPException(status_code=400, detail="show in Finder is macOS-only")
    job = jobs.get(job_id)
    if job is None or job.get("status") != "ready" or not job.get("filename"):
        raise HTTPException(status_code=404, detail="not found")
    path = DOWNLOADS_DIR / job_id / job["filename"]
    if not path.exists():
        raise HTTPException(status_code=404, detail="file missing")
    try:
        reveal(path)
    except Exception as e:
        raise HTTPException(status_code=400, detail=str(e)) from e
    return {"ok": True}


@app.get("/api/jobs/{job_id}")
def get_job(job_id: str, x_password: str | None = Header(None)):
    check(x_password)
    job = jobs.get(job_id)
    if job is None:
        raise HTTPException(status_code=404, detail="not found")
    return job


@app.get("/api/files/{job_id}")
def download_file(job_id: str, token: str | None = None, pw: str | None = None):
    if pw is not None:
        raise HTTPException(status_code=401, detail="use the download token, not ?pw=")
    job = jobs.get(job_id)
    if job is None:
        raise HTTPException(status_code=404, detail="not found")
    if job["status"] != "ready" or not job["filename"]:
        raise HTTPException(status_code=400, detail="not ready")
    if not token or token != job.get("token"):
        raise HTTPException(status_code=401, detail="bad download token")
    path = DOWNLOADS_DIR / job_id / job["filename"]
    if not path.exists():
        raise HTTPException(status_code=404, detail="file missing")

    # Stream-and-delete mode: nuke the output + job dir + dict entry as soon
    # as the response completes. Truly ephemeral storage at the cost of "I'd
    # like to download that again" being a re-process.
    if DOWNLOAD_AUTODELETE:
        from starlette.background import BackgroundTask

        def _cleanup():
            try:
                shutil.rmtree(path.parent, ignore_errors=True)
                jobs.pop(job_id, None)
            except Exception:
                pass

        return FileResponse(
            path, filename=job["filename"], media_type="application/octet-stream",
            background=BackgroundTask(_cleanup),
        )

    return FileResponse(path, filename=job["filename"], media_type="application/octet-stream")


@app.post("/api/thumbnail")
async def thumbnail(
    file: UploadFile = File(...),
    x_password: str | None = Header(None),
):
    """Server-side thumbnail rendering. Browsers can't decode HEIC/HEIF/AVIF/
    TIFF in <img> tags but the server has Pillow + pillow-heif. POST a file,
    get back a small JPEG. Used by the frontend preview tiles."""
    check(x_password)
    import io
    from PIL import Image, ImageOps
    # Cap to keep one user's giant photo from blowing memory on this preview path.
    data = await file.read(50 * 1024 * 1024)
    try:
        name = file.filename or ""
        suffix = Path(name).suffix.lower()
        if suffix in {".svg"} | RAW_SUFFIXES | {".jxl"}:
            import tempfile
            with tempfile.NamedTemporaryFile(suffix=suffix, delete=False) as tmp:
                tmp.write(data)
                tmp_path = Path(tmp.name)
            try:
                img = _open_raster(tmp_path)
            finally:
                tmp_path.unlink(missing_ok=True)
        else:
            img = Image.open(io.BytesIO(data))
            img = ImageOps.exif_transpose(img)
        img.thumbnail((220, 220))  # max edge 220 px
        if img.mode != "RGB":
            img = img.convert("RGB")
        buf = io.BytesIO()
        img.save(buf, format="JPEG", quality=72, optimize=True)
        return Response(content=buf.getvalue(), media_type="image/jpeg",
                        headers={"Cache-Control": "private, max-age=300"})
    except Exception as e:
        raise HTTPException(status_code=400, detail=f"can't decode as image: {e}")


@app.get("/api/capabilities")
def capabilities():
    """Public — no auth — so the frontend can render a "what's supported" panel."""
    whisper_model = default_whisper_model()
    offline = is_offline()
    groq_key = bool(os.environ.get("EXPORT_GROQ_API_KEY"))
    return {
        "url_extraction": {
            "video_audio_sites": "~1800 (yt-dlp)",
            "image_sites": "~300 (gallery-dl)",
            "url_outputs": ["mp3", "mp4", "images", "txt", "srt", "vtt", "notes",
                            "kit", "pdf", "docx", "epub", "captioned", "frames", "subs",
                            "translate"],
        },
        "file_inputs": {
            "image":       sorted(IMAGE_SUFFIXES),
            "video":       sorted([".mp4",".mov",".webm",".mkv",".avi",".m4v",".flv",".wmv",".3gp",".ts",".mts",".m2ts",".ogv",".mpg",".mpeg",".vob",".asf",".mxf"]),
            "audio":       sorted([".mp3",".m4a",".wav",".flac",".ogg",".aac",".opus",".aiff",".aif",".wma",".amr",".3ga",".m4b",".ac3",".caf"]),
            "pdf":         [".pdf"],
            "document":    sorted([".md",".markdown",".html",".htm",".rtf",".epub",".txt",".tex",".org",".rst",".ipynb",".adoc",".asciidoc",".textile",".docbook",".fb2"]),
            "office":      sorted([".docx",".doc",".pptx",".ppt",".xlsx",".xls",".odp",".ods",".odt"]),
            "spreadsheet": sorted([".csv",".tsv",".xlsx",".json",".yaml",".yml",".toml",".xml"]),
            "subtitle":    sorted([".srt",".vtt",".ass",".ssa",".sub",".sbv"]),
            "font":        sorted([".ttf",".otf",".woff",".woff2"]),
            "comic":       [".cbz"],
        },
        "convert_outputs": {
            "image":       sorted(IMAGE_FORMATS | IMAGE_TO_PDF | IMAGE_TO_TEXT | IMAGE_TO_NOBG | IMAGE_TO_SVG | IMAGE_TO_DOC | TRANSLATE_OUTPUT),
            "video":       sorted(VIDEO_TO_VIDEO | VIDEO_TO_GIF | AUDIO_FORMATS | AUDIO_TO_VIDEO | WHISPER_OUTPUTS | TRANSCRIPT_KIT | VIDEO_CAPTIONED | VIDEO_FRAMES | VIDEO_SUBS | TRANSLATE_OUTPUT | {"txt", "pdf", "docx", "epub", "png"}),
            "audio":       sorted(AUDIO_FORMATS | WHISPER_OUTPUTS | AUDIO_DENOISE | AUDIO_STEMS | AUDIO_TO_VIDEO | TRANSCRIPT_KIT | TRANSLATE_OUTPUT | {"txt", "pdf", "docx", "epub"}),
            "pdf":         sorted(PDF_TO_IMAGE | PDF_TO_TEXT | PDF_TO_DOCX | PDF_OCR_OUTPUTS | PDF_TO_MD | PDF_TO_NOTES | PDF_TO_DOC | TTS_AUDIO_FORMATS | AUDIO_TO_VIDEO | TRANSLATE_OUTPUT),
            "document":    sorted(DOC_FORMATS | DOC_TO_PDF | TTS_AUDIO_FORMATS | TRANSLATE_OUTPUT),
            "office":      sorted(OFFICE_TO_PDF | OFFICE_TO_IMAGE | OFFICE_TO_TEXT | DOC_FORMATS | TTS_AUDIO_FORMATS | AUDIO_TO_VIDEO | TRANSLATE_OUTPUT),
            "spreadsheet": sorted(SPREADSHEET_FORMATS | SPREADSHEET_TO_TEXT | SPREADSHEET_TO_PDF | TRANSLATE_OUTPUT),
            "subtitle":    sorted(SUBTITLE_FORMATS | SUBTITLE_TO_TEXT | SUBTITLE_TO_DOC | TTS_AUDIO_FORMATS | AUDIO_TO_VIDEO | TRANSLATE_OUTPUT),
            "font":        sorted(FONT_FORMATS | FONT_TO_IMAGE),
            "comic":       sorted(COMIC_TO_PDF),
        },
        "operations": {
            "compress_archive":     sorted(ARCHIVE_FORMATS),
            "compress_single_file": sorted(SINGLE_FILE_COMPRESS),
            "shrink_levels":        sorted(SHRINK_LEVELS),
            "shrink_target_byte":   20_000_000,
            "pdf_ops":              sorted(list(PDF_OPS) + ["merge"]),
            "batch_mergeable":      sorted(MERGEABLE_FORMATS),
            "meta_op":              sorted(META_OP),
        },
        "meta": {
            "op": sorted(META_OP),
            "is_finder_tag": is_darwin(),
            "is_created_at": is_darwin(),
            "is_reveal": is_darwin(),
        },
        "language": {
            "ui": [{"code": c, "name": language_name(c)} for c in UI_LANGUAGE],
            "ocr_pack": list(installed_tess_lang()),
            "whisper_model": whisper_model,
            "is_whisper_multilingual": not is_whisper_english_only(whisper_model),
            "tts_backend": tts_backend(),
            "translate": "nllb-200-distilled-600M (CTranslate2 int8, 200 languages incl. Filipino)",
        },
        "offline": {
            "is_offline": offline,
            "is_url_enabled": not offline,
            "is_groq_enabled": groq_key and not offline,
            "is_model_download_enabled": not offline,
        },
        "limits": {
            "max_upload_bytes":   MAX_UPLOAD_BYTES,        # 0 = unlimited
            "max_compress_bytes": MAX_COMPRESS_TOTAL_BYTES, # 0 = unlimited
            "job_ttl_seconds":    CLEANUP_AGE_SECONDS,
        },
    }


@app.get("/api/reachable")
def reachable_format(ext: str = ""):
    """Public — formats reachable from this input suffix by composing bridges."""
    from .graph import grouped_format, reachable
    suffix = ext if ext.startswith(".") else f".{ext}"
    fmt_list = reachable(suffix)
    return {"format": fmt_list, "kind": grouped_format(fmt_list)}


@app.get("/")
def index():
    return FileResponse(STATIC_DIR / "index.html")


app.mount("/static", StaticFiles(directory=STATIC_DIR), name="static")
