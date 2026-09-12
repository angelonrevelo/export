"""Reachable convert formats by composing existing one-hop bridges.

Query (BFS) is pure — no ffmpeg. Execution walks the path by calling
`_convert_one` in jobs.py. Direct one-hop always wins over a longer
path, so video→pdf stays the transcript edge (not frames→images→pdf).

Equal-length ties prefer raster intermediates (`png` before `pdf`) so
video→jpeg is a still frame, not a raster of the transcript PDF.
"""
from __future__ import annotations

from collections import deque

# Offered as convert-to targets but not walked as graph nodes. Animated /
# bundle / effect outputs are dead-ends; the still-frame bridge is png.
TERMINAL_FORMAT = frozenset({
    "kit", "captioned", "frames", "subs", "notes", "translate",
    "nobg", "stems", "denoise", "audiogram", "pdfa",
    "gif", "ico", "icns",
})

_KIND_RANK = {
    "image": 0,
    "video": 1,
    "audio": 2,
    "subtitle": 3,
    "spreadsheet": 4,
    "font": 5,
    "comic": 6,
    "pdf": 7,
    "document": 8,
    "office": 9,
}


def _sets():
    from . import jobs as J
    return J


def kind_of_suffix(suffix: str) -> str | None:
    """Map `.mp4` / `mp4` to a kind used as a graph node."""
    J = _sets()
    s = suffix if suffix.startswith(".") else f".{suffix}"
    s = s.lower()
    if s == ".jpg":
        s = ".jpeg"
    # gif is in both image and video suffix sets; match detectType (image first).
    if s in J.IMAGE_SUFFIXES:
        return "image"
    if s in J.VIDEO_SUFFIXES:
        return "video"
    if s in J.AUDIO_SUFFIXES:
        return "audio"
    if s in J.PDF_SUFFIXES:
        return "pdf"
    if s in J.COMIC_SUFFIXES:
        return "comic"
    if s in J.FONT_SUFFIXES:
        return "font"
    if s in J.SUBTITLE_SUFFIXES:
        return "subtitle"
    if s in J.SPREADSHEET_SUFFIXES:
        return "spreadsheet"
    if s in J.OFFICE_SUFFIXES:
        return "office"
    if s in J.DOC_INPUT_SUFFIXES or s in J.TTS_INPUT_SUFFIXES:
        return "document"
    return None


def kind_of_format(fmt: str) -> str | None:
    """If this output format can be fed back in as an input, its kind."""
    if fmt in {"jpg", "jpeg", "png", "webp", "avif", "tiff", "bmp", "gif",
               "ico", "icns", "jp2", "jxl", "svg", "heic", "heif", "nobg"}:
        return "image"
    if fmt in {"mp4", "mov", "webm", "mkv", "audiogram", "captioned"}:
        return "video"
    if fmt in {"mp3", "m4a", "m4b", "wav", "flac", "ogg", "opus", "aac", "denoise"}:
        return "audio"
    if fmt in {"pdf", "pdfa"}:
        return "pdf"
    if fmt in {"txt", "md", "html", "htm", "rtf", "epub", "odt", "tex", "org",
               "rst", "pptx", "notes", "translate"}:
        return "document"
    if fmt in {"docx", "doc"}:
        return "office"
    if fmt in {"csv", "tsv", "xlsx", "json", "yaml", "yml", "toml"}:
        return "spreadsheet"
    if fmt in {"srt", "vtt", "ass", "ssa", "sub", "sbv"}:
        return "subtitle"
    if fmt in {"ttf", "otf", "woff", "woff2"}:
        return "font"
    return None


