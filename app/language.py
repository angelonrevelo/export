"""Language detection + pack picking for OCR / STT / TTS.

Offline-first. Unicode script ranges plus langdetect (no network). Tesseract
language packs and macOS/piper voices are resolved from what is actually
installed on this machine.

ISO 639-1 (`en`, `tl`, `zh`) is the canonical code everywhere in the app.
Tesseract uses its own 3-letter / script codes (`eng`, `fil`, `chi_sim`);
those are mapped at the tess boundary only.
"""
from __future__ import annotations

import os
import shutil
import subprocess
import sys
from contextvars import ContextVar
from functools import lru_cache
from pathlib import Path

# Job-scoped override (set from the request's `language` field). EXPORT_LANGUAGE
# is the process-wide fallback. Empty / "auto" means detect.
_job_language_code: ContextVar[str | None] = ContextVar("job_language_code", default=None)
# Target language for the NLLB translate pass. Empty means leave text as-is.
_job_translate_to: ContextVar[str | None] = ContextVar("job_translate_to", default=None)

# ISO 639-1 (and a few 639-3 / BCP-47 we normalize) → tesseract tessdata name.
# Values that are tuples are tried in order until an installed pack is found.
TESS_CODE = {
    "en": "eng",
    "tl": "fil",
    "fil": "fil",
    "es": "spa",
    "fr": "fra",
    "de": "deu",
    "pt": "por",
    "it": "ita",
    "nl": "nld",
    "pl": "pol",
    "tr": "tur",
    "uk": "ukr",
    "cs": "ces",
    "ro": "ron",
    "hu": "hun",
    "sv": "swe",
    "fi": "fin",
    "no": "nor",
    "da": "dan",
    "el": "ell",
    "zh": ("chi_sim", "chi_tra"),
    "zh-cn": "chi_sim",
    "zh-tw": "chi_tra",
    "ja": "jpn",
    "ko": "kor",
    "ar": "ara",
    "ru": "rus",
    "hi": "hin",
    "th": "tha",
    "vi": "vie",
    "id": "ind",
    "ms": "msa",
    "bn": "ben",
    "ta": "tam",
    "te": "tel",
    "ml": "mal",
    "kn": "kan",
    "he": "heb",
    "fa": "fas",
    "ur": "urd",
    "ne": "nep",
    "si": "sin",
    "my": "mya",
    "km": "khm",
    "lo": "lao",
    "am": "amh",
    "sw": "swa",
    "af": "afr",
    "ca": "cat",
    "eu": "eus",
    "gl": "glg",
    "cy": "cym",
    "ga": "gle",
    "is": "isl",
    "lt": "lit",
    "lv": "lav",
    "et": "est",
    "sk": "slk",
    "hr": "hrv",
    "sr": "srp",
    "bg": "bul",
    "la": "lat",
}

# Reverse: tessdata name → ISO 639-1 (best-effort; chi_tra stays zh-tw).
ISO_FROM_TESS = {}
for _iso, _tess in TESS_CODE.items():
    if isinstance(_tess, tuple):
        for _t in _tess:
            ISO_FROM_TESS.setdefault(_t, _iso)
    else:
        ISO_FROM_TESS.setdefault(_tess, _iso)

# Unicode script → default ISO code when langdetect can't run (short / mixed text).
SCRIPT_LANGUAGE = {
    "latin": "en",
    "han": "zh",
    "hangul": "ko",
    "kana": "ja",
    "arabic": "ar",
    "cyrillic": "ru",
    "thai": "th",
    "devanagari": "hi",
    "hebrew": "he",
    "bengali": "bn",
    "tamil": "ta",
    "greek": "el",
    "other": "en",
}

# Tesseract OSD script names → our script keys.
OSD_SCRIPT = {
    "latin": "latin",
    "han": "han",
    "hangul": "hangul",
    "japanese": "kana",
    "katakana": "kana",
    "hiragana": "kana",
    "arabic": "arabic",
    "cyrillic": "cyrillic",
    "thai": "thai",
    "devanagari": "devanagari",
    "hebrew": "hebrew",
    "bengali": "bengali",
    "tamil": "tamil",
    "greek": "greek",
}

