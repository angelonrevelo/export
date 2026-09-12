#!/usr/bin/env bash
# Pre-download ML models so EXPORT_OFFLINE=1 jobs don't hit the network.
# Safe to re-run — huggingface / rembg / demucs caches skip existing files.
set -euo pipefail

cd "$(dirname "$0")/.."

if [ ! -x .venv/bin/python ]; then
  echo "run scripts/install-macos.sh (or install-linux.sh) first" >&2
  exit 1
fi

echo ">>> tesseract language packs"
bash scripts/install-tessdata.sh

echo ">>> faster-whisper ${EXPORT_WHISPER_MODEL:-large-v3-turbo} (CPU int8)"
./.venv/bin/python - <<'PY'
import os
from faster_whisper import WhisperModel
name = os.environ.get("EXPORT_WHISPER_MODEL", "large-v3-turbo")
root = os.environ.get("EXPORT_WHISPER_MODEL_DIR") or None
print(f"downloading {name} …")
WhisperModel(name, device="cpu", compute_type="int8", download_root=root)
print("whisper cached")
PY

echo ">>> rembg u2net"
./.venv/bin/python - <<'PY'
from rembg import new_session
new_session("u2net")
print("rembg u2net cached")
PY

echo ">>> NLLB-200 distilled 600M (CTranslate2 int8, ~1.2 GB)"
./.venv/bin/python - <<'PY'
from app.translate import translate_text
# Force a model load + a 3-word roundtrip so the tokenizer AND weights cache.
print(translate_text("hello world", "en", "es"))
print("nllb cached")
PY

echo
echo "done. you can now set EXPORT_OFFLINE=1 in .env"
echo "  (URL extraction and Groq still need a network — they error closed.)"
