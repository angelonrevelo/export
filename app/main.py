import asyncio
import os
import shutil
import time
import uuid
from contextlib import asynccontextmanager
from pathlib import Path

from fastapi import BackgroundTasks, FastAPI, File, Form, Header, HTTPException, UploadFile
from fastapi.responses import FileResponse
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

from .jobs import (
    ARCHIVE_FORMATS,
    AUDIO_DENOISE,
    AUDIO_FORMATS,
    AUDIO_STEMS,
    DOC_FORMATS,
    DOC_TO_PDF,
    IMAGE_FORMATS,
    IMAGE_TO_NOBG,
    IMAGE_TO_PDF,
    IMAGE_TO_SVG,
    IMAGE_TO_TEXT,
    OFFICE_TO_PDF,
    PDF_OCR_OUTPUTS,
    PDF_OPS,
    PDF_TO_DOCX,
    SINGLE_FILE_COMPRESS,
    PDF_TO_IMAGE,
    PDF_TO_TEXT,
    SHRINK_LEVELS,
    TTS_AUDIO_FORMATS,
    VIDEO_TO_GIF,
    VIDEO_TO_VIDEO,
    WHISPER_OUTPUTS,
    jobs,
    run_compress_job,
    run_convert_job,
    run_job,
    run_pdf_merge_job,
    run_pdf_op_job,
    run_shrink_job,
)

ALLOWED_UPLOAD_FORMATS = (
    IMAGE_FORMATS | DOC_FORMATS | IMAGE_TO_PDF | IMAGE_TO_TEXT
    | IMAGE_TO_NOBG | IMAGE_TO_SVG
    | PDF_TO_IMAGE | PDF_TO_TEXT | PDF_TO_DOCX | PDF_OCR_OUTPUTS
    | DOC_TO_PDF | VIDEO_TO_GIF | VIDEO_TO_VIDEO
    | AUDIO_FORMATS | AUDIO_DENOISE | AUDIO_STEMS
    | OFFICE_TO_PDF | TTS_AUDIO_FORMATS | WHISPER_OUTPUTS
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
CLEANUP_AGE_SECONDS = 3600          # delete job dirs older than 1 h
CLEANUP_INTERVAL_SECONDS = 900      # sweep every 15 min

PASSWORD = os.environ.get("EXPORT_PASSWORD", "changeme")
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


class CreateJob(BaseModel):
    url: str
    format: str = "mp3"


@app.post("/api/jobs")
def create_job(body: CreateJob, bg: BackgroundTasks, x_password: str | None = Header(None)):
    check(x_password)
    job_id = uuid.uuid4().hex[:12]
    jobs[job_id] = {
        "status": "pending",
        "progress": 0,
        "title": "",
        "error": None,
        "filename": None,
        "original_size": None,
        "output_size": None,
    }
    bg.add_task(run_job, job_id, body.url, body.format, DOWNLOADS_DIR)
    return {"job_id": job_id}


@app.post("/api/upload")
async def upload_file(
    bg: BackgroundTasks,
    file: UploadFile = File(...),
    format: str = Form(...),
    quality: int | None = Form(None),
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
    }
    bg.add_task(run_convert_job, job_id, input_path, format, DOWNLOADS_DIR, quality)
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
    x_password: str | None = Header(None),
):
    check(x_password)
    if level not in SHRINK_LEVELS:
        raise HTTPException(status_code=400, detail=f"invalid level: {level}")
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
    bg.add_task(run_shrink_job, job_id, input_path, level, DOWNLOADS_DIR, quality)
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


@app.get("/api/jobs/{job_id}")
def get_job(job_id: str, x_password: str | None = Header(None)):
    check(x_password)
    job = jobs.get(job_id)
    if job is None:
        raise HTTPException(status_code=404, detail="not found")
    return job


@app.get("/api/files/{job_id}")
def download_file(job_id: str, pw: str | None = None):
    check(pw)
    job = jobs.get(job_id)
    if job is None:
        raise HTTPException(status_code=404, detail="not found")
    if job["status"] != "ready" or not job["filename"]:
        raise HTTPException(status_code=400, detail="not ready")
    path = DOWNLOADS_DIR / job_id / job["filename"]
    if not path.exists():
        raise HTTPException(status_code=404, detail="file missing")
    return FileResponse(path, filename=job["filename"], media_type="application/octet-stream")


@app.get("/api/capabilities")
def capabilities():
    """Public — no auth — so the frontend can render a "what's supported" panel."""
    return {
        "url_extraction": {
            "video_audio_sites": "~1800 (yt-dlp)",
            "image_sites": "~300 (gallery-dl)",
            "url_outputs": ["mp3", "mp4", "images", "txt", "srt", "vtt"],
        },
        "file_inputs": {
            "image":  sorted([".jpg",".jpeg",".png",".webp",".avif",".heic",".heif",".tiff",".bmp",".gif"]),
            "video":  sorted([".mp4",".mov",".webm",".mkv",".avi",".m4v"]),
            "audio":  sorted([".mp3",".m4a",".wav",".flac",".ogg",".aac",".opus"]),
            "pdf":    [".pdf"],
            "document": sorted([".md",".html",".htm",".rtf",".epub",".txt",".tex",".org",".rst"]),
            "office": sorted([".docx",".doc",".pptx",".ppt",".xlsx",".xls",".odp",".ods",".odt"]),
        },
        "convert_outputs": {
            "image":   sorted(IMAGE_FORMATS | IMAGE_TO_PDF | IMAGE_TO_TEXT | IMAGE_TO_NOBG | IMAGE_TO_SVG),
            "video":   sorted(VIDEO_TO_VIDEO | VIDEO_TO_GIF | WHISPER_OUTPUTS | {"txt"}),
            "audio":   sorted(AUDIO_FORMATS | WHISPER_OUTPUTS | AUDIO_DENOISE | AUDIO_STEMS | {"txt"}),
            "pdf":     sorted(PDF_TO_IMAGE | PDF_TO_TEXT | PDF_TO_DOCX | PDF_OCR_OUTPUTS),
            "document": sorted(DOC_FORMATS | DOC_TO_PDF | TTS_AUDIO_FORMATS),
            "office":  sorted(OFFICE_TO_PDF),
        },
        "operations": {
            "compress_archive":     sorted(ARCHIVE_FORMATS),
            "compress_single_file": sorted(SINGLE_FILE_COMPRESS),
            "shrink_levels":        sorted(SHRINK_LEVELS),
            "pdf_ops":              sorted(list(PDF_OPS) + ["merge"]),
        },
        "limits": {
            "max_upload_bytes":   MAX_UPLOAD_BYTES,        # 0 = unlimited
            "max_compress_bytes": MAX_COMPRESS_TOTAL_BYTES, # 0 = unlimited
            "job_ttl_seconds":    CLEANUP_AGE_SECONDS,
        },
    }


@app.get("/")
def index():
    return FileResponse(STATIC_DIR / "index.html")


app.mount("/static", StaticFiles(directory=STATIC_DIR), name="static")