LANGUAGE_NAME = {
    "en": "English", "tl": "Filipino", "fil": "Filipino", "es": "Spanish",
    "fr": "French", "de": "German", "pt": "Portuguese", "it": "Italian",
    "nl": "Dutch", "pl": "Polish", "tr": "Turkish", "uk": "Ukrainian",
    "cs": "Czech", "ro": "Romanian", "hu": "Hungarian", "sv": "Swedish",
    "fi": "Finnish", "no": "Norwegian", "da": "Danish", "el": "Greek",
    "zh": "Chinese", "zh-cn": "Chinese (Simplified)", "zh-tw": "Chinese (Traditional)",
    "ja": "Japanese", "ko": "Korean", "ar": "Arabic", "ru": "Russian",
    "hi": "Hindi", "th": "Thai", "vi": "Vietnamese", "id": "Indonesian",
    "ms": "Malay", "bn": "Bengali", "ta": "Tamil", "te": "Telugu",
    "ml": "Malayalam", "kn": "Kannada", "he": "Hebrew", "fa": "Persian",
    "ur": "Urdu", "ne": "Nepali", "si": "Sinhala", "my": "Burmese",
    "km": "Khmer", "lo": "Lao", "am": "Amharic", "sw": "Swahili",
    "af": "Afrikaans", "ca": "Catalan", "eu": "Basque", "gl": "Galician",
    "cy": "Welsh", "ga": "Irish", "is": "Icelandic", "lt": "Lithuanian",
    "lv": "Latvian", "et": "Estonian", "sk": "Slovak", "hr": "Croatian",
    "sr": "Serbian", "bg": "Bulgarian", "la": "Latin",
}

# UI / capabilities shortlist — globally high-use languages, Filipino first after English
# because this tool is used in the PH. The detector still handles anything in TESS_CODE.
UI_LANGUAGE = [
    "en", "tl", "es", "zh", "hi", "ar", "fr", "pt", "ru", "ja",
    "de", "ko", "it", "tr", "vi", "id", "th", "pl", "nl", "uk",
]

# Latin-script packs we combine for a one-pass OCR when OSD says Latin and the
# user didn't pin a language. Capped so tesseract doesn't get a 40-lang string.
LATIN_OCR_PRIORITY = ["eng", "fil", "spa", "fra", "deu", "por", "ita", "nld", "ind", "vie", "pol", "tur"]

WHISPER_ENGLISH_ONLY = ("distil-large-v3", "distil-medium.en", "distil-small.en")


def is_offline() -> bool:
    return os.environ.get("EXPORT_OFFLINE", "").strip().lower() in {
        "1", "true", "yes", "on",
    }


def is_whisper_english_only(model_name: str) -> bool:
    name = (model_name or "").lower()
    if name.endswith(".en"):
        return True
    return any(name == m or name.endswith("/" + m) for m in WHISPER_ENGLISH_ONLY)


def normalize_language_code(raw: str | None) -> str | None:
    if not raw:
        return None
    code = raw.strip().lower().replace("_", "-")
    if code in {"", "auto", "detect", "und", "unknown"}:
        return None
    if code == "fil":
        return "tl"
    if code == "zh-cn":
        return "zh"
    if code in TESS_CODE or code in LANGUAGE_NAME:
        return code
    # tesseract tessdata name passed in directly (chi_sim, fil, …)
    tess_name = code.replace("-", "_")
    if tess_name in ISO_FROM_TESS:
        iso = ISO_FROM_TESS[tess_name]
        return "tl" if iso == "fil" else iso
    # BCP-47 like pt-BR / zh-Hans
    if "-" in code:
        prefix, suffix = code.split("-", 1)
        if code in TESS_CODE:
            return code
        if prefix == "zh":
            if suffix in {"tw", "hk", "hant", "tra"}:
                return "zh-tw"
            return "zh"
        if prefix in TESS_CODE:
            return prefix
    return code if len(code) in (2, 3) else None


