"""Offline NLLB-200 translation (CTranslate2 int8).

200-language coverage including Filipino (`tgl_Latn`). The tokenizer comes
from `facebook/nllb-200-distilled-600M`; the decoder is a CTranslate2
conversion of the same checkpoint. First use downloads ~1.2 GB; after that
it runs fully offline.

Tests swap `_translate_batch_impl` rather than loading the model.
"""
from __future__ import annotations

import os
import re
from functools import lru_cache
from pathlib import Path

from .language import (
    detect_language_code,
    is_offline,
    normalize_language_code,
    record_on_job,
    requested_translate_to,
)

# ISO 639-1 (plus zh-tw) → NLLB FLORES-200 code.
FLORES_CODE = {
    "en": "eng_Latn",
    "tl": "tgl_Latn",
    "es": "spa_Latn",
    "zh": "zho_Hans",
    "zh-tw": "zho_Hant",
    "hi": "hin_Deva",
    "ar": "arb_Arab",
    "fr": "fra_Latn",
    "pt": "por_Latn",
    "ru": "rus_Cyrl",
    "ja": "jpn_Jpan",
    "de": "deu_Latn",
    "ko": "kor_Hang",
    "it": "ita_Latn",
    "tr": "tur_Latn",
    "vi": "vie_Latn",
    "id": "ind_Latn",
    "th": "tha_Thai",
    "pl": "pol_Latn",
    "nl": "nld_Latn",
    "uk": "ukr_Cyrl",
    "cs": "ces_Latn",
    "ro": "ron_Latn",
    "hu": "hun_Latn",
    "sv": "swe_Latn",
    "fi": "fin_Latn",
    "no": "nob_Latn",
    "da": "dan_Latn",
    "el": "ell_Grek",
    "he": "heb_Hebr",
    "fa": "pes_Arab",
    "ur": "urd_Arab",
    "bn": "ben_Beng",
    "ta": "tam_Taml",
    "te": "tel_Telu",
    "ml": "mal_Mlym",
    "ms": "zsm_Latn",
    "af": "afr_Latn",
    "ca": "cat_Latn",
    "sw": "swh_Latn",
}

DEFAULT_CT2_REPO = "entai2965/nllb-200-distilled-600M-ctranslate2"
DEFAULT_TOKENIZER = "facebook/nllb-200-distilled-600M"
# Stay under NLLB's 512-token window. Characters are a cheap proxy.
CHUNK_CHAR = 720
BATCH_SIZE = 16

_translator = None
_tokenizer = None
_model_dir: Path | None = None

# Tests replace this. Signature: (list[str], src_flores, tgt_flores) -> list[str]
_translate_batch_impl = None


def flores_code(language_code: str | None) -> str | None:
    code = normalize_language_code(language_code)
    if not code:
        return None
    return FLORES_CODE.get(code)


def _split_chunk(text: str) -> list[str]:
    """Split `text` into <= CHUNK_CHAR pieces on paragraph / sentence boundaries."""
    text = text.strip()
    if not text:
        return []
    if len(text) <= CHUNK_CHAR:
        return [text]
    paragraph = re.split(r"(\n\s*\n+)", text)
    piece: list[str] = []
    buf = ""
    for part in paragraph:
        if len(buf) + len(part) <= CHUNK_CHAR:
            buf += part
            continue
        if buf.strip():
            piece.extend(_split_sentence(buf) if len(buf) > CHUNK_CHAR else [buf])
        buf = part
    if buf.strip():
        piece.extend(_split_sentence(buf) if len(buf) > CHUNK_CHAR else [buf])
    return [p for p in piece if p.strip()]


def _split_sentence(text: str) -> list[str]:
    if len(text) <= CHUNK_CHAR:
        return [text]
    part = re.split(r"(?<=[.!?。！？])\s+", text)
    piece: list[str] = []
    buf = ""
    for s in part:
        if buf and len(buf) + 1 + len(s) > CHUNK_CHAR:
            piece.append(buf)
            buf = s
        else:
            buf = f"{buf} {s}".strip() if buf else s
    if buf:
        piece.append(buf)
    # Hard wrap anything still oversized.
    out: list[str] = []
    for p in piece:
        if len(p) <= CHUNK_CHAR:
            out.append(p)
        else:
            for i in range(0, len(p), CHUNK_CHAR):
                out.append(p[i:i + CHUNK_CHAR])
    return out


