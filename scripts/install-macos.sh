#!/usr/bin/env bash
# One-shot setup for export on macOS. Run from repo root: bash scripts/install-macos.sh
set -euo pipefail

if ! command -v brew >/dev/null 2>&1; then
  echo "homebrew is required: https://brew.sh"
  exit 1
fi

echo ">>> installing system packages (brew)"
brew install ffmpeg pandoc ghostscript poppler tesseract pango cairo gdk-pixbuf glib deno libraw

if [ ! -d "/Applications/LibreOffice.app" ]; then
  echo ">>> installing libreoffice (cask)"
  brew install --cask libreoffice
fi

echo ">>> creating venv"
# Prefer 3.11: torch / faster-whisper / rembg wheels are still flaky on 3.14.
PYTHON=python3
if command -v python3.11 >/dev/null 2>&1; then
  PYTHON=python3.11
fi
echo "    using $($PYTHON --version)"
"$PYTHON" -m venv .venv
./.venv/bin/pip install --upgrade pip
./.venv/bin/pip install -e ".[media]" || ./.venv/bin/pip install -e .

echo ">>> tesseract language packs (fil/spa/cjk/ara/…)"
bash scripts/install-tessdata.sh || echo "WARN: tessdata download failed — OCR stays English until you re-run scripts/install-tessdata.sh"

echo
echo "done."
echo "  next: bash scripts/run.sh   # password is 1234"
echo "  optional offline models: bash scripts/prefetch.sh"
