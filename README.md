# export

A personal web tool that converts, extracts, compresses, and transforms files —
from URLs (YouTube, Twitter, etc.) and from local uploads. ~85 conversion paths
across image / video / audio / document / PDF / archive, all running locally
with open-source libraries. Designed to replace the half-dozen
freeconvert/ilovepdf/yt-mp3 sites you'd otherwise tab through, plus
NoteGPT-style URL-to-transcript that those sites don't offer.

```
http://localhost:8500   →   single password   →   paste a URL or drop a file
```

The UI has a **show what's supported** link at the bottom that hits
`GET /api/capabilities` (public, no auth) and renders the live capability map.
That's always the source of truth — the table below is the human-friendly
version.

## Capabilities

### From a URL

| Source | Outputs |
|---|---|
| YouTube, Facebook (public), Vimeo, SoundCloud, Bandcamp, Twitch, Reddit, TED, Dailymotion, PeerTube, Mixcloud, +~1800 more sites via [yt-dlp](https://github.com/yt-dlp/yt-dlp) | `mp3`, `mp4`, **`notes`** (NoteGPT-style markdown), `txt` (transcript), `srt` / `vtt` (subtitles) |
| Image-rich URLs (Wikimedia Commons, Reddit galleries, Pinterest, etc.) via [gallery-dl](https://github.com/mikf/gallery-dl) | `images` (single file or zip) |
| **Login-walled** (Instagram, locked X, private FB groups) | Set `EXPORT_YTDLP_BROWSER=chrome` to read cookies from your logged-in browser |
| **IP-blocked** (TikTok, Rumble) | Set `EXPORT_HTTP_PROXY=socks5://127.0.0.1:9050` (Tor) or any residential proxy |

### From an uploaded file

| Input type | Convert to | Other operations |
|---|---|---|
| **Image** (`.jpg .jpeg .png .webp .avif .heic .heif .tiff .bmp .gif .ico`) | `jpeg, png, webp, avif, tiff, ico, pdf, txt (OCR), nobg (transparent PNG via rembg), svg (vectorize via vtracer)` | `shrink` (with quality slider) |
| **Video** (`.mp4 .mov .webm .mkv .avi .m4v .flv .wmv .3gp .ts .mts .m2ts .ogv`) | `mp4, mov, webm, mkv, gif, txt, srt, vtt, notes (Whisper transcript)` | `shrink` (SVT-AV1 re-encode with quality slider) |
| **Audio** (`.mp3 .m4a .wav .flac .ogg .aac .opus .aiff .aif .wma`) | `mp3, m4a, wav, flac, ogg, opus, aac, txt, srt, vtt, notes (Whisper), denoise (noisereduce), stems (Demucs vocal/drum/bass/other split)` | — |
| **PDF** (`.pdf`) | `png, jpeg, webp, txt (auto-OCR fallback for scanned), docx, pdfa (searchable PDF/A via OCRmyPDF)` | `shrink`, `merge`, `split`, `rotate`, `watermark`, `encrypt`, `decrypt` |
| **Documents** (`.md .html .htm .rtf .epub .txt .tex .org .rst`) | `md, html, docx, rtf, epub, txt, odt, pdf, mp3, m4a, wav (TTS via macOS say or piper-tts)` | — |
| **Office** (`.docx .doc .pptx .ppt .xlsx .xls .odp .ods .odt`) | `pdf` (via LibreOffice headless) | — |

### Multi-file operations

| Action | Inputs | Output |
|---|---|---|
| **archive** (multi-file) | any files | `zip`, `7z`, `tar.zst` (3.8× smaller than tar.gz at zstd level 10), `tar.gz`, `tar.bz2`, `tar.xz` |
| **single-file compress** | one file | `gz`, `bz2`, `xz`, `zst` (raw stream compression — no tar wrapper) |
| **PDF merge** | 2+ PDFs | single combined PDF |

### Lossy shrink levels

| Input | `low` | `medium` | `high` |
|---|---|---|---|
| Image (any) | quality 50 | quality 70 | quality 85 |
| Video | SVT-AV1 CRF 45, preset 8 | CRF 35, preset 6 | CRF 28, preset 4 |
| PDF | Ghostscript `/screen` | `/ebook` | `/printer` |

The **advanced disclosure** under shrink and image/video convert exposes a
1–100 quality slider that overrides these presets. The slider's "estimated
output size" is type-aware (image curve, AV1 floor for video, Ghostscript
preset bands for PDF). After processing, the actual `original → output (n%)`
delta is shown next to the download link, with a green badge if smaller.

A **shrink more ↓** button appears after a successful shrink — clicks step
quality down by 10 (floor 10), Clop-style, with cumulative size displayed
relative to the original input.

## Quick start

### macOS

```bash
git clone <this repo> export && cd export
bash scripts/install-macos.sh
cp .env.example .env && $EDITOR .env   # set EXPORT_PASSWORD=...
bash scripts/run.sh
```

Open `http://localhost:8500` and enter your password. The first request that
needs a downloaded model (Whisper, rembg, demucs) will pause for ~30s while it
fetches and caches.

### Linux (Debian/Ubuntu)

```bash
git clone <this repo> /opt/export && cd /opt/export
bash scripts/install-linux.sh    # ~5 min, ~1.5 GB system deps
cp .env.example .env             # set EXPORT_PASSWORD + EXPORT_PIPER_VOICE
bash scripts/run.sh
```

For a real deployment, see [scripts/export.service](scripts/export.service)
(systemd unit) and [scripts/nginx-export.conf](scripts/nginx-export.conf)
(reverse proxy for `export.bygelo.com`).

## Architecture

Single-process FastAPI app. Job state lives in a Python dict; downloaded files
in `$EXPORT_DOWNLOADS` (default `/tmp/export/`). A lifespan-managed cleanup loop
deletes job dirs older than 1 h every 15 min.

```
┌─────────────────────────────────────────────────┐
│ uvicorn (port 8500)                             │
│                                                 │
│   ┌───────────────────────────────────────┐     │
│   │ FastAPI (app/main.py)                 │     │
│   │   /api/jobs       → URL extraction    │     │
│   │   /api/upload     → file conversion   │     │
│   │   /api/compress   → multi-file zip    │     │
│   │   /api/shrink     → lossy size cut    │     │
│   │   /api/pdf/merge  → multi-PDF merge   │     │
│   │   /api/pdf/op     → split/rotate/...  │     │
│   │   /api/jobs/:id   → poll status       │     │
│   │   /api/files/:id  → download          │     │
│   │   /api/capabilities → format listing  │     │
│   └───────────────────────────────────────┘     │
│                  ↕                              │
│   In-memory `jobs` dict + BackgroundTasks       │
│                  ↕                              │
│   /tmp/export/{job_id}/output_file              │
└─────────────────────────────────────────────────┘
```

### Adding a new conversion (the bridge pattern)

We deliberately don't use a generic graph router (that's the
[convert.to.it](https://github.com/p2r3/convert) approach — see the sibling
`autoheal/` project for the writeup on why we picked the simpler model).
Instead, each conversion is an explicit function with an entry in the dispatch.

To add one new bridge in `app/jobs.py`:

1. Add a format set: `MY_NEW_BRIDGE = {"target_fmt"}`
2. Write a helper: `def _do_my_thing(input_path, out_dir) -> Path: ...`
3. Add a branch in `run_convert_job`'s if/elif chain
4. Re-export the format set from `app/main.py`'s import + `ALLOWED_UPLOAD_FORMATS`
5. Add `target_fmt` to `static/index.html`'s `TYPES.<input>.convertTo` array

That's the whole pattern. ~30 lines per new conversion. No abstraction needed
until you have ≥10 handlers in one input domain.

### Tech stack

| Layer | What |
|---|---|
| HTTP | FastAPI + uvicorn |
| Frontend | Single HTML page, vanilla JS, no framework |
| Job state | Python dict + FastAPI `BackgroundTasks` |
| Image | Pillow + pillow-heif + pillow-avif-plugin |
| Video / Audio | ffmpeg (libsvtav1, libopus, libmp3lame, libvorbis, libx264, aac) |
| URL extraction | yt-dlp + gallery-dl |
| Documents | Pandoc + WeasyPrint + LibreOffice headless |
| PDF | pypdf + pikepdf + reportlab + pdfplumber + pdf2docx + pdf2image + Ghostscript + OCRmyPDF |
| OCR | Tesseract (via pytesseract) |
| STT | faster-whisper (default `distil-large-v3`) |
| TTS | macOS `say` / `piper-tts` (Linux) |
| Audio AI | Demucs (htdemucs) for stems, noisereduce for denoise |
| Image AI | rembg (u2net) for background removal, vtracer for vectorization |
| Compression | zstandard, py7zr, stdlib zipfile + tarfile |

## Configuration

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_PASSWORD` | `changeme` | Required. Sent as `X-Password` header. |
| `EXPORT_DOWNLOADS` | `/tmp/export` | Where job output lives. |
| `EXPORT_WHISPER_MODEL` | `distil-large-v3` | Any faster-whisper model name. |
| `EXPORT_WHISPER_MODEL_DIR` | (auto) | Override model cache dir. |
| `EXPORT_PIPER_VOICE` | (none) | Required on Linux for TTS — path to a `.onnx` voice. |
| `EXPORT_YTDLP_COOKIES` | (none) | Path to a Netscape-format `cookies.txt`. Unlocks login-walled Instagram, locked X tweets, private FB groups. |
| `EXPORT_YTDLP_BROWSER` | (none) | Easier alternative — name of an installed browser (`chrome`, `firefox`, `safari`, `edge`, `brave`, `opera`, `vivaldi`). yt-dlp reads cookies live from the browser's cookie database. |
| `EXPORT_HTTP_PROXY` | (none) | Routes URL extraction through a proxy. Beats IP blocks (TikTok, Rumble) and geo-restrictions. Format: `http://user:pw@host:port` or `socks5://127.0.0.1:9050` (Tor). |
| `EXPORT_WATCH_DIR` | (none) | Folder watch mode (Clop-style). Anything dropped in this folder is auto-shrunk into `<dir>/_optimized/`. Image/video/PDF only. |
| `EXPORT_MAX_UPLOAD_MB` | `0` (unlimited) | Per-file upload cap in MB. Set to a positive integer only if exposing publicly. |
| `EXPORT_MAX_COMPRESS_MB` | `0` (unlimited) | Combined cap for multi-file compress / merge. |
| `DYLD_FALLBACK_LIBRARY_PATH` | `/opt/homebrew/lib` (macOS) | WeasyPrint needs to find libgobject. |

### URL extractor notes

| Failure mode | What you'll see | Fix |
|---|---|---|
| **YouTube** missing formats | `JS challenge solving failed` warning | Install `deno` (handled by install scripts) |
| **IP-blocked** (TikTok, Rumble, geo-locked news) | "Your IP is blocked" / 403 | `EXPORT_HTTP_PROXY=socks5://127.0.0.1:9050` (Tor) or any residential proxy |
| **Login-walled** (Instagram, private FB groups, locked X tweets, members-only) | "Login required" / "API not granting access" | `EXPORT_YTDLP_BROWSER=chrome` (reads cookies from your logged-in browser — easiest), or `EXPORT_YTDLP_COOKIES=/path/to/cookies.txt` for headless setups |
| **Stale content** | "no longer available" / 404 | The content was deleted — no fix, get a working URL |

The error messages surfaced in the UI are humanized — instead of a 200-line yt-dlp stack trace you'll see one of these short hints.

## Limits

- Single file upload: **unlimited** by default (set `EXPORT_MAX_UPLOAD_MB=N` to cap)
- Multi-file (compress / merge): **unlimited** by default (set `EXPORT_MAX_COMPRESS_MB=N` to cap)
- Job dirs auto-deleted after 1 hour (lifespan-managed cleanup loop)
- Single-process — concurrent jobs share CPU; not designed for multi-tenant load
- Whisper is single-instance — concurrent transcriptions queue serially
- LibreOffice cold-starts each office→PDF call (~3 s); could daemonize with
  `unoserver` later if it matters
- Job dirs auto-deleted after 1 hour
- Single-process — concurrent jobs share CPU; not designed for multi-tenant load
  (deferred until needed; would migrate to `arq + Redis` per the research)
- Whisper is single-instance — concurrent transcriptions queue serially
- LibreOffice cold-starts each office→PDF call (~3 s); could daemonize with
  `unoserver` later if it matters

## Error messages

When URL extraction fails, the UI surfaces a one-line humanized message
instead of the raw 200-line yt-dlp stack trace. Examples:

| What you'll see | What it means |
|---|---|
| "Login required. Set EXPORT_YTDLP_BROWSER=chrome…" | Site needs cookies (Instagram, private FB, locked X) |
| "This platform blocks your IP. Set EXPORT_HTTP_PROXY…" | TikTok / Rumble / geo-block — needs a proxy |
| "This content has been deleted by its uploader" | Stale URL — no fix available |
| "Page not found — the URL may be wrong, expired, or deleted" | 404 — check the URL |
| "Geo-restricted — set EXPORT_HTTP_PROXY to a server in an allowed region" | Geographic block |

## Security & deployment scope

`export` is built for **personal use** (localhost or your private network). If
you're considering exposing it on the public internet — even just to share
with a friend — read [SECURITY.md](SECURITY.md) first. It contains an honest
audit of the current security posture and a phased hardening roadmap (Phase 1:
where we are; Phase 2: ~90 min of work for "trusted small group on the open
internet"; Phase 3: real product territory).

## License

This wires together a lot of OSS tools with various licenses. The glue code in
this repo is MIT. Component licenses to be aware of in any redistribution:

| Component | License |
|---|---|
| Pandoc | GPL-2.0+ |
| LibreOffice | MPL-2.0 |
| Ghostscript | AGPL |
| rembg u2net default model | Apache-2.0 |
| BiRefNet / RMBG-2.0 (if you swap them in) | CC-BY-NC-4.0 (non-commercial) |
| yt-dlp | Public domain (Unlicense) |
| Marker / Nougat (if added) | GPL / CC-BY-NC |