def language_name(code: str | None) -> str:
    if not code:
        return "unknown"
    return LANGUAGE_NAME.get(code, code)


def requested_language_code() -> str | None:
    """Job override, then EXPORT_LANGUAGE, else None (auto-detect)."""
    job = _job_language_code.get()
    if job:
        return job
    return normalize_language_code(os.environ.get("EXPORT_LANGUAGE"))


def requested_translate_to() -> str | None:
    """Job override, then EXPORT_TRANSLATE_TO, else None (do not translate)."""
    job = _job_translate_to.get()
    if job:
        return job
    return normalize_language_code(os.environ.get("EXPORT_TRANSLATE_TO"))


class use_language:
    """Context manager that pins a language for the current convert job."""

    def __init__(self, code: str | None):
        self.code = normalize_language_code(code)
        self.token = None

    def __enter__(self):
        self.token = _job_language_code.set(self.code)
        return self.code

    def __exit__(self, *exc):
        if self.token is not None:
            _job_language_code.reset(self.token)


class use_translate_to:
    """Context manager that pins the NLLB target language for this job."""

    def __init__(self, code: str | None):
        self.code = normalize_language_code(code)
        self.token = None

    def __enter__(self):
        self.token = _job_translate_to.set(self.code)
        return self.code

    def __exit__(self, *exc):
        if self.token is not None:
            _job_translate_to.reset(self.token)


def detect_script(text: str) -> str:
    """Majority Unicode script of `text`. Cheap, no model, works offline."""
    count = {
        "latin": 0, "han": 0, "hangul": 0, "kana": 0, "arabic": 0,
        "cyrillic": 0, "thai": 0, "devanagari": 0, "hebrew": 0,
        "bengali": 0, "tamil": 0, "greek": 0,
    }
    for ch in text:
        o = ord(ch)
        if (0x41 <= o <= 0x5A) or (0x61 <= o <= 0x7A) or (0xC0 <= o <= 0x24F) or (0x1E00 <= o <= 0x1EFF):
            count["latin"] += 1
        elif 0x4E00 <= o <= 0x9FFF or 0x3400 <= o <= 0x4DBF:
            count["han"] += 1
        elif 0xAC00 <= o <= 0xD7AF:
            count["hangul"] += 1
        elif 0x3040 <= o <= 0x30FF:
            count["kana"] += 1
        elif 0x0600 <= o <= 0x06FF or 0x0750 <= o <= 0x077F or 0x08A0 <= o <= 0x08FF:
            count["arabic"] += 1
        elif 0x0400 <= o <= 0x04FF:
            count["cyrillic"] += 1
        elif 0x0E00 <= o <= 0x0E7F:
            count["thai"] += 1
        elif 0x0900 <= o <= 0x097F:
            count["devanagari"] += 1
        elif 0x0590 <= o <= 0x05FF:
            count["hebrew"] += 1
        elif 0x0980 <= o <= 0x09FF:
            count["bengali"] += 1
        elif 0x0B80 <= o <= 0x0BFF:
            count["tamil"] += 1
        elif 0x0370 <= o <= 0x03FF:
            count["greek"] += 1
    winner = max(count, key=count.get)
    return winner if count[winner] else "other"


def detect_language_code(text: str | None) -> str | None:
    """ISO 639-1 (or zh-tw) for `text`. Script fallback when langdetect is unsure."""
    sample = (text or "").strip()
    if not sample:
        return requested_language_code()
    forced = requested_language_code()
    if forced:
        return forced
    script = detect_script(sample)
    if script != "latin" and script != "other":
        # Non-Latin: script is a stronger signal than langdetect on short OCR.
        return SCRIPT_LANGUAGE[script]
    if len(sample) < 24:
        return SCRIPT_LANGUAGE.get(script, "en")
    try:
        from langdetect import detect as _detect
        guessed = normalize_language_code(_detect(sample))
        if guessed:
            return guessed
    except Exception:
        pass
    return SCRIPT_LANGUAGE.get(script, "en")


