#!/usr/bin/env bash
# Download a curated global Tesseract language pack set (tessdata_fast).
# Idempotent — skips packs that already exist. Safe to re-run.
set -euo pipefail

LANGS=(
  fil spa fra deu por ita nld pol tur ukr ces ron hun
  chi_sim chi_tra jpn kor
  ara rus hin tha vie ind
  ben tam ell heb
)

find_tessdata() {
  if [ -n "${TESSDATA_PREFIX:-}" ] && [ -d "${TESSDATA_PREFIX}" ]; then
    echo "${TESSDATA_PREFIX}"
    return
  fi
  if command -v brew >/dev/null 2>&1; then
    local brew_dir
    brew_dir="$(brew --prefix tesseract 2>/dev/null || true)/share/tessdata"
    if [ -d "${brew_dir}" ]; then
      echo "${brew_dir}"
      return
    fi
  fi
  for d in \
    /usr/share/tesseract-ocr/5/tessdata \
    /usr/share/tesseract-ocr/4.00/tessdata \
    /usr/share/tessdata \
    /opt/homebrew/share/tessdata \
    /usr/local/share/tessdata
  do
    if [ -d "$d" ]; then
      echo "$d"
      return
    fi
  done
  echo "could not find tessdata directory. install tesseract first." >&2
  exit 1
}

DEST="$(find_tessdata)"
echo ">>> tessdata dir: ${DEST}"

# tessdata_fast matches Homebrew's tesseract formula. The full tessdata repo
# is 2–4× larger per pack for a small accuracy bump we don't need here.
BASE="https://github.com/tesseract-ocr/tessdata_fast/raw/main"

need_sudo=""
if [ ! -w "${DEST}" ]; then
  need_sudo="sudo"
  echo ">>> ${DEST} is not writable; using sudo"
fi

for lang in "${LANGS[@]}"; do
  target="${DEST}/${lang}.traineddata"
  if [ -f "${target}" ]; then
    echo "  skip ${lang} (already installed)"
    continue
  fi
  echo "  fetch ${lang}"
  tmp="$(mktemp)"
  if curl -fsSL -o "${tmp}" "${BASE}/${lang}.traineddata"; then
    ${need_sudo} mv "${tmp}" "${target}"
  else
    echo "  WARN: failed to fetch ${lang}" >&2
    rm -f "${tmp}"
  fi
done

echo ">>> installed packs:"
tesseract --list-langs 2>/dev/null || true
