"""Refuse URL extract targets that resolve to private / metadata addresses.

Pure: URL → raise SsrfRefused. Tests cover literal IPs without yt-dlp.
"""
from __future__ import annotations

import ipaddress
import socket
from urllib.parse import urlparse

BLOCKED_NETWORK = (
    ipaddress.ip_network("0.0.0.0/8"),
    ipaddress.ip_network("10.0.0.0/8"),
    ipaddress.ip_network("127.0.0.0/8"),
    ipaddress.ip_network("169.254.0.0/16"),
    ipaddress.ip_network("172.16.0.0/12"),
    ipaddress.ip_network("192.168.0.0/16"),
    ipaddress.ip_network("::1/128"),
    ipaddress.ip_network("fc00::/7"),
    ipaddress.ip_network("fe80::/10"),
)


class SsrfRefused(ValueError):
    """URL is not allowed for extraction."""


def is_blocked_ip(ip: ipaddress._BaseAddress) -> bool:
    if ip.is_private or ip.is_loopback or ip.is_link_local or ip.is_multicast or ip.is_reserved:
        return True
    for net in BLOCKED_NETWORK:
        if ip in net:
            return True
    return False


def refuse_ssrf(url: str) -> None:
    parsed = urlparse(url)
    if parsed.scheme not in ("http", "https"):
        raise SsrfRefused("only http/https URLs")
    host = parsed.hostname
    if not host:
        raise SsrfRefused("URL has no host")
    try:
        ip = ipaddress.ip_address(host)
    except ValueError:
        ip = None
    if ip is not None:
        if is_blocked_ip(ip):
            raise SsrfRefused("refused: private or metadata address")
        return
    try:
        info = socket.getaddrinfo(host, None)
    except socket.gaierror as e:
        raise SsrfRefused("could not resolve host") from e
    for item in info:
        addr = item[4][0]
        resolved = ipaddress.ip_address(addr)
        if is_blocked_ip(resolved):
            raise SsrfRefused("refused: private or metadata address")
