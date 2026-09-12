# Changelog

## [Unreleased]

### Added
- Multilingual OCR/STT/TTS with auto-detect, Tesseract pack picking, and a UI language dropdown.
- Offline mode (`EXPORT_OFFLINE`) plus `scripts/prefetch.sh` / `scripts/install-tessdata.sh`.
- NLLB-200 translation (`translate_to` / format `translate`), including Filipino.
- Extra conversion bridges: SVG input, JPEG XL / camera RAW (optional `.[media]`), YAML/TOML/XML, CBZ→PDF, video `kit` / `captioned` / `frames` / `subs`.
- Exact byte-target shrink (`target_byte`; 20 MB = 20_000_000). Encode under the cap then pad. Under-cap sources are not grown.
- Convert **reachable set**: after a file is chosen, convert-to lists every format reachable by composing existing bridges (multi-hop). `GET /api/reachable?ext=mp4` returns `{ "format": [...] }`. Video→`png` is a still frame (not a transcript, not raw bytes in a PNG). Video→JPEG hops through that still. Video→PDF stays the existing transcript PDF.

### Changed
- Default local Whisper model is `large-v3-turbo` (multilingual). `distil-large-v3` is English-only.
- Shrink prediction with a byte cap is a single exact size, not a lo–hi range.
- Default video shrink profile remains playback-safe h.264 / yuv420p / faststart.

### Fixed
- Resize-to-cap (`level=resize` + `target_byte`) no longer requires `max_dim` or `scale_percent`.
- Captioned burn-in falls back to a PIL overlay when ffmpeg has no `subtitles` filter.

### Not done
- `git push` / origin.
- SECURITY.md Phase 2 (public `?pw=` / rate limit / upload caps).
- Bilingual srt, HTML-safe translate, Homebrew libass burn-in.
- New format families (aseprite, N64, Minecraft, WASM). Video→PDF is still STT, not frames-as-PDF.
- No in-repo tests for `/api/compress`, `/api/pdf/op`, `/api/pdf/merge`, `/api/images/batch`, `/api/thumbnail`.
