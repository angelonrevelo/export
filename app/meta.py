"""Unusual file ops: dates, Finder tags, unlock (quarantine/lock), decrypt.

Browser downloads reset mtime and drop xattrs. On macOS, Reveal in Finder
lets you move the job copy with tags and dates intact.
"""
from __future__ import annotations

import os
import plistlib
import shutil
import stat
import subprocess
import sys
import time
import zipfile
from datetime import datetime
from pathlib import Path

from PIL import Image, ExifTags

from .jobs import _pdf_decrypt, jobs, mark_ready

META_OP = frozenset({"date", "tag", "unlock", "decrypt"})
TAG_XATTR = "com.apple.metadata:_kMDItemUserTags"
QUARANTINE_XATTR = "com.apple.quarantine"
TAG_COLOR = {
    "gray": 1, "grey": 1, "green": 2, "purple": 3,
    "blue": 4, "yellow": 5, "red": 6, "orange": 7,
}


def is_darwin() -> bool:
    return sys.platform == "darwin"


def parse_when(raw: str) -> float:
    s = (raw or "").strip()
    if not s:
        raise ValueError("need modified_at")
    if s.endswith("Z"):
        s = s[:-1] + "+00:00"
    dt = datetime.fromisoformat(s)
    return dt.timestamp()


def exif_datetime(path: Path) -> float | None:
    try:
        with Image.open(path) as img:
            exif = img.getexif()
            if not exif:
                return None
            by_name = {ExifTags.TAGS.get(k, k): v for k, v in exif.items()}
            raw = by_name.get("DateTimeOriginal") or by_name.get("DateTime")
            if not raw:
                return None
            dt = datetime.strptime(str(raw), "%Y:%m:%d %H:%M:%S")
            return dt.timestamp()
    except Exception:
        return None


def set_birthtime(path: Path, timestamp: float) -> None:
    """Set st_birthtime (created) on macOS. No-op elsewhere."""
    if not is_darwin():
        return
    import ctypes
    from ctypes import Structure, c_char_p, c_int64, c_long, c_uint16, c_uint32, c_ulong, byref, sizeof

    class timespec(Structure):
        _fields_ = [("tv_sec", c_int64), ("tv_nsec", c_long)]

    class attrlist(Structure):
        _fields_ = [
            ("bitmapcount", c_uint16),
            ("reserved", c_uint16),
            ("commonattr", c_uint32),
            ("volattr", c_uint32),
            ("dirattr", c_uint32),
            ("fileattr", c_uint32),
            ("forkattr", c_uint32),
        ]

    ATTR_BIT_MAP_COUNT = 5
    ATTR_CMN_CRTIME = 0x00000200
    libc = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    al = attrlist()
    al.bitmapcount = ATTR_BIT_MAP_COUNT
    al.commonattr = ATTR_CMN_CRTIME
    ts = timespec(int(timestamp), int((timestamp % 1) * 1_000_000_000))
    rc = libc.setattrlist(
        c_char_p(os.fsencode(path)),
        byref(al),
        byref(ts),
        sizeof(ts),
        c_ulong(0),
    )
    if rc != 0:
        raise OSError(ctypes.get_errno(), "setattrlist ATTR_CMN_CRTIME failed")


def tag_entry(raw: str) -> list[str]:
    """Finder tag strings: 'Red\\n6'. Empty input clears tags."""
    entry: list[str] = []
    for part in (raw or "").split(","):
        name = part.strip()
        if not name:
            continue
        key = name.lower()
        if key in TAG_COLOR:
            entry.append(f"{key.title()}\n{TAG_COLOR[key]}")
        else:
            entry.append(f"{name}\n0")
    return entry


def _libc():
    import ctypes
    lib = ctypes.CDLL("/usr/lib/libSystem.B.dylib", use_errno=True)
    lib.getxattr.restype = ctypes.c_ssize_t
    lib.listxattr.restype = ctypes.c_ssize_t
    return lib


def _xattr_set(path: Path, name: str, value: bytes) -> None:
    if hasattr(os, "setxattr"):
        os.setxattr(path, name, value)
        return
    if not is_darwin():
        raise OSError("xattr not supported")
    import ctypes
    libc = _libc()
    buf = (ctypes.c_char * len(value)).from_buffer_copy(value)
    rc = libc.setxattr(os.fsencode(path), name.encode(), buf, len(value), 0, 0)
    if rc != 0:
        raise OSError(ctypes.get_errno(), "setxattr failed")


def _xattr_get(path: Path, name: str) -> bytes | None:
    if hasattr(os, "getxattr"):
        try:
            return os.getxattr(path, name)
        except OSError:
            return None
    if not is_darwin():
        return None
    import ctypes
    libc = _libc()
    size = libc.getxattr(os.fsencode(path), name.encode(), None, 0, 0, 0)
    if size < 0:
        return None
    buf = ctypes.create_string_buffer(size)
    got = libc.getxattr(os.fsencode(path), name.encode(), buf, size, 0, 0)
    if got < 0:
        return None
    return buf.raw[:got]


