# Changelog

## [Unreleased]

### Added
- Rust workspace in `Cargo.toml`, one member per crate: `export-aac`, `export-audio`, `export-avcbox`, `export-deflate`, `export-docs`, `export-fetch`, `export-fmp4`, `export-gif`, `export-h264`, `export-hls`, `export-image`, `export-kernel`, `export-media`, `export-meta`, `export-mp3`, `export-opus`, `export-pack`, `export-pdf`, `export-sheet`, `export-speech`, `export-subtitle`, `export-translate`, `export-vorbis`, `export-vp9`, `export-web`, `export-webm`. These are libraries. `scripts/run.sh` still serves the Python app in `app/`. The page does not call them.
- `export-kernel` keeps its checked raw DEFLATE inflater. `export-deflate` is a separate raw compressor and inflater. Pack, image, docs, and sheet keep their own inflate code.
- `scripts/outside_score.py` scores uploads already in the repo against the Python app. `EXPORT_RUST_ORACLE` may name a separate program. The script is not the server.
- `AGENTS.md`: standing orders for coding agents — N of M on real files is the target, the `EXPORT_RUST_ORACLE` CLI contract, the gate, and what not to do.
- `scripts/outside_score.py` also reads a local real-file corpus from disk — `corpus/` (gitignored), `--corpus DIR`, or `EXPORT_CORPUS` — so private uploads count toward N/M without being committed.
- `EXPORT_CORS_ORIGIN` CORS allow-list (comma-separated exact origins; unset = no CORS, as before). Allows `X-Password` + `Content-Type`, answers private-network preflights with `Access-Control-Allow-Private-Network: true`, and 401/429 responses carry the CORS headers. For cuttingboard's Companion.
- `words` transcript format → `{stem}_words.json` = `{ language, word: [{ text, start, end, confidence }] }` from faster-whisper `word_timestamps=True` (turbo and the pinned PH dialect model) or Groq word granularity (`confidence` null). Upload, URL, capabilities and the reachable set list it. Not translated.
- PH dialect STT: UI pins Cebuano / Ilocano / Hiligaynon / Waray / Bikol (`ceb` `ilo` `hil` `war` `bcl`) to `rbcurzon/whisper-medium-ph`. Tagalog/auto stay on `large-v3-turbo`. Missing dialect weights error; they do not transcribe as `tl`.
- File meta: **scrub** (GPS/serial EXIF + Where From + xattrs), Finder **comment** / Where From, **hidden** / clear custom icon, bulk date/tag on N copies, decrypt **7z** and encrypted **docx/xlsx**. Dates, tags, unlock, pdf/zip decrypt stay.
- File meta ops: set modified/created date (EXIF or a datetime), Finder tags on Mac, unlock (quarantine + lock), zip decrypt. **Show in Finder** so tags/dates survive (browser download strips xattrs).
- Phase 2 hardening: per-job download `token` (not `?pw=`), SSRF refuse on URL extract, 30/min mutating rate limit, Pillow `MAX_IMAGE_PIXELS=64000000`. Convert-to picker grouped by kind (`<optgroup>`).
- Multilingual OCR/STT/TTS with auto-detect, Tesseract pack picking, and a UI language dropdown.
- Offline mode (`EXPORT_OFFLINE`) plus `scripts/prefetch.sh` / `scripts/install-tessdata.sh`.
- NLLB-200 translation (`translate_to` / format `translate`), including Filipino.
- Extra conversion bridges: SVG input, JPEG XL / camera RAW (optional `.[media]`), YAML/TOML/XML, CBZ→PDF, video `kit` / `captioned` / `frames` / `subs`.
- Exact byte-target shrink (`target_byte`; 20 MB = 20_000_000). Encode under the cap then pad. Under-cap sources are not grown.
- Convert **reachable set**: after a file is chosen, convert-to lists every format reachable by composing existing bridges (multi-hop). `GET /api/reachable?ext=mp4` returns `{ "format": [...] }`. Video→`png` is a still frame (not a transcript, not raw bytes in a PNG). Video→JPEG hops through that still. Video→PDF stays the existing transcript PDF.

### Changed
- `scripts/outside_score.py` scores shrinks at a 1 MB photo / 5 MB video cap instead of the 20 MB maximum (under 20 MB the app copies, so a copy-only converter scored), and a shrink-cap match must decode to the same pixel size or duration (padding behind a magic no longer passes). Fixed: the Python reference now runs when the script is invoked directly (`app` was not importable), and a locked scratch file on Windows no longer aborts the run.
- Docs name the Rust crates on `main` and the Python paths that still serve. The runnable app is a local `scripts/run.sh`.
- Default local Whisper model is `large-v3-turbo` (multilingual). `distil-large-v3` is English-only.
- Shrink prediction with a byte cap is a single exact size, not a lo–hi range.
- Default video shrink profile remains playback-safe h.264 / yuv420p / faststart.

### Fixed
- Resize-to-cap (`level=resize` + `target_byte`) no longer requires `max_dim` or `scale_percent`.
- Captioned burn-in falls back to a PIL overlay when ffmpeg has no `subtitles` filter.
- Reachable-set unions every kind a suffix belongs to, so gif→`mp4` (and mov/webm/mkv) stay one-hop convert-to instead of disappearing behind the image-only family.
- Rate limit keys on the TCP peer, not the leftmost `X-Forwarded-For` hop (clients could rotate that header and skip the cap).

### Not done
- `git push` / origin.
- Live certbot / DNS / tmpfs mount (documented, not provisioned here).
- SECURITY.md Phase 3 (accounts, Redis, sandbox converters).
- Bilingual srt, HTML-safe translate, Homebrew libass burn-in.
- New format families (aseprite, N64, Minecraft, WASM). Video→PDF is still STT, not frames-as-PDF.
- No in-repo tests for `/api/compress`, `/api/pdf/op`, `/api/pdf/merge`, `/api/images/batch`, `/api/thumbnail`.
