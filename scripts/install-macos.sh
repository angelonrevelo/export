#!/usr/bin/env bash
# One-shot setup for export on macOS. Run from repo root: bash scripts/install-macos.sh
set -euo pipefail

if ! command -v brew >/dev/null 2>&1; then
  echo "homebrew is required: https://brew.sh"
  exit 1
fi

echo ">>> installing system packages (brew)"
brew install ffmpeg pandoc ghostscript poppler tesseract pango cairo gdk-pixbuf glib deno

if [ ! -d "/Applications/LibreOffice.app" ]; then
  echo ">>> installing libreoffice (cask)"
  brew install --cask libreoffice
fi

echo ">>> creating venv"
python3 -m venv .venv
./.venv/bin/pip install --upgrade pip
./.venv/bin/pip install -e .

echo
echo "done."
echo "  next: copy .env.example to .env, edit EXPORT_PASSWORD, then:"
echo "  bash scripts/run.sh"
