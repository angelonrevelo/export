"""Opt-in CORS allow-list for a browser app on another origin.

EXPORT_CORS_ORIGIN is a comma-separated list of exact origins
(`https://cut.example,http://127.0.0.1:10100`). Unset or empty = no CORS
headers at all — the same-origin UI on :8500 is all that can call the API,
which is the default. Read per request, so a test can flip it.

Allowed origins get `X-Password` + `Content-Type` as request headers and
`GET, POST, OPTIONS`. A preflight that asks for private-network access
(Chrome's Private Network Access: a public https page calling
`localhost:8500`) also gets `Access-Control-Allow-Private-Network: true`.
Credentials (cookies) are never allowed; auth is the X-Password header.
"""
from __future__ import annotations

import os

ALLOW_METHOD = "GET, POST, OPTIONS"
ALLOW_HEADER = "X-Password, Content-Type"
EXPOSE_HEADER = "Content-Disposition"
MAX_AGE = "600"


def cors_origin() -> frozenset[str]:
    raw = os.environ.get("EXPORT_CORS_ORIGIN", "")
    return frozenset(o.strip().rstrip("/") for o in raw.split(",") if o.strip())


def _header(scope) -> dict[str, str]:
    return {
        k.decode("latin-1").lower(): v.decode("latin-1")
        for k, v in scope.get("headers", [])
    }


class CorsAllowList:
    """Pure ASGI middleware. Passes through untouched unless Origin is allowed."""

    def __init__(self, app):
        self.app = app

    async def __call__(self, scope, receive, send):
        if scope["type"] != "http":
            await self.app(scope, receive, send)
            return
        header = _header(scope)
        origin = header.get("origin")
        if not origin or origin not in cors_origin():
            await self.app(scope, receive, send)
            return

        base = [
            (b"access-control-allow-origin", origin.encode("latin-1")),
            (b"vary", b"Origin"),
        ]
        if scope["method"] == "OPTIONS" and "access-control-request-method" in header:
            out = base + [
                (b"access-control-allow-methods", ALLOW_METHOD.encode()),
                (b"access-control-allow-headers", ALLOW_HEADER.encode()),
                (b"access-control-max-age", MAX_AGE.encode()),
                (b"content-length", b"0"),
            ]
            if header.get("access-control-request-private-network", "").lower() == "true":
                out.append((b"access-control-allow-private-network", b"true"))
            await send({"type": "http.response.start", "status": 204, "headers": out})
            await send({"type": "http.response.body", "body": b""})
            return

        async def send_with_cors(message):
            if message["type"] == "http.response.start":
                message = {
                    **message,
                    "headers": list(message.get("headers", [])) + base
                    + [(b"access-control-expose-headers", EXPOSE_HEADER.encode())],
                }
            await send(message)

        await self.app(scope, receive, send_with_cors)
