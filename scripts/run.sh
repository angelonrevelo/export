#!/usr/bin/env bash
# Launch uvicorn with the right env for the platform. Run from repo root.
set -euo pipefail
cd "$(dirname "$0")/.."

# Auto-bootstrap .env so `npm run dev` / `bash scripts/run.sh` work first try.
# Writes a random EXPORT_PASSWORD; everything else stays at .env.example defaults.
if [ ! -f .env ] && [ -f .env.example ]; then
  cp .env.example .env
  RANDOM_PW="$(openssl rand -base64 24 2>/dev/null | tr -d '/+=' | head -c 32)"
  if [ -n "$RANDOM_PW" ]; then
    # in-place sed that works on both BSD (mac) and GNU sed
    sed -i.bak "s/^EXPORT_PASSWORD=changeme/EXPORT_PASSWORD=${RANDOM_PW}/" .env && rm .env.bak
    echo ">>> created .env with random EXPORT_PASSWORD: ${RANDOM_PW}"
  else
    echo ">>> created .env from .env.example — edit EXPORT_PASSWORD before exposing publicly"
  fi
fi

# load .env if present (export each KEY=VALUE line, ignore comments)
if [ -f .env ]; then
  set -a
  # shellcheck disable=SC1091
  source .env
  set +a
fi

# WeasyPrint needs to find libgobject etc.
case "$(uname -s)" in
  Darwin)
    export DYLD_FALLBACK_LIBRARY_PATH="/opt/homebrew/lib:${DYLD_FALLBACK_LIBRARY_PATH:-}"
    ;;
  Linux)
    # Most distros put these in /usr/lib/x86_64-linux-gnu — leave default unless missing.
    : "${LD_LIBRARY_PATH:=}"
    ;;
esac

: "${EXPORT_PASSWORD:?EXPORT_PASSWORD is required (set in .env or shell)}"
: "${EXPORT_DOWNLOADS:=/tmp/export}"
: "${PORT:=8500}"
: "${HOST:=127.0.0.1}"

mkdir -p "$EXPORT_DOWNLOADS"

exec ./.venv/bin/uvicorn app.main:app --host "$HOST" --port "$PORT"
