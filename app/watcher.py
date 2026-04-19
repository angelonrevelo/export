"""Folder watch mode — Clop-style ambient compression.

Set EXPORT_WATCH_DIR=/path/to/dir and any image/video/PDF dropped into that
folder is auto-shrunk into <dir>/_optimized/. The watcher runs in a daemon
thread inside the FastAPI lifespan; no API requests are made.
"""

from __future__ import annotations

import logging
import threading
import time
from pathlib import Path

from watchdog.events import FileSystemEvent, FileSystemEventHandler
from watchdog.observers import Observer

from .jobs import (
    IMAGE_SUFFIXES,
    PDF_SUFFIXES,
    VIDEO_SUFFIXES,
    _shrink_image,
    _shrink_pdf,
    _shrink_video,
)

log = logging.getLogger("export.watcher")

# Suffixes the watcher will act on. Anything else is ignored.
_HANDLED = IMAGE_SUFFIXES | VIDEO_SUFFIXES | PDF_SUFFIXES


class ShrinkWatcher(FileSystemEventHandler):
    def __init__(self, watch_dir: Path, output_dir: Path, level: str = "medium") -> None:
        self.watch_dir = watch_dir
        self.output_dir = output_dir
        self.level = level
        self._processed: set[str] = set()
        output_dir.mkdir(parents=True, exist_ok=True)

    def on_created(self, event: FileSystemEvent) -> None:
        if event.is_directory:
            return
        path = Path(event.src_path)
        if path.suffix.lower() not in _HANDLED:
            return
        if path.name.startswith(".") or path.parent == self.output_dir:
            return
        # Hand off to a worker thread; we don't want to block watchdog's loop
        # while we wait for the file to finish copying.
        threading.Thread(
            target=self._process_when_stable, args=(path,), daemon=True,
        ).start()

    def _process_when_stable(self, path: Path) -> None:
        # Wait until size stops changing — defends against partial writes
        # mid-copy. Bail after 30s if it never settles.
        last_size = -1
        for _ in range(60):
            time.sleep(0.5)
            try:
                cur = path.stat().st_size
            except FileNotFoundError:
                return
            if cur == last_size and cur > 0:
                break
            last_size = cur
        else:
            log.warning("watcher: %s never stabilized, skipping", path.name)
            return

        key = f"{path}::{path.stat().st_mtime}"
        if key in self._processed:
            return
        self._processed.add(key)
        self._optimize(path)

    def _optimize(self, path: Path) -> None:
        suffix = path.suffix.lower()
        try:
            if suffix in IMAGE_SUFFIXES:
                target = _shrink_image(path, self.level, self.output_dir)
            elif suffix in VIDEO_SUFFIXES:
                target = _shrink_video(path, self.level, self.output_dir)
            elif suffix in PDF_SUFFIXES:
                target = _shrink_pdf(path, self.level, self.output_dir)
            else:
                return
            orig_size = path.stat().st_size
            new_size = target.stat().st_size
            ratio = 100 * new_size / orig_size if orig_size else 0
            log.info("watcher: %s -> %s (%d → %d bytes, %.1f%%)",
                     path.name, target.name, orig_size, new_size, ratio)
        except Exception as e:  # don't crash the daemon
            log.error("watcher failed on %s: %s", path.name, e)


def start_watcher(watch_dir: Path, output_dir: Path | None = None,
                  level: str = "medium") -> Observer:
    """Start watching `watch_dir`, returning the running Observer.

    Caller is responsible for `.stop()` + `.join()` at shutdown."""
    watch_dir = Path(watch_dir).expanduser().resolve()
    if not watch_dir.exists():
        watch_dir.mkdir(parents=True, exist_ok=True)
    out = output_dir or (watch_dir / "_optimized")
    out = Path(out).expanduser().resolve()

    handler = ShrinkWatcher(watch_dir, out, level=level)
    observer = Observer()
    observer.schedule(handler, str(watch_dir), recursive=False)
    observer.start()
    log.info("watching %s -> %s (level=%s)", watch_dir, out, level)
    return observer
