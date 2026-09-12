"""In-process per-IP rate limit for mutating /api POSTs.

Default 30/minute. Unittest auto-disables unless EXPORT_RATE_LIMIT_PER_MINUTE
is set (so the existing suite does not 429). 0 disables.
"""
from __future__ import annotations

import os
import sys
import threading
import time
from collections import defaultdict, deque

_lock = threading.Lock()
_hit: dict[str, deque[float]] = defaultdict(deque)


class RateLimited(Exception):
    """Too many mutating requests from this IP."""


def reset() -> None:
    with _lock:
        _hit.clear()


def per_minute() -> int | None:
    """None = skip (unittest default); 0 = off; N = cap."""
    raw = os.environ.get("EXPORT_RATE_LIMIT_PER_MINUTE")
    if raw is None:
        if "unittest" in sys.modules:
            return None
        return 30
    try:
        return max(0, int(raw))
    except ValueError:
        return 30


def enforce(ip: str) -> None:
    n = per_minute()
    if n is None or n <= 0:
        return
    now = time.monotonic()
    with _lock:
        q = _hit[ip or "unknown"]
        while q and now - q[0] > 60.0:
            q.popleft()
        if len(q) >= n:
            raise RateLimited("too many requests — wait a minute")
        q.append(now)
