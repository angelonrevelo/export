"""In-process per-IP rate limit for mutating /api POSTs.

Default 30/minute. Unittest auto-disables unless EXPORT_RATE_LIMIT_PER_MINUTE
is set (so the existing suite does not 429). 0 disables.

Client IP: the TCP peer, unless that peer is the local reverse proxy
(127.0.0.1 / ::1), in which case X-Real-IP or the rightmost X-Forwarded-For
hop. Never the leftmost XFF — clients control that.
"""
from __future__ import annotations

import ipaddress
import os
import sys
import threading
import time
from collections import defaultdict, deque

_LOCAL_PROXY = frozenset({"127.0.0.1", "::1"})


def _parse_ip(raw: str) -> str | None:
    s = raw.strip().strip("[]")
    if not s:
        return None
    try:
        ipaddress.ip_address(s)
        return s
    except ValueError:
        return None


def client_ip(request) -> str:
    peer = (request.client.host if request.client else "") or "unknown"
    if peer not in _LOCAL_PROXY:
        return peer
    real = _parse_ip(request.headers.get("x-real-ip") or "")
    if real:
        return real
    forwarded = request.headers.get("x-forwarded-for") or ""
    hop = [h.strip() for h in forwarded.split(",") if h.strip()]
    if hop:
        parsed = _parse_ip(hop[-1])
        if parsed:
            return parsed
    return peer

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
