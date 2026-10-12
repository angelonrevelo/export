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
function from one format to another. Those bridges are Python. A request
from the page does not call the Rust crates.

## Running locally

`scripts/run.sh` is the runnable app. It loads `.env` and starts uvicorn on
`app.main:app`, bound to `127.0.0.1:8500` unless `HOST` and `PORT` say
otherwise. A checkout of `main` is what that local run serves.

## Deploying

For a server, see [scripts/export.service](scripts/export.service) (systemd),
[scripts/nginx-export.conf](scripts/nginx-export.conf) and
[scripts/Caddyfile](scripts/Caddyfile) (HTTPS reverse proxy). Read
[SECURITY.md](SECURITY.md) before exposing it beyond localhost.

## Rust crates

`Cargo.toml` is one workspace. Each crate is listed once. They are
zero-dependency libraries next to the Python app, not a second server.
`export-kernel` has a checked raw DEFLATE inflater, and `export-deflate` is
a separate raw compressor and inflater. Pack, image, docs, and sheet keep
their own inflate code too.

| Crate | What the library does |
|---|---|
| `export-kernel` | Little-endian container, streaming SHA-256, CRC-32, magic-byte sniff, checked raw DEFLATE inflate. No network. |
| `export-subtitle` | SRT, WebVTT, and ASS/SSA cues, plus a CRC-checked cue file. One ASS bold, italic, underline, and strikeout override. |
| `export-web` | In-memory job runner with a hard byte budget. No threads and no model. Not the FastAPI process. |
| `export-pdf` | Split and merge classic-xref text PDFs, including one small uncompressed RGB, gray, or CMYK page. |
| `export-pack` | Store-method zip, CBZ to a minimal PDF, TTF/OTF container copy, PNG, and this crate's own inflate. |
| `export-sheet` | Streaming CSV, TSV, and one JSON array. BIFF8 `.xls` read covers the generated layouts in the crate. One-sheet XLSX: stored write, stored or checked deflate read. |
| `export-meta` | `EXMETA01` record and in-place scrub of that header. OOXML decrypt for the standard AES-128-ECB and agile AES-256-CBC/SHA-512 packages this crate accepts. |
| `export-media` | Stream-copy of non-fragmented MP4/MOV/M4A, WAV/PCM, a one-pass WAV shrink, and a one-frame gray audiogram. Fragmented MP4 is refused. AAC, Opus, MP3, FLAC, GIF, and WebM are identified and not decoded. |
| `export-translate` | English ↔ Spanish and English ↔ Filipino lexicon transfer. Not NLLB. Any other pair fails. |
| `export-speech` | Log-mel, integer kernels, greedy CTC writers, a fixture that spells `HI`, `IH`, or `NO`, 5×7 OCR for a small glyph set, formant-sine WAV, and spectral subtraction. Not Whisper, Tesseract, or Piper. |
| `export-image` | PNG decode, baseline JPEG shrink-to-bytes, 220px JPEG thumbnails, a simple WebP still plus a few listed animations, and a 16×16 or 32×32 black-square SVG. Not HEIC, AVIF, TIFF, RAW, JPEG XL, or background removal. |
| `export-fetch` | Cleartext HTTP/1.1 (redirects and ranges). HTTPS, YouTube ciphers, and DRM are refused. |
| `export-docs` | Document hops with no LibreOffice, pandoc, or Ghostscript. `pairs()` rows with `ready: true` are the implemented text conversions (docx, odt, markdown, HTML, txt, and classic PDF text, split, merge, rotate, watermark, and a Flate shrink that must inflate back to the same bytes). `ready: false` rows are not implemented, including PDF encrypt, decrypt, pdf→docx, and pdf→png. |
| `export-gif` | GIF87a/89a LZW and a 3-3-2 quantizer. It does not decode video. |
| `export-deflate` | Raw DEFLATE compress and inflate (RFC 1951). No zlib wrapper. Separate from `export-kernel`'s inflater. |
| `export-audio` | FLAC subset encode and decode, plus WAV. It does not call ffmpeg. |
| `export-h264` | Constrained Baseline Annex B written by this crate: one I-frame and following P-frames, and a still of that I-frame reconstruction. Not a general MP4 decoder and not a shrink. |
| `export-opus` | One Opus packet: silence, a few 20 ms 440 Hz sine frames, and one prepared stereo packet. Not a song encoder. |
| `export-webm` | Stream-copy WebM and Matroska. VP8, VP9, Opus, and Vorbis payloads are copied and not decoded. |
| `export-aac` | AAC-LC ADTS at 44.1 kHz, 1024 samples per channel, including later frames of one sine and a stereo pair. HE-AAC, SBR, and parametric stereo are unsupported. |
| `export-hls` | Cleartext HLS media playlists, and cleartext HTTP/1.1 fetch of them. `https`, `#EXT-X-KEY`, cookies, and credentials are unsupported. |
| `export-fmp4` | Stream-copy of fragmented MP4 sample bytes. H.264 `avc1` is copied, not decoded. AAC, Opus, and VP9 are sniffed and unsupported. |
| `export-avcbox` | Boxes Annex B H.264 pictures into a non-fragmented MP4. It does not encode H.264. |
| `export-vorbis` | One short mono 48 kHz Vorbis packet: silence, or two half-scale 440 Hz frames plus the next 32 samples. |
| `export-mp3` | One MPEG-1 Layer III frame, mono, 44.1 kHz, 1152 samples. No bit reservoir. |
| `export-vp9` | Decode one generated keyframe at 8×8 through 2048×2048. Inter frames, extra frames, and VP8 are unsupported. |

`scripts/outside_score.py` scores files already stored in this repo, plus
real files you drop in a local `corpus/` folder (gitignored, never pushed;
`--corpus` or `EXPORT_CORPUS` points elsewhere), against the Python app. It
may call a separate program named by `EXPORT_RUST_ORACLE`. It is not the server.

Crate tests: `cargo test --workspace --offline`. Python tests: `pytest`.

## Links

- [docs/internals.md](docs/internals.md) — full capability tables, architecture, every env var with defaults, compatibility matrix, limits, licences
- [CHANGELOG.md](CHANGELOG.md)
- [SECURITY.md](SECURITY.md)

The glue code is MIT; bundled tools carry their own licences (Pandoc GPL,
Ghostscript AGPL, …) — see [docs/internals.md](docs/internals.md#license).