def _xattr_remove(path: Path, name: str) -> None:
    if hasattr(os, "removexattr"):
        try:
            os.removexattr(path, name)
        except OSError:
            pass
        return
    if not is_darwin():
        return
    _libc().removexattr(os.fsencode(path), name.encode(), 0)


def _xattr_list(path: Path) -> list[str]:
    if hasattr(os, "listxattr"):
        try:
            return list(os.listxattr(path))
        except OSError:
            return []
    if not is_darwin():
        return []
    libc = _libc()
    size = libc.listxattr(os.fsencode(path), None, 0, 0)
    if size <= 0:
        return []
    import ctypes
    buf = ctypes.create_string_buffer(size)
    got = libc.listxattr(os.fsencode(path), buf, size, 0)
    if got <= 0:
        return []
    return [n.decode() for n in buf.raw[:got].split(b"\x00") if n]


def write_tag(path: Path, raw: str) -> None:
    entry = tag_entry(raw)
    payload = plistlib.dumps(entry, fmt=plistlib.FMT_BINARY)
    _xattr_set(path, TAG_XATTR, payload)


def read_tag(path: Path) -> list[str]:
    data = _xattr_get(path, TAG_XATTR)
    if not data:
        return []
    arr = plistlib.loads(data)
    out: list[str] = []
    for item in arr:
        name = str(item).split("\n", 1)[0]
        if name:
            out.append(name)
    return out


def clear_quarantine(path: Path) -> None:
    _xattr_remove(path, QUARANTINE_XATTR)
    try:
        flags = path.stat().st_flags  # type: ignore[attr-defined]
        os.chflags(path, flags & ~getattr(stat, "UF_IMMUTABLE", 0))  # type: ignore[attr-defined]
    except (AttributeError, OSError):
        pass


def _dest(src: Path, out_dir: Path) -> Path:
    return out_dir / src.name.removeprefix("input_")


def apply_date(
    src: Path,
    out_dir: Path,
    modified_at: str = "",
    is_from_exif: bool = False,
) -> Path:
    dest = _dest(src, out_dir)
    shutil.copy2(src, dest)
    if is_from_exif:
        ts = exif_datetime(src)
        if ts is None:
            raise ValueError("no EXIF date on this file")
    else:
        ts = parse_when(modified_at) if modified_at.strip() else time.time()
    if is_darwin():
        set_birthtime(dest, ts)
    os.utime(dest, (ts, ts))
    return dest


def apply_tag(src: Path, out_dir: Path, tag: str) -> Path:
    dest = _dest(src, out_dir)
    shutil.copy2(src, dest)
    write_tag(dest, tag)
    return dest


def apply_unlock(src: Path, out_dir: Path) -> Path:
    dest = _dest(src, out_dir)
    shutil.copy2(src, dest)
    clear_quarantine(dest)
    return dest


def apply_decrypt(src: Path, out_dir: Path, password: str) -> Path:
    suffix = src.suffix.lower()
    if suffix == ".pdf":
        return _pdf_decrypt(src, password, out_dir)
    if suffix in {".zip", ".cbz"}:
        if not password:
            raise ValueError("password is required")
        dest_dir = out_dir / "unlocked"
        dest_dir.mkdir(parents=True, exist_ok=True)
        with zipfile.ZipFile(src) as zf:
            zf.extractall(dest_dir, pwd=password.encode("utf-8"))
        inner = [p for p in dest_dir.rglob("*") if p.is_file()]
        if len(inner) == 1:
            return inner[0]
        zip_out = out_dir / (src.stem.removeprefix("input_") + "_unlocked.zip")
        with zipfile.ZipFile(zip_out, "w", compression=zipfile.ZIP_DEFLATED) as out:
            for p in inner:
                out.write(p, arcname=str(p.relative_to(dest_dir)))
        return zip_out
    raise ValueError(f"no decrypt for {suffix or '(no ext)'} — try unlock for quarantine")


def reveal(path: Path) -> None:
    if not is_darwin():
        raise ValueError("show in Finder is macOS-only")
    subprocess.run(["open", "-R", str(path)], check=True, timeout=15)


def run_meta_job(
    job_id: str,
    input_path: Path,
    op: str,
    downloads_dir: Path,
    modified_at: str = "",
    tag: str = "",
    password: str = "",
    is_from_exif: bool = False,
) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "processing"
        if op == "date":
            target = apply_date(input_path, out_dir, modified_at, is_from_exif=is_from_exif)
        elif op == "tag":
            target = apply_tag(input_path, out_dir, tag)
        elif op == "unlock":
            target = apply_unlock(input_path, out_dir)
        elif op == "decrypt":
            target = apply_decrypt(input_path, out_dir, password)
        else:
            raise ValueError(f"unknown meta op: {op}")
        input_path.unlink(missing_ok=True)
        mark_ready(job, target)
        job["is_reveal"] = is_darwin()
    except Exception as e:
        job["status"] = "error"
        job["error"] = str(e) or e.__class__.__name__