@lru_cache(maxsize=1)
def installed_tess_lang() -> tuple[str, ...]:
    """Tesseract tessdata names actually present (`eng`, `fil`, `osd`, …)."""
    tesseract = shutil.which("tesseract")
    if not tesseract:
        return ()
    try:
        r = subprocess.run(
            [tesseract, "--list-langs"],
            capture_output=True, text=True, timeout=10,
        )
    except (OSError, subprocess.TimeoutExpired):
        return ()
    out = (r.stdout or "") + "\n" + (r.stderr or "")
    pack = []
    for line in out.splitlines():
        name = line.strip()
        if not name or " " in name or name.lower().startswith("list"):
            continue
        pack.append(name)
    return tuple(pack)


def tess_code_for(language_code: str | None) -> str | None:
    """Best installed tesseract pack for an ISO code, or None."""
    if not language_code:
        return None
    mapped = TESS_CODE.get(language_code)
    installed = set(installed_tess_lang())
    if mapped is None:
        # already a tess name?
        return language_code if language_code in installed else None
    if isinstance(mapped, tuple):
        for name in mapped:
            if name in installed:
                return name
        return mapped[0]
    return mapped


def pick_tess_lang(text: str | None = None, osd_script: str | None = None) -> str:
    """Tesseract `-l` string. Honors EXPORT_OCR_LANG, then job language, then detect.

    Latin auto-detect combines the installed high-use Latin packs (eng+fil+spa…)
    so a mixed-language page still OCRs in one pass.
    """
    override = os.environ.get("EXPORT_OCR_LANG", "").strip()
    if override:
        return override
    installed = list(installed_tess_lang())
    installed_set = set(installed)
    forced = requested_language_code()
    if forced:
        tess = tess_code_for(forced)
        if tess:
            # Always include eng as a fallback for mixed pages (menus, UI chrome).
            if tess != "eng" and "eng" in installed_set:
                return f"{tess}+eng"
            return tess

    script = osd_script
    if not script and text:
        script = detect_script(text)

    if script and script != "latin" and script != "other":
        iso = SCRIPT_LANGUAGE.get(script, "en")
        tess = tess_code_for(iso)
        if tess and tess in installed_set:
            if tess != "eng" and "eng" in installed_set:
                return f"{tess}+eng"
            return tess
        if script == "han":
            han = [n for n in ("chi_sim", "chi_tra", "jpn") if n in installed_set]
            if han:
                return "+".join(han + (["eng"] if "eng" in installed_set else []))

    # Latin / unknown: combine the highest-priority installed Latin packs.
    combined = [n for n in LATIN_OCR_PRIORITY if n in installed_set][:6]
    if combined:
        return "+".join(combined)
    if "eng" in installed_set:
        return "eng"
    return installed[0] if installed else "eng"


def osd_script_from_image(image) -> str | None:
    """Tesseract OSD script name mapped to our script key. None on failure."""
    try:
        import pytesseract
        osd = pytesseract.image_to_osd(image) or ""
    except Exception:
        return None
    script_raw = ""
    for line in osd.splitlines():
        if line.lower().startswith("script:"):
            script_raw = line.split(":", 1)[1].strip().lower()
            break
    return OSD_SCRIPT.get(script_raw)


def whisper_language_arg(code: str | None) -> str | None:
    """faster-whisper / Groq `language=` value. None means auto-detect."""
    code = normalize_language_code(code) or requested_language_code()
    if not code:
        return None
    if code in {"zh-cn", "zh-tw", "zh"}:
        return "zh"
    if code == "fil":
        return "tl"
    return code


def default_whisper_model() -> str:
    return os.environ.get("EXPORT_WHISPER_MODEL", "large-v3-turbo")


