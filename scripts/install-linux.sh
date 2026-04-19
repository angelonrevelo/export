#!/usr/bin/env bash
# One-shot setup for export on Debian/Ubuntu. Run from repo root: bash scripts/install-linux.sh
# For other distros, translate the apt packages and re-run from the venv step.
set -euo pipefail

echo ">>> installing system packages (apt)"
sudo apt-get update
sudo apt-get install -y \
  python3 python3-venv python3-pip curl unzip \
  ffmpeg pandoc ghostscript poppler-utils tesseract-ocr \
  libreoffice \
  libpango-1.0-0 libpangoft2-1.0-0 libcairo2 libgdk-pixbuf-2.0-0 \
  libffi-dev shared-mime-info

# Deno for yt-dlp's YouTube JS runtime (Apt repo doesn't ship deno; one-line install)
if ! command -v deno >/dev/null 2>&1; then
  curl -fsSL https://deno.land/install.sh | DENO_INSTALL=/usr/local sudo -E sh
fi

echo ">>> creating venv"
python3 -m venv .venv
./.venv/bin/pip install --upgrade pip
./.venv/bin/pip install -e .

echo ">>> downloading piper voice (en_US-lessac-medium, ~60MB)"
mkdir -p voices
if [ ! -f voices/en_US-lessac-medium.onnx ]; then
  curl -L -o voices/en_US-lessac-medium.onnx \
    https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx
  curl -L -o voices/en_US-lessac-medium.onnx.json \
    https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json
fi

echo
echo "done."
echo "  set in your .env or shell:"
echo "    EXPORT_PIPER_VOICE=$(pwd)/voices/en_US-lessac-medium.onnx"
echo "  then: bash scripts/run.sh"