def direct_target(kind: str) -> frozenset[str]:
    """One-hop outputs from an input kind. Includes video→png (still frame)."""
    J = _sets()
    if kind == "image":
        return frozenset(
            J.IMAGE_FORMATS | J.IMAGE_TO_PDF | J.IMAGE_TO_TEXT | J.IMAGE_TO_NOBG
            | J.IMAGE_TO_SVG | J.IMAGE_TO_DOC
        )
    if kind == "video":
        return frozenset(
            J.VIDEO_TO_GIF | J.VIDEO_TO_VIDEO | J.WHISPER_OUTPUTS | {"txt"}
            | J.TRANSCRIPT_KIT | J.VIDEO_CAPTIONED | J.VIDEO_FRAMES | J.VIDEO_SUBS
            | {"pdf", "docx", "epub"} | J.AUDIO_TO_VIDEO | J.AUDIO_FORMATS
            | J.TRANSLATE_OUTPUT
            | {"png"}  # still frame — not a transcript, not a zip of frames
        )
    if kind == "audio":
        return frozenset(
            J.AUDIO_FORMATS | J.WHISPER_OUTPUTS | {"txt"} | J.TRANSCRIPT_KIT
            | J.AUDIO_DENOISE | J.AUDIO_STEMS | J.AUDIO_TO_VIDEO
            | {"pdf", "docx", "epub"} | J.TRANSLATE_OUTPUT
        )
    if kind == "pdf":
        return frozenset(
            J.PDF_TO_IMAGE | J.PDF_TO_TEXT | J.PDF_TO_DOCX | J.PDF_OCR_OUTPUTS
            | J.PDF_TO_MD | J.PDF_TO_NOTES | J.PDF_TO_DOC | J.TTS_AUDIO_FORMATS
            | J.AUDIO_TO_VIDEO | J.TRANSLATE_OUTPUT
        )
    if kind == "document":
        return frozenset(J.DOC_FORMATS | J.DOC_TO_PDF | J.TTS_AUDIO_FORMATS | J.TRANSLATE_OUTPUT)
    if kind == "office":
        return frozenset(
            J.OFFICE_TO_PDF | J.OFFICE_TO_IMAGE | J.OFFICE_TO_TEXT | J.DOC_FORMATS
            | J.TTS_AUDIO_FORMATS | J.AUDIO_TO_VIDEO | J.TRANSLATE_OUTPUT
        )
    if kind == "spreadsheet":
        return frozenset(
            J.SPREADSHEET_FORMATS | J.SPREADSHEET_TO_TEXT | J.SPREADSHEET_TO_PDF
            | J.TRANSLATE_OUTPUT
        )
    if kind == "subtitle":
        return frozenset(
            J.SUBTITLE_FORMATS | J.SUBTITLE_TO_TEXT | J.SUBTITLE_TO_DOC
            | J.TTS_AUDIO_FORMATS | J.AUDIO_TO_VIDEO | J.TRANSLATE_OUTPUT
        )
    if kind == "font":
        return frozenset(J.FONT_FORMATS | J.FONT_TO_IMAGE)
    if kind == "comic":
        return frozenset(J.COMIC_TO_PDF)
    return frozenset()


def _visit_order(format_set: frozenset[str] | set[str]) -> list[str]:
    """Stable BFS neighbor order. Raster hops win equal-length ties."""
    return sorted(
        format_set,
        key=lambda f: (_KIND_RANK.get(kind_of_format(f) or "", 10), f),
    )


def shortest_path(suffix: str, fmt: str) -> list[str] | None:
    """Formats to convert TO, in order. Empty list means already that format.
    None means no semantic path (not a binary waterfall)."""
    kind = kind_of_suffix(suffix)
    if kind is None:
        return None
    if fmt in direct_target(kind):
        return [fmt]
    parent: dict[str, str | None] = {}
    q: deque[str] = deque()
    for f in _visit_order(direct_target(kind)):
        parent[f] = None
        q.append(f)
    seen = set(parent)
    while q:
        cur = q.popleft()
        if cur in TERMINAL_FORMAT:
            continue
        nxt_kind = kind_of_format(cur)
        if nxt_kind is None:
            continue
        for g in _visit_order(direct_target(nxt_kind)):
            if g in seen:
                continue
            seen.add(g)
            parent[g] = cur
            if g == fmt:
                hop = [fmt]
                p = cur
                while p is not None:
                    hop.append(p)
                    p = parent[p]
                hop.reverse()
                return hop
            q.append(g)
    return None


def reachable(suffix: str) -> list[str]:
    kind = kind_of_suffix(suffix)
    if kind is None:
        return []
    seen: set[str] = set(direct_target(kind))
    q = deque(_visit_order(seen))
    while q:
        cur = q.popleft()
        if cur in TERMINAL_FORMAT:
            continue
        nxt_kind = kind_of_format(cur)
        if nxt_kind is None:
            continue
        for g in _visit_order(direct_target(nxt_kind)):
            if g not in seen:
                seen.add(g)
                q.append(g)
    return sorted(seen)