@lru_cache(maxsize=1)
def _say_voice_table() -> tuple[tuple[str, str], ...]:
    """(voice_name, bcp47) from `say -v '?'`. Empty on non-macOS / missing say."""
    if sys.platform != "darwin" or not shutil.which("say"):
        return ()
    try:
        r = subprocess.run(["say", "-v", "?"], capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return ()
    row = []
    for line in (r.stdout or "").splitlines():
        # "Samantha            en_US    # comment"
        if "#" in line:
            left, _ = line.split("#", 1)
        else:
            left = line
        parts = left.split()
        if len(parts) < 2:
            continue
        lang_tag = parts[-1].replace("_", "-")
        voice = " ".join(parts[:-1]).strip()
        if voice and lang_tag:
            row.append((voice, lang_tag.lower()))
    return tuple(row)


def pick_say_voice(language_code: str | None) -> str | None:
    """macOS `say -v` voice for an ISO code. None → let `say` use the system default."""
    code = normalize_language_code(language_code) or requested_language_code()
    if not code:
        return None
    table = _say_voice_table()
    if not table:
        return None
    want = {
        "en": ("en-us", "en-gb", "en-au", "en"),
        "tl": ("fil", "tl"),
        "zh": ("zh-cn", "zh-hans", "zh"),
        "zh-cn": ("zh-cn", "zh-hans"),
        "zh-tw": ("zh-tw", "zh-hant", "zh-hk"),
        "pt": ("pt-br", "pt-pt", "pt"),
        "es": ("es-mx", "es-es", "es"),
    }.get(code, (code,))
    # Prefer an exact prefix match (en-US for en).
    for voice, tag in table:
        tag_l = tag.lower()
        for w in want:
            if tag_l == w or tag_l.startswith(w + "-"):
                return voice
    prefix = code.split("-")[0]
    for voice, tag in table:
        if tag.lower().startswith(prefix):
            return voice
    return None


def pick_piper_voice(language_code: str | None) -> Path | None:
    """Path to a piper .onnx matching the language, else EXPORT_PIPER_VOICE."""
    default = os.environ.get("EXPORT_PIPER_VOICE", "")
    default_path = Path(default) if default else None
    code = normalize_language_code(language_code) or requested_language_code()
    search_dir: list[Path] = []
    if default_path:
        search_dir.append(default_path.parent)
    repo_voices = Path(__file__).resolve().parent.parent / "voices"
    if repo_voices.is_dir():
        search_dir.append(repo_voices)
    env_dir = os.environ.get("EXPORT_PIPER_VOICE_DIR", "")
    if env_dir:
        search_dir.append(Path(env_dir))

    prefixes = []
    if code:
        prefixes.append(code.replace("-", "_"))
        prefixes.append(code.split("-")[0])
        if code == "tl":
            prefixes.extend(["fil", "tl_PH", "fil_PH"])
        if code in {"zh", "zh-cn"}:
            prefixes.extend(["zh_CN", "zh"])
        if code == "zh-tw":
            prefixes.extend(["zh_TW"])
        if code == "en":
            prefixes.extend(["en_US", "en_GB"])

    seen: set[Path] = set()
    for folder in search_dir:
        try:
            files = sorted(folder.glob("*.onnx"))
        except OSError:
            continue
        for f in files:
            if f in seen:
                continue
            seen.add(f)
            name = f.name.lower()
            for pfx in prefixes:
                if name.startswith(pfx.lower() + "_") or name.startswith(pfx.lower() + "-"):
                    return f
    if default_path and default_path.exists():
        return default_path
    return None


def tts_backend() -> str:
    if sys.platform == "darwin" and shutil.which("say"):
        return "say"
    if shutil.which("piper") and os.environ.get("EXPORT_PIPER_VOICE"):
        return "piper"
    if shutil.which("piper"):
        return "piper"
    return "none"


def record_on_job(job: dict | None, language_code: str | None,
                  translate_to: str | None = None) -> None:
    if job is None:
        return
    if language_code:
        job["language"] = language_code
        job["language_name"] = language_name(language_code)
    target = translate_to or requested_translate_to()
    if target:
        job["translate_to"] = target
        src = language_name(language_code) if language_code else "auto"
        job["language_name"] = f"{src} → {language_name(target)}"
