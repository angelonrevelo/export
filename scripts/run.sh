#!/usr/bin/env bash
# Launch uvicorn with the right env for the platform. Run from repo root.
set -euo pipefail
cd "$(dirname "$0")/.."

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
