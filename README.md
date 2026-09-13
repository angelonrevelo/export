# export

A personal web tool that converts, extracts, compresses, and transforms files —
from URLs (YouTube, Twitter, etc.) and from local uploads. 180+ conversion
paths across image / video / audio / document / PDF / spreadsheet / subtitle
/ font / comic / archive, all running locally with open-source libraries
(optional Groq cloud for fast transcription). Multi-language OCR / STT / TTS
with auto-detect, chain outputs (kit / captioned / frames), and an explicit
offline mode. Designed to replace the half-dozen freeconvert/ilovepdf/yt-mp3
sites you'd otherwise tab through, plus NoteGPT-style URL-to-transcript that
those sites don't offer.

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
| YouTube, Facebook (public), Vimeo, SoundCloud, Bandcamp, Twitch, Reddit, TED, Dailymotion, PeerTube, Mixcloud, +~1800 more sites via [yt-dlp](https://github.com/yt-dlp/yt-dlp) | `mp3`, `mp4`, **`notes`** (NoteGPT-style markdown), `kit` (zip of txt+srt+vtt+notes), `txt` / `srt` / `vtt`, `pdf` / `docx` / `epub` (transcript as a document), `captioned` (burned-in subs), `frames`, `subs` |
| Image-rich URLs (Wikimedia Commons, Reddit galleries, Pinterest, etc.) via [gallery-dl](https://github.com/mikf/gallery-dl) | `images` (single file or zip) |
| **Login-walled** (Instagram, locked X, private FB groups) | Set `EXPORT_YTDLP_BROWSER=chrome` to read cookies from your logged-in browser |
| **IP-blocked** (TikTok, Rumble) | Set `EXPORT_HTTP_PROXY=socks5://127.0.0.1:9050` (Tor) or any residential proxy |

### From an uploaded file

| Input type | Convert to | Other operations |
|---|---|---|
| **Image** (`.jpg .jpeg .png .webp .avif .heic .heif .tiff .bmp .gif .ico .jp2 .psd .tga .pcx .eps .icns .jxl .svg` + camera RAW `.dng .cr2 .cr3 .nef .arw .raf .orf .rw2 .pef .srw`) | `jpeg, png, webp, avif, jxl, tiff, bmp, gif, ico, icns, jp2, pdf, txt (OCR via Tesseract, auto language), nobg (transparent PNG via rembg), svg (vectorize via vtracer)`. Also via OCR→pandoc chain: `docx, md, html, rtf, epub, odt, tex`. JXL and RAW need the optional `.[media]` extra. | `shrink` (with quality slider) |
| **Video** (`.mp4 .mov .webm .mkv .avi .m4v .flv .wmv .3gp .ts .mts .m2ts .ogv .gif .mpg .mpeg .vob .asf .mxf`) | Direct: `mp4, mov, webm, mkv, gif, png` (one still frame — pictures of frames, not a transcript), `mp3, m4a, m4b, wav, flac, ogg, opus, aac` (audio extraction via ffmpeg `-vn`), `audiogram`, `txt, srt, vtt, notes, kit` (Whisper — local multilingual `large-v3-turbo` or Groq), `captioned` (burn-in subs), `frames` (zip of 1 fps PNGs, cap 120), `subs` (embedded stream or Whisper fallback), `pdf / docx / epub` (transcript as a document). Convert-to in the UI is the **reachable set** (composed hops, e.g. video → `png` → `jpeg`), not only this family list. `.gif` is both image and video. | `shrink` (libx264 re-encode with quality slider — universal playback) |
| **Audio** (`.mp3 .m4a .wav .flac .ogg .aac .opus .aiff .aif .wma .amr .3ga .m4b .ac3 .caf`) | `mp3, m4a, m4b, wav, flac, ogg, opus, aac, txt, srt, vtt, notes, kit (Whisper), denoise (noisereduce), stems (Demucs), audiogram, pdf / docx / epub` (transcript as a document) | — |
| **PDF** (`.pdf`) | `png, jpeg, webp, txt (auto-OCR fallback, auto language), md, notes, docx, html, epub, rtf, odt, tex, pdfa (searchable PDF/A via OCRmyPDF with the detected language pack), mp3/m4a/wav (audiobook via TTS, voice matched to detected language), audiogram` | `shrink`, `merge`, `split`, `rotate`, `watermark`, `encrypt`, `decrypt` |
| **Documents** (`.md .markdown .html .htm .rtf .epub .txt .tex .org .rst .ipynb .adoc .asciidoc .textile .docbook .fb2`) | `md, html, docx, rtf, epub, txt, odt, tex, pptx (Pandoc slideshow), pdf, mp3, m4a, wav (TTS via macOS say or piper-tts)` | — |
| **Office** (`.docx .doc .pptx .ppt .xlsx .xls .odp .ods .odt`) | `pdf` (LibreOffice headless), `png/jpeg/webp` (each page/slide rasterized at 200 DPI via office→pdf→image, multi-page → zip). For `.docx / .odt / .pptx` (pandoc-readable): `md, html, rtf, epub, txt, odt, tex` direct via pandoc. For all office formats: `txt` via office→pdf→text chain, plus `mp3/m4a/wav` (audiobook via TTS), `audiogram` (waveform mp4 of the TTS render). | — |
| **Spreadsheet** (`.csv .tsv .xlsx .json .yaml .yml .toml .xml`) | `csv, tsv, xlsx, json, yaml, toml, txt, md, html, pdf`. Nested JSON/YAML is auto-tolerated — walks 3 levels deep for an inner `list[dict]`; otherwise degrades to a 2-col `key/value` sheet. `html` renders JSON as a styled `<dl>` tree (`html`/`body`/`content`/`rendered`/`markup` keys render raw). `pdf` is HTML → WeasyPrint. | — |
| **Comic** (`.cbz`) | `pdf` (zip of images → multi-page PDF, page order = zip order) | — |
| **Subtitle** (`.srt .vtt .ass .ssa .sub .sbv`) | `srt, vtt, ass, ssa, sub, sbv` (cross-conversion via pysubs2). Also: `txt / md / notes` (transcript dumps; notes groups paragraphs by ≥1.5 s gap between cues), `docx / rtf / epub / odt / tex / html / pdf` (printable transcript via pandoc), `mp3 / m4a / wav` (TTS read-aloud), `audiogram` (waveform mp4 of the TTS render). | — |
| **Font** (`.ttf .otf .woff .woff2`) | `ttf, otf, woff, woff2` (container conversion via fontTools — TTF↔OTF is a no-op, woff/woff2 wrap or unwrap). Also: `png / jpeg / webp` specimen — renders the font name + a large AaBbCc preview + two pangrams as an image (woff/woff2 routed through a fontTools TTF intermediate so older Pillow builds still work). | — |

### File meta (dates, tags, unlock, scrub, comment, hidden)

Unusual ops that convert sites skip.

| Action | What it does |
|---|---|
| `date` | Set filesystem **modified** (and **created** on macOS) to a datetime, `now`, or the image’s EXIF date. **N files** apply the same datetime to every copy on disk. |
| `tag` | Write **Finder tags** (`red, work`, …) as `com.apple.metadata:_kMDItemUserTags`. **N files** get the same tags. |
| `unlock` | Strip `com.apple.quarantine` and the uchg lock flag. |
| `scrub` | Strip EXIF GPS/serial, `kMDItemWhereFroms`, and leftover xattrs. JPEG scan data is not re-encoded (pixels stay). |
| `comment` | Set or clear Spotlight Finder comment and download **Where From** URL. Empty input clears. |
| `hidden` | `hide` / `show` the macOS hidden flag (`UF_HIDDEN`), or `clear-icon` (FinderInfo custom-icon bit + resource fork). |
| `decrypt` | PDF, password-zip / cbz, password-`.7z`, encrypted `.docx` / `.xlsx` / `.pptx`. |

A browser **download** typically resets mtime and drops xattrs. On a Mac, use **show in Finder** after the job — that selects the job copy under `EXPORT_DOWNLOADS` so you can move it with tags and dates intact. Bulk date/tag also writes a `batch.zip` for download; the per-file copies stay next to it.

### Multi-file operations

| Action | Inputs | Output |
|---|---|---|
| **archive** (multi-file) | any files | `zip`, `7z`, `tar.zst` (3.8× smaller than tar.gz at zstd level 10), `tar.gz`, `tar.bz2`, `tar.xz` |
| **single-file compress** | one file | `gz`, `bz2`, `xz`, `zst` (raw stream compression — no tar wrapper) |
| **PDF merge** | 2+ PDFs | single combined PDF (pypdf) |
| **Image batch — multi-shape** | 2+ images | `pdf` (multi-page), `gif` (animated slideshow, 2 s/frame), `mp4` (libx264 slideshow, 3 s/frame). EXIF rotation auto-applied; per-tile rotate (↺ / ↻) and remove (✕) controls. Routes through `/api/images/batch`. |
| **Bulk shrink / resize** | 2+ images or 2+ videos | Same shrink/resize params (`level`, `quality`, `max_dim`) applied to every file in the batch. Returns a zip of the N processed outputs. Routes through `/api/batch/shrink`. Useful for "make all my phone photos 1080p webp" or "shrink this folder of clips to 720p for sharing." |
| **Generic batch convert** | 2+ files of the same type | Each file runs through the normal single-file convert pipeline. Output mode toggle: **1:1** (zip of N converted files) or **merge** (one combined file). Routes through `/api/batch`. **39 mergeable target formats**, smart per-family: PDF (pypdf merge); text-like `txt md csv json html rtf …` (concat with `--- filename ---` separators; JSON wraps as a real array; CSV/TSV de-dupes header rows); subtitles `srt vtt ass ssa sub sbv` (pysubs2 with cumulative time-shift so cues don't overlap); audio `mp3 m4a wav flac ogg opus aac` and video `mp4 mov webm mkv` (ffmpeg concat with `-c copy` fast path + re-encode fallback); images `jpeg png webp avif tiff bmp` (tile grid — `cols=⌈√N⌉` rows of max-cell-size, smaller images letterboxed); xlsx (multi-sheet workbook, one sheet per input file, names truncated to 31-char cap). Non-mergeable targets fall back to zip mode. |

### Lossy shrink levels

| Input | `low` | `medium` | `high` | `resize` |
|---|---|---|---|---|
| Image (any) | quality 50 | quality 70 | quality 85 | quality 95 (near-lossless); max-dim, scale%, **or** a `target_byte` cap |
| Video | libx264 CRF 30, preset fast | CRF 26, preset medium | CRF 22, preset slow | CRF 18, preset medium; max-dim, scale%, **or** a `target_byte` cap |
| PDF | Ghostscript `/screen` | `/ebook` | `/printer` | — (resize n/a for PDF) |

The **advanced disclosure** under shrink and image/video convert exposes a
1–100 quality slider *and* an **exact size** cap (`target_byte`, presets include
**20 MB = 20_000_000 bytes**). When a byte cap is set, the predicted size is
that single number (not a lo–hi range) and the delivered file **equals** the
cap: the encoder aims under budget, then a container-legal pad (JPEG COM /
PNG tEXt / MP4 `free` box) lands on the exact count. A source already at or
under the cap is copied and **never grown**. Without a cap, the quality slider
still shows a source-aware range. After processing, the actual
`original → output (n%)` delta is shown next to the download link, with a
green badge if smaller.

Video shrink defaults to **libx264 (h.264)** for universal playback —
QuickTime/Preview/iOS Safari/Discord/messaging apps all accept it natively.
Set `EXPORT_SHRINK_VIDEO_CODEC=av1` to use SVT-AV1 (~20–30% smaller files
at matched quality, but breaks on some consumer players like Preview seeks
and Discord embeds — use only when your playback chain explicitly supports
AV1: recent Chrome/Firefox, VLC, mpv).

A **shrink more ↓** button appears after a successful shrink — clicks step
quality down by 10 (floor 10), Clop-style, with cumulative size displayed
relative to the original input.

## Quick start

### macOS

```bash
git clone <this repo> export && cd export
bash scripts/install-macos.sh
npm run dev    # or: bash scripts/run.sh
```

`run.sh` auto-creates `.env` with password `1234` on first launch.
For muscle-memory comfort, the project ships a `package.json` with `dev`,
`start`, and `watch` scripts that all wrap `scripts/run.sh` — no actual Node
deps, just the one-line shim.

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
deletes job dirs older than 6 h every 15 min.

```
┌─────────────────────────────────────────────────┐
│ uvicorn (port 8500)                             │
│                                                 │
│   ┌───────────────────────────────────────┐     │
│   │ FastAPI (app/main.py)                 │     │
│   │   /api/jobs            → URL extract  │     │
│   │   /api/upload          → file convert │     │
│   │   /api/compress        → multi-file   │     │
│   │   /api/shrink          → lossy size   │     │
│   │   /api/pdf/merge       → multi-PDF    │     │
│   │   /api/pdf/op          → split/rot/.. │     │
│   │   /api/images/batch    → img→pdf/gif/mp4│   │
│   │   /api/images/to-pdf   → legacy       │     │
│   │   /api/batch           → N→zip or merge│    │
│   │   /api/batch/shrink    → N→zip of N shr│    │
│   │   /api/thumbnail       → server prev. │     │
│   │   /api/jobs/:id        → poll status  │     │
│   │   /api/files/:id       → download     │     │
│   │   /api/capabilities    → format list  │     │
│   │   /api/reachable       → hop targets  │     │
│   └───────────────────────────────────────┘     │
│                  ↕                              │
│   In-memory `jobs` dict + BackgroundTasks       │
│                  ↕                              │
│   /tmp/export/{job_id}/output_file              │
└─────────────────────────────────────────────────┘
```

### Adding a new conversion (the bridge pattern)

Each **one-hop** conversion is still an explicit function in
[app/jobs.py](app/jobs.py). After a file is chosen, convert-to is the
**reachable set**: BFS over those hops in [app/graph.py](app/graph.py),
including multi-hop (video → `png` still → `jpeg`). Direct one-hop always
wins, so video→PDF stays the existing transcript PDF (not frames-as-PDF).
A format with no path is not offered; the convert entry raises `no bridge`
rather than stuffing source bytes into the target container. A suffix in
more than one family (`.gif` is image and video) unions every one-hop those
families already have, so gif→`mp4` stays a direct convert-to.

`GET /api/reachable?ext=mp4` returns `{ "format": [...], "kind": { "image": [...], ... } }`
(singular collection keys). The UI convert-to picker is grouped by kind
(not a flat A–Z dump). Query is pure — no ffmpeg. Execution walks the path by
calling `_convert_one` per hop. Cheap one-hop paths (PNG→JPEG, YAML→JSON)
stay the first step and do not load Whisper / rembg / NLLB.

To add one new **one-hop** bridge in [app/jobs.py](app/jobs.py):

1. Add a format set: `MY_NEW_BRIDGE = {"target_fmt"}`
2. Write a helper: `def _do_my_thing(input_path, out_dir) -> Path: ...`
3. Add a branch in `_convert_one`'s if/elif chain
4. Re-export the format set from [app/main.py](app/main.py)'s import + `ALLOWED_UPLOAD_FORMATS`
5. Add `target_fmt` to the matching `convert_outputs` list in `/api/capabilities`
6. `TYPES.<input>.convertTo` in [static/index.html](static/index.html) is only a fallback if `/api/reachable` fails — the picker fills from the reachable set

That's the whole pattern. ~30 lines per new one-hop. Multi-hop targets
appear automatically once both edges exist.

For multi-file batch operations:

- **Generic homogeneous batch** (`/api/batch`) handles "N files of the same
  type → zip-of-N or one merged file" using the shared `_convert_one` dispatch
  per file. To make a new target format **mergeable**, extend
  [app/jobs.py](app/jobs.py)'s `_merge_outputs` with a new `if fmt in …:`
  branch (mirror the existing PDF / xlsx / subtitle / image-tile / text-concat
  / ffmpeg-concat branches), add the format to `MERGEABLE_FORMATS`, and add it
  to `MERGEABLE_OUTPUT_FORMATS` in [static/index.html](static/index.html) so
  the frontend's merge toggle appears.
- **Special multi-shape batches** (like `/api/images/batch`'s pdf/gif/mp4
  outputs) get their own dedicated endpoint: `list[UploadFile]` → a `_batch_*`
  helper that produces an output shape that doesn't fit the "N → 1 of same fmt"
  pattern (multi-page PDF, animated GIF, slideshow MP4).

### Tech stack

| Layer | What |
|---|---|
| HTTP | FastAPI + uvicorn |
| Frontend | Single HTML page, vanilla JS, no framework |
| Job state | Python dict + FastAPI `BackgroundTasks` (single-process); folder-watch via `watchdog` |
| Image | Pillow (incl. PSD/TGA/PCX/EPS/ICNS/JP2 read) + pillow-heif + pillow-avif-plugin |
| Video / Audio | ffmpeg (libsvtav1, libx264, libvpx-vp9, libopus, libmp3lame, libvorbis, aac, flac); audiogram pipeline uses `showwavespic` + `dynaudnorm` for the static waveform and `overlay` with `eval=frame` for the sweeping playhead |
| URL extraction | yt-dlp[curl-cffi] (~1800 sites) + gallery-dl + deno (YouTube JS challenge solver) |
| Documents | Pandoc (md/html/docx/rtf/epub/odt/tex/pptx/ipynb/...) + WeasyPrint + LibreOffice headless |
| PDF | pypdf + pikepdf + reportlab + pdfplumber + pdf2docx + pdf2image + Ghostscript + OCRmyPDF |
| Spreadsheet | openpyxl + stdlib csv/json |
| Subtitle | pysubs2 |
| Font | fontTools + brotli (for woff2) |
| OCR | Tesseract (via pytesseract); applied as auto-fallback when scanned PDFs return empty pdfplumber output |
| STT | faster-whisper local (default `large-v3-turbo`, multilingual) — or Groq cloud `whisper-large-v3-turbo` when `EXPORT_GROQ_API_KEY` is set and `EXPORT_OFFLINE` is unset. `distil-large-v3` is English-only and is no longer the default. |
| TTS | macOS `say` (voice picked from detected language) / `piper-tts` (Linux; `{lang}_*.onnx` in `voices/` or `EXPORT_PIPER_VOICE`) |
| Language | `langdetect` + Unicode script ranges + Tesseract OSD. UI language dropdown pins OCR/STT/TTS per job. |
| Translate | NLLB-200 distilled 600M via CTranslate2 int8 (200 languages including Filipino `tgl_Latn`). UI **to** dropdown; format `translate` dumps translated txt. Applies to OCR / STT / subtitles / TTS / notes / kit / captioned. |
| Offline | `EXPORT_OFFLINE=1` refuses URL extraction and Groq, and blocks HuggingFace model downloads. Prefetch with `bash scripts/prefetch.sh`. |
| Audio AI | Demucs (htdemucs) for vocal/drum/bass/other stem separation, noisereduce for spectral denoise |
| Image AI | rembg (u2net) for background removal, vtracer for raster→SVG |
| Compression | zstandard (`tar.zst` + single-file `.zst`), py7zr (`.7z`), stdlib zipfile + tarfile (gzip/bzip2/xz) |
| Browser impersonation | curl_cffi (TLS fingerprint mimicry for TikTok/IG) |

## Configuration

Every knob is set via env var (read from `.env` if present — auto-created on
first run with password `1234`). Setting one to its default has the same
effect as not setting it.

### Required

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_PASSWORD` | `1234` | Required. Sent as `X-Password` header. Localhost stays `1234`. Public/trusted-group: 32+ chars (`openssl rand -base64 24`). |

### Storage & limits

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_DOWNLOADS` | `/tmp/export` | Where job output lives. Mount as tmpfs for RAM-backed ephemeral storage on a VPS. |
| `EXPORT_MAX_UPLOAD_MB` | `0` (unlimited) | Per-file upload cap in MB. Public Phase 2: `500`. |
| `EXPORT_MAX_COMPRESS_MB` | `0` (unlimited) | Combined cap for multi-file compress / merge / batch. Public Phase 2: `2000`. |
| `EXPORT_RATE_LIMIT_PER_MINUTE` | `30` | Mutating `/api` POSTs per IP. `0` disables. Unittest auto-off unless this env is set. |
| `EXPORT_CLEANUP_AGE_SECONDS` | `21600` (6 h) | Age threshold for the lifespan cleanup loop. Lower for more-ephemeral storage; raise if you process long files and download later. |
| `EXPORT_CLEANUP_INTERVAL_SECONDS` | `900` (15 min) | How often the cleanup loop sweeps. |
| `EXPORT_DOWNLOAD_AUTODELETE` | (off) | When `1` / `true` / `yes`, the output file + job dir are deleted the *instant* the download stream finishes. Maximum-ephemeral mode. |

### Folder watch

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_WATCH_DIR` | (none) | Folder watch mode (Clop-style). Anything image/video/PDF dropped in this folder is auto-shrunk into `<dir>/_optimized/`. |

### Whisper STT (local CPU)

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_WHISPER_MODEL` | `large-v3-turbo` | Any faster-whisper model name. Default is multilingual. `distil-large-v3` and `*.en` are English-only. Smaller = less disk + faster but lower quality. |
| `EXPORT_WHISPER_MODEL_DIR` | (auto) | Override model cache dir (defaults to `~/.cache/huggingface`). |
| `EXPORT_LANGUAGE` | (auto) | Pin OCR / STT / TTS to an ISO 639-1 code (`en`, `tl`, `es`, `zh`, …). The UI dropdown sends the same value per job. |
| `EXPORT_OCR_LANG` | (auto) | Force a Tesseract `-l` string (`eng+fil`). Overrides pack picking. |
| `EXPORT_OFFLINE` | (off) | When `1`, URL extraction and Groq are refused and Whisper/rembg/demucs/NLLB will not download models. Prefetch first: `bash scripts/prefetch.sh`. |
| `EXPORT_TRANSLATE_TO` | (off) | Process-wide NLLB target (ISO 639-1). The UI **to** dropdown sends the same value per job. |

### Whisper STT (Groq cloud, optional)

When `EXPORT_GROQ_API_KEY` is set and `EXPORT_OFFLINE` is unset, all transcription
(audio/video → txt / srt / vtt / notes / kit) routes to Groq's hosted Whisper
API instead of local CPU. ~200× realtime on `whisper-large-v3-turbo`. Free tier
covers 8 h/day of audio; paid tier is $0.04/h. Offline mode ignores the key.

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_GROQ_API_KEY` | (unset = local Whisper) | Get a key at https://console.groq.com/keys. Presence is the only switch — falls back to local Whisper when unset. |
| `EXPORT_GROQ_MODEL` | `whisper-large-v3-turbo` | Other valid: `whisper-large-v3`. |
| `EXPORT_GROQ_CHUNK_SECONDS` | `1200` (20 min) | Audio is pre-encoded to mp3 mono 16 kHz @ 64 kbps then segmented at this interval (keeps each chunk well under Groq's 25 MB free-tier upload cap). Per-chunk timestamps are offset on stitch so srt/vtt timecodes stay correct across the join. |

### Piper TTS

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_PIPER_VOICE` | (none) | Required on Linux for TTS — path to a `.onnx` voice. macOS uses built-in `say` instead. `install-linux.sh` downloads `voices/en_US-lessac-medium.onnx` for you. |
| `EXPORT_PIPER_VOICE_DIR` | (none) | Extra directory of `{lang}_*.onnx` Piper voices; the TTS picker matches on filename prefix. |

### Translation (NLLB)

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_NLLB_MODEL` | `entai2965/nllb-200-distilled-600M-ctranslate2` | CTranslate2 int8 snapshot (200 languages including Filipino). First use downloads ~1.2 GB. |
| `EXPORT_NLLB_TOKENIZER` | `facebook/nllb-200-distilled-600M` | Hugging Face tokenizer id paired with the CT2 weights. |
| `EXPORT_NLLB_MODEL_DIR` | (auto) | Local dir of already-converted CT2 weights; used when the snapshot download is skipped. |

### URL extraction (yt-dlp / gallery-dl)

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_YTDLP_BROWSER` | (none) | Easiest cookie source — name of an installed browser (`chrome`, `firefox`, `safari`, `edge`, `brave`, `opera`, `vivaldi`). yt-dlp reads cookies live. |
| `EXPORT_YTDLP_COOKIES` | (none) | Headless alternative — Netscape-format `cookies.txt` path. |
| `EXPORT_HTTP_PROXY` | (none) | Routes URL extraction through a proxy. Beats IP blocks (TikTok, Rumble) and geo-restrictions. Format: `http://user:pw@host:port` or `socks5://127.0.0.1:9050` (Tor). |
| `EXPORT_YTDLP_IMPERSONATE` | `auto` | Browser impersonation mode for yt-dlp's curl-cffi transport. `auto` uses Chrome impersonation only for TikTok / Instagram / Threads; public YouTube stays on yt-dlp's default transport to avoid curl-cffi cleanup crashes on Python 3.14. Set `always` to force it or `never` to disable it everywhere. |

### Video shrink

| Env var | Default | Notes |
|---|---|---|
| `EXPORT_SHRINK_VIDEO_CODEC` | `x264` | Video shrink codec. Default `x264` (libx264 high@4.0 yuv420p faststart) plays everywhere — QuickTime, Preview, iMovie, iOS Safari, Discord, web `<video>`, every NLE. Set `av1` to use SVT-AV1 for ~20-30% smaller files at matched quality, but only when your playback chain supports it (recent Chrome/Firefox, VLC, mpv); AV1-in-MP4 breaks on macOS Preview seek, Discord embeds, old Safari, and many other consumer players. |

### Platform

| Env var | Default | Notes |
|---|---|---|
| `DYLD_FALLBACK_LIBRARY_PATH` | `/opt/homebrew/lib` (macOS) | WeasyPrint needs libgobject. Set automatically by `run.sh`. |
| `LD_LIBRARY_PATH` | (system) | Linux equivalent — usually unset is correct. |

### Ephemeral-storage modes

| Use case | Strategy | Storage profile |
|---|---|---|
| **Personal localhost** (default) | Local `/tmp/export`, 1-hour TTL | Effectively ephemeral; old jobs auto-pruned |
| **Small VPS, trusted friends** | tmpfs mount + shorter TTL + `EXPORT_DOWNLOAD_AUTODELETE=1` | Maximum ephemeral; bounded by RAM, not disk |
| **Public-facing service** | All of above + S3/R2 with lifecycle policy | True "unlimited"; bounded retention, unbounded throughput |

For the VPS path, mount `/tmp/export` as RAM-backed tmpfs:

```bash
sudo mkdir -p /tmp/export
echo 'tmpfs /tmp/export tmpfs size=4G,mode=1777 0 0' | sudo tee -a /etc/fstab
sudo mount /tmp/export
```

Combined with `EXPORT_DOWNLOAD_AUTODELETE=1`, files never touch persistent
storage. The "storage" constraint becomes peak concurrent jobs × average job
size, not total throughput.

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
- Multi-file (compress / merge / batch): **unlimited** by default (set `EXPORT_MAX_COMPRESS_MB=N` to cap)
- Job dirs auto-deleted after 6 hours (lifespan-managed cleanup loop)
- Single-process — concurrent jobs share CPU; not designed for multi-tenant load
- Whisper is single-instance — concurrent transcriptions queue serially
- LibreOffice cold-starts each office→PDF call (~3 s); could daemonize with
  `unoserver` later if it matters
- The `/api/thumbnail` endpoint reads up to 50 MB per file for preview rendering
  (HEIC/AVIF/TIFF/ICO that browsers can't decode natively)

## Compatibility matrix (input × output)

The full live matrix is at `GET /api/capabilities` (no auth required) and
the **show what's supported** link in the UI renders it inline. Snapshot:

### Single-file conversions

| Input → | image | pdf | docx | epub | html | md | txt | mp3 | mp4 | csv | xlsx | json | srt | vtt | ttf | woff2 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| image | ✓ | ✓ | OCR | — | OCR | OCR | OCR | — | — | — | — | — | — | — | — | — |
| video | still | STT | STT | STT | — | notes | STT | ✓ | ✓ | — | — | — | STT | STT | — | — |
| audio | — | STT | STT | STT | — | notes | STT | ✓ | AG | — | — | — | STT | STT | — | — |
| pdf | ✓ | OCR | ✓ | ✓ | ✓ | ✓ | ✓+OCR | TTS | AG | — | — | — | — | — | — | — |
| document | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | TTS | — | — | — | — | — | — | — | — |
| office | — | ✓ | — | — | — | — | — | TTS | AG | — | — | — | — | — | — | — |
| spreadsheet | — | ✓ | — | — | ✓ | ✓ | ✓ | — | — | ✓ | ✓ | ✓ | — | — | — | — |
| subtitle | — | ✓ | ✓ | ✓ | ✓ | ✓ | ✓ | TTS | AG | — | — | — | ✓ | ✓ | — | — |
| font | — | — | — | — | — | — | — | — | — | — | — | — | — | — | ✓ | ✓ |
| comic | — | ✓ | — | — | — | — | — | — | — | — | — | — | — | — | — | — |
| URL | — | STT | STT | STT | — | notes | STT | ✓ | ✓ | — | — | — | STT | STT | — | — |

(`✓` = direct conversion, `still` = one sampled PNG frame (composable to other
image formats), `OCR` = via Tesseract, `STT` = via Whisper, `TTS` =
via `say`/piper, `AG` = audiogram (mp4 with waveform + playhead), `—` = no
semantic path. The UI convert-to list is the reachable set (composed hops),
not only this snapshot. The `image` row also covers special outputs `nobg`
(transparent PNG), `svg` (vectorized) and `jxl`; the `audio`/`video` rows also
produce `denoise`, `stems`, `kit` (zip of txt+srt+vtt+notes), `captioned`,
`frames`, `subs`. Spreadsheet also cross-converts `yaml`/`toml`/`xml`.)

### Operations (not format conversions)

| Action | Input | Notes |
|---|---|---|
| `shrink` | image / video / pdf | Lossy size reduction with `low / medium / high` presets + advanced quality slider (1–100). For image/video the advanced panel also exposes **two mutually-exclusive resize knobs**: a **max longest side (px)** input (presets `source / 4K / 1080p / 720p / 480p`) and a **scale by (%)** input (presets `off / 75% / 50% / 25%`). When set, downscales preserving aspect ratio (Pillow `resize`/`thumbnail` for images, ffmpeg `-vf scale` for video, both rounded to even dimensions). |
| `compress` (archive) | any files | `zip` / `7z` / `tar.zst` / `tar.gz` / `tar.bz2` / `tar.xz` |
| `compress` (single-file) | one file | `gz` / `bz2` / `xz` / `zst` (raw stream, no tar) |
| `resize` | one image / video | Downscale by longest-side (`max_dim=N` px) OR `scale_percent=N`, **or** an exact `target_byte` cap with no max_dim/scale (same encode-under + pad as shrink-to-20-MB). Internally `/api/shrink?level=resize`. |
| `merge` | 2+ PDFs | Single combined PDF (pypdf) |
| `split` | one PDF | Each page as a separate PDF, zipped |
| `rotate` / `watermark` / `encrypt` / `decrypt` | one PDF | Per-PDF operations |
| Image batch (multi-shape) | 2+ images | `pdf` (multi-page) / animated `gif` / `mp4` slideshow |
| Generic batch convert | 2+ same-type files | Each file converted, then either zipped 1:1 or merged into one. **39 mergeable target formats** with format-aware mergers (PDF, text concat, JSON array, CSV/TSV header-dedupe, subtitle time-shift, audio/video ffmpeg concat, image tile grid, xlsx multi-sheet workbook). |
- Job dirs auto-deleted after 6 hours (tunable via `EXPORT_CLEANUP_AGE_SECONDS`)
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

## Running it

| Command | What it does |
|---|---|
| `npm run dev` / `npm start` / `npm run watch` | Wraps `bash scripts/run.sh`. Auto-creates `.env` with password `1234` on first run. No actual Node deps — pure muscle-memory shim. |
| `bash scripts/run.sh` | Same. Loads `.env`, sets `DYLD_FALLBACK_LIBRARY_PATH` on macOS, execs uvicorn on `HOST` (default `127.0.0.1`) / `PORT` (default `8500`). |
| `npm run install:macos` / `bash scripts/install-macos.sh` | One-shot macOS setup: brew installs (ffmpeg, pandoc, ghostscript, poppler, tesseract, pango, deno, libraw), LibreOffice cask, venv + `pip install -e ".[media]"`, curated tessdata packs. |
| `npm run install:linux` / `bash scripts/install-linux.sh` | One-shot Debian/Ubuntu setup: apt installs, deno via official installer, venv + pip, tessdata packs, piper voice. Prints tmpfs guidance for ephemeral storage. |
| `bash scripts/install-tessdata.sh` | Download Filipino/CJK/Arabic/… Tesseract packs into the system tessdata dir. Idempotent. |
| `bash scripts/prefetch.sh` | Cache Whisper + rembg models so `EXPORT_OFFLINE=1` jobs don't hit the network. |

The first job that needs a downloaded model (Whisper, rembg, demucs, Piper)
pauses ~30 s while it fetches and caches. Subsequent jobs reuse the cached
model.

## Security & deployment scope

`export` is built for **personal use** (localhost or your private network).
Phase 2 app hardening is in: download uses a short-lived `token` (not `?pw=`),
mutating POSTs are rate-limited, URL extract refuses SSRF, Pillow bombs are
capped at 64_000_000 pixels. HTTPS is a reverse-proxy job —
[scripts/nginx-export.conf](scripts/nginx-export.conf) and
[scripts/Caddyfile](scripts/Caddyfile). Certbot / live DNS / tmpfs mount are
deploy-time, not claimed here.

For a trusted-group host set `EXPORT_MAX_UPLOAD_MB=500`,
`EXPORT_MAX_COMPRESS_MB=2000`, `EXPORT_DOWNLOAD_AUTODELETE=1`,
`EXPORT_CLEANUP_AGE_SECONDS=900`, `EXPORT_CLEANUP_INTERVAL_SECONDS=60`, and a
32+ character `EXPORT_PASSWORD`. Localhost may keep unlimited caps and `1234`.
Read [SECURITY.md](SECURITY.md) before exposing anything.

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