def _ensure_model() -> tuple[object, object]:
    """Load CTranslate2 translator + NLLB tokenizer. Cached process-wide."""
    global _translator, _tokenizer, _model_dir
    if _translator is not None and _tokenizer is not None:
        return _translator, _tokenizer

    import ctranslate2
    from huggingface_hub import snapshot_download
    from transformers import AutoTokenizer

    tok_id = os.environ.get("EXPORT_NLLB_TOKENIZER", DEFAULT_TOKENIZER)
    ct2_id = os.environ.get("EXPORT_NLLB_MODEL", DEFAULT_CT2_REPO)
    offline = is_offline()
    _tokenizer = AutoTokenizer.from_pretrained(tok_id, local_files_only=offline)

    cache = os.environ.get("EXPORT_NLLB_MODEL_DIR")
    try:
        model_dir = snapshot_download(ct2_id, local_files_only=offline)
    except Exception:
        if offline:
            raise RuntimeError(
                "Offline mode — NLLB model is not cached. "
                "Run `bash scripts/prefetch.sh` once online."
            )
        # Convert the original Facebook checkpoint into CTranslate2 int8.
        dest = Path(cache) if cache else Path.home() / ".cache" / "export" / "nllb-ct2"
        dest.mkdir(parents=True, exist_ok=True)
        if not any(dest.glob("model*.bin")):
            from ctranslate2.converters import TransformersConverter
            TransformersConverter(tok_id).convert(
                str(dest), quantization="int8", force=True,
            )
        model_dir = str(dest)

    _model_dir = Path(model_dir)
    _translator = ctranslate2.Translator(
        str(_model_dir), device="cpu", compute_type="int8",
    )
    return _translator, _tokenizer


def _nllb_batch(text_list: list[str], src_flores: str, tgt_flores: str) -> list[str]:
    if _translate_batch_impl is not None:
        return _translate_batch_impl(text_list, src_flores, tgt_flores)
    translator, tokenizer = _ensure_model()
    tokenizer.src_lang = src_flores
    out: list[str] = []
    for i in range(0, len(text_list), BATCH_SIZE):
        batch = text_list[i:i + BATCH_SIZE]
        source = [
            tokenizer.convert_ids_to_tokens(tokenizer.encode(t))
            for t in batch
        ]
        result = translator.translate_batch(
            source,
            target_prefix=[[tgt_flores]] * len(batch),
            beam_size=1,
            max_decoding_length=512,
        )
        for row in result:
            hyp = row.hypotheses[0]
            if hyp and hyp[0] == tgt_flores:
                hyp = hyp[1:]
            out.append(
                tokenizer.decode(
                    tokenizer.convert_tokens_to_ids(hyp),
                    skip_special_tokens=True,
                )
            )
    return out


def translate_batch(text_list: list[str], source_code: str | None,
                    target_code: str) -> list[str]:
    """Translate a list of strings. Empty / identical-language items pass through."""
    target_code = normalize_language_code(target_code) or target_code
    tgt_flores = flores_code(target_code)
    if not tgt_flores:
        raise RuntimeError(f"NLLB has no mapping for target language '{target_code}'")
    src_code = normalize_language_code(source_code) or source_code
    src_flores = flores_code(src_code) or "eng_Latn"

    if src_code == target_code:
        return list(text_list)

    # Flatten chunks so we batch across items, then stitch back.
    plan: list[tuple[int, int]] = []  # (item_index, n_chunks)
    flat: list[str] = []
    passthrough: dict[int, str] = {}
    for i, raw in enumerate(text_list):
        text = (raw or "").strip()
        if not text:
            passthrough[i] = raw or ""
            plan.append((i, 0))
            continue
        chunks = _split_chunk(text)
        plan.append((i, len(chunks)))
        flat.extend(chunks)

    translated_flat = _nllb_batch(flat, src_flores, tgt_flores) if flat else []
    cursor = 0
    out: list[str] = []
    for i, n in plan:
        if i in passthrough:
            out.append(passthrough[i])
            continue
        piece = translated_flat[cursor:cursor + n]
        cursor += n
        # Rejoin paragraph chunks with a blank line if the original had one.
        joined = "\n\n".join(p.strip() for p in piece if p.strip())
        out.append(joined)
    return out


def translate_text(text: str, source_code: str | None, target_code: str) -> str:
    if not (text or "").strip():
        return text
    return translate_batch([text], source_code, target_code)[0]


def maybe_translate_text(text: str, job: dict | None = None) -> str:
    """No-op unless this job pinned a translate_to language."""
    target = requested_translate_to()
    if not target or not (text or "").strip():
        return text
    source = detect_language_code(text)
    if source == target:
        record_on_job(job, source, translate_to=target)
        return text
    translated = translate_text(text, source, target)
    record_on_job(job, source, translate_to=target)
    return translated


def maybe_translate_segment(segment_list: list, job: dict | None = None) -> list:
    """Translate `.text` (or `['text']`) on Whisper-style segments in place-ish."""
    target = requested_translate_to()
    if not target or not segment_list:
        return segment_list

    def _get(s, k):
        return s[k] if isinstance(s, dict) else getattr(s, k)

    def _set(s, k, v):
        if isinstance(s, dict):
            s[k] = v
        else:
            setattr(s, k, v)

    sample = " ".join((_get(s, "text") or "") for s in segment_list[:8])
    source = detect_language_code(sample)
    record_on_job(job, source, translate_to=target)
    if source == target:
        return segment_list
    translated = translate_batch(
        [(_get(s, "text") or "") for s in segment_list],
        source, target,
    )
    out = []
    for s, t in zip(segment_list, translated):
        if isinstance(s, dict):
            cloned = dict(s)
            cloned["text"] = t
            out.append(cloned)
        else:
            # faster-whisper NamedTuple-like: build a tiny shim dict
            out.append({"start": _get(s, "start"), "end": _get(s, "end"), "text": t})
    return out
