<p align="center">
  <img src="https://img.shields.io/badge/python-3.11%2B-3776AB?logo=python&logoColor=white" alt="Python 3.11+">
  <img src="https://img.shields.io/badge/FastAPI-uvicorn-009688?logo=fastapi&logoColor=white" alt="FastAPI">
  <img src="https://img.shields.io/badge/runs-locally-informational" alt="Runs locally">
  <img src="https://img.shields.io/badge/glue%20code-MIT-green" alt="Glue code MIT">
</p>

# export

**One local web page that replaces the file-converter, PDF-tool and YouTube-to-MP3 sites you keep tabbing through.**

Paste a URL or drop a file, and export converts, extracts, compresses or
transforms it — 180+ conversion paths across images, video, audio, documents,
PDFs, spreadsheets, subtitles, fonts, comics and archives.

- **From a link:** YouTube and ~1800 other sites (via yt-dlp / gallery-dl) to
  MP3, MP4, a transcript, subtitles, burned-in captions, frames, or
  NoteGPT-style notes.
- **From a file:** convert between formats, shrink to a target size, merge or
  split PDFs, OCR, speech-to-text, text-to-speech, translation, background
  removal, metadata scrubbing.
- **Private by default:** everything runs on your machine with open-source
  tools. Groq cloud transcription is optional, and there is an explicit offline
  mode.

It sits behind a single password and is built for personal use on localhost or
a private network.

```
http://localhost:8500   →   password   →   paste a URL or drop a file
```

The **show what's supported** link at the bottom of the page lists every
conversion the running server can do right now.

## Quick start

macOS:

```bash
git clone https://github.com/angelonrevelo/export.git && cd export
npm run install:macos   # brew deps, LibreOffice, venv, Tesseract packs
npm run dev             # → http://localhost:8500
```

Debian / Ubuntu:

```bash
npm run install:linux   # or: bash scripts/install-linux.sh
cp .env.example .env    # set EXPORT_PASSWORD
bash scripts/run.sh
```

`npm` here is only a shortcut — `dev`, `start` and `watch` all run
`scripts/run.sh`, which loads `.env` and starts uvicorn. On first launch it
creates `.env` with the password `1234`. The first job that needs a model
(Whisper, rembg, demucs, Piper) pauses about 30 s while it downloads; run
`bash scripts/prefetch.sh` to fetch them ahead of time.

Tests: `pip install -e ".[dev]" && pytest`.

## Configuration

Everything is an env var in `.env`; all are optional except the password.
Defaults and details are in [docs/internals.md](docs/internals.md#configuration).

| Key | What it controls |
|---|---|
| `EXPORT_PASSWORD` | The one password the UI asks for |
| `EXPORT_DOWNLOADS` | Where job output is written |
| `EXPORT_MAX_UPLOAD_MB` | Per-file upload cap |
| `EXPORT_MAX_COMPRESS_MB` | Combined cap for multi-file jobs |
| `EXPORT_RATE_LIMIT_PER_MINUTE` | Mutating requests per IP per minute |
| `EXPORT_CLEANUP_AGE_SECONDS` | How old a job gets before it is deleted |
| `EXPORT_CLEANUP_INTERVAL_SECONDS` | How often cleanup runs |
| `EXPORT_DOWNLOAD_AUTODELETE` | Delete output the moment it is downloaded |
| `EXPORT_CORS_ORIGIN` | Other web apps allowed to call the API |
| `EXPORT_WATCH_DIR` | A folder whose drops are auto-shrunk |
| `EXPORT_OFFLINE` | Refuse any network fetch |
| `EXPORT_LANGUAGE` | Default language for speech/OCR |
| `EXPORT_OCR_LANG` | Tesseract language packs |
| `EXPORT_WHISPER_MODEL` / `EXPORT_WHISPER_MODEL_DIR` | Local speech-to-text model |
| `EXPORT_WHISPER_PH_MODEL` / `EXPORT_WHISPER_PH_DIR` | Model for Philippine languages |
| `EXPORT_GROQ_API_KEY` / `EXPORT_GROQ_MODEL` / `EXPORT_GROQ_CHUNK_SECONDS` | Optional Groq cloud transcription |
| `EXPORT_PIPER_VOICE` / `EXPORT_PIPER_VOICE_DIR` | Text-to-speech voice |
| `EXPORT_NLLB_MODEL` / `EXPORT_NLLB_MODEL_DIR` / `EXPORT_NLLB_TOKENIZER` | Translation model |
| `EXPORT_TRANSLATE_TO` | Default translation target |
| `EXPORT_YTDLP_BROWSER` / `EXPORT_YTDLP_COOKIES` | Cookies for login-walled sites |
| `EXPORT_YTDLP_IMPERSONATE` | Browser impersonation for yt-dlp |
| `EXPORT_HTTP_PROXY` | Proxy for IP-blocked sites |
| `EXPORT_SHRINK_VIDEO_CODEC` | Codec used when shrinking video |
| `HOST` / `PORT` | Bind address (default `127.0.0.1:8500`) |

## How it works

```
browser ──► FastAPI (app/main.py, port 8500)
               │  URL jobs → yt-dlp / gallery-dl → ffmpeg / Whisper
               │  uploads  → Pillow · ffmpeg · pandoc · LibreOffice · Ghostscript · Tesseract …
               ▼
          /tmp/export/{job_id}/  ──► download link (short-lived token)
```

One process, jobs held in memory, output files in a temp folder that a
background loop clears every few hours. Each conversion is a small bridge
function from one format to another.

## Deploying

For a server, see [scripts/export.service](scripts/export.service) (systemd),
[scripts/nginx-export.conf](scripts/nginx-export.conf) and
[scripts/Caddyfile](scripts/Caddyfile) (HTTPS reverse proxy). Read
[SECURITY.md](SECURITY.md) before exposing it beyond localhost.

## Links

- [docs/internals.md](docs/internals.md) — full capability tables, architecture, every env var with defaults, compatibility matrix, limits, licences
- [CHANGELOG.md](CHANGELOG.md)
- [SECURITY.md](SECURITY.md)

The glue code is MIT; bundled tools carry their own licences (Pandoc GPL,
Ghostscript AGPL, …) — see [docs/internals.md](docs/internals.md#license).
