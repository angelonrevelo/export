"""Unusual file ops: dates, Finder tags, unlock, decrypt, scrub, comment, hidden.

Browser downloads reset mtime and drop xattrs. On macOS, Reveal in Finder
lets you move the job copy with tags and dates intact. Bulk date/tag leaves
per-file copies on disk (zip is extra; zip drops xattrs).
"""
from __future__ import annotations

import os
import plistlib
import shutil
import stat
import struct
import subprocess
import sys
import time
import zipfile
from datetime import datetime
from pathlib import Path

from PIL import Image, ExifTags

from .jobs import _pdf_decrypt, jobs, mark_ready

META_OP = frozenset({
    "date", "tag", "unlock", "decrypt",
    "scrub", "comment", "hidden",
})
TAG_XATTR = "com.apple.metadata:_kMDItemUserTags"
QUARANTINE_XATTR = "com.apple.quarantine"
COMMENT_XATTR = "com.apple.metadata:kMDItemFinderComment"
WHERE_FROM_XATTR = "com.apple.metadata:kMDItemWhereFroms"
FINDER_INFO_XATTR = "com.apple.FinderInfo"
RESOURCE_FORK_XATTR = "com.apple.ResourceFork"
# FinderInfo finderFlags bit (big-endian UInt16 at offset 8).
HAS_CUSTOM_ICON = 0x0400
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


def strip_xattr(path: Path) -> None:
    for name in list(_xattr_list(path)):
        _xattr_remove(path, name)


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


def write_comment(path: Path, comment: str) -> None:
    text = (comment or "").strip()
    if not text:
        _xattr_remove(path, COMMENT_XATTR)
        return
    _xattr_set(path, COMMENT_XATTR, plistlib.dumps(text, fmt=plistlib.FMT_BINARY))


def read_comment(path: Path) -> str:
    data = _xattr_get(path, COMMENT_XATTR)
    if not data:
        return ""
    val = plistlib.loads(data)
    return str(val) if val is not None else ""


def write_where_from(path: Path, url: str) -> None:
    text = (url or "").strip()
    if not text:
        _xattr_remove(path, WHERE_FROM_XATTR)
        return
    _xattr_set(path, WHERE_FROM_XATTR, plistlib.dumps([text], fmt=plistlib.FMT_BINARY))


def read_where_from(path: Path) -> str:
    data = _xattr_get(path, WHERE_FROM_XATTR)
    if not data:
        return ""
    val = plistlib.loads(data)
    if isinstance(val, list) and val:
        return str(val[0])
    return str(val) if val else ""


def clear_quarantine(path: Path) -> None:
    _xattr_remove(path, QUARANTINE_XATTR)
    try:
        flags = path.stat().st_flags  # type: ignore[attr-defined]
        os.chflags(path, flags & ~getattr(stat, "UF_IMMUTABLE", 0))  # type: ignore[attr-defined]
    except (AttributeError, OSError):
        pass


def _hidden_flag() -> int:
    return int(getattr(stat, "UF_HIDDEN", 0x8000))


def set_hidden(path: Path, is_hidden: bool) -> None:
    if not is_darwin():
        raise ValueError("hidden flag is macOS-only")
    flags = path.stat().st_flags  # type: ignore[attr-defined]
    bit = _hidden_flag()
    os.chflags(path, (flags | bit) if is_hidden else (flags & ~bit))  # type: ignore[attr-defined]


def is_hidden(path: Path) -> bool:
    try:
        return bool(path.stat().st_flags & _hidden_flag())  # type: ignore[attr-defined]
    except AttributeError:
        return False


def clear_icon(path: Path) -> None:
    """Clear Finder custom-icon bit and resource fork."""
    info = _xattr_get(path, FINDER_INFO_XATTR)
    if info and len(info) >= 10:
        buf = bytearray(info)
        flags = struct.unpack_from(">H", buf, 8)[0]
        struct.pack_into(">H", buf, 8, flags & ~HAS_CUSTOM_ICON)
        if any(buf):
            _xattr_set(path, FINDER_INFO_XATTR, bytes(buf))
        else:
            _xattr_remove(path, FINDER_INFO_XATTR)
    else:
        _xattr_remove(path, FINDER_INFO_XATTR)
    _xattr_remove(path, RESOURCE_FORK_XATTR)


def plant_custom_icon(path: Path) -> None:
    """Test helper: FinderInfo custom-icon bit + dummy resource fork."""
    buf = bytearray(32)
    struct.pack_into(">H", buf, 8, HAS_CUSTOM_ICON)
    _xattr_set(path, FINDER_INFO_XATTR, bytes(buf))
    _xattr_set(path, RESOURCE_FORK_XATTR, b"\x00ICON\x00")


def has_custom_icon(path: Path) -> bool:
    info = _xattr_get(path, FINDER_INFO_XATTR)
    if info and len(info) >= 10:
        flags = struct.unpack_from(">H", info, 8)[0]
        if flags & HAS_CUSTOM_ICON:
            return True
    return _xattr_get(path, RESOURCE_FORK_XATTR) is not None


def _dest(src: Path, out_dir: Path) -> Path:
    return out_dir / src.name.removeprefix("input_")


def strip_jpeg_exif(data: bytes) -> bytes:
    """Drop APP1 Exif segments without re-encoding scan data."""
    if len(data) < 4 or data[:2] != b"\xff\xd8":
        return data
    out = bytearray(b"\xff\xd8")
    i = 2
    n = len(data)
    while i < n:
        if data[i] != 0xFF:
            out.extend(data[i:])
            break
        while i < n and data[i] == 0xFF:
            i += 1
        if i >= n:
            break
        marker = data[i]
        i += 1
        if marker == 0xD9:
            out.extend(b"\xff\xd9")
            break
        if marker == 0xDA:
            out.extend(b"\xff\xda")
            out.extend(data[i:])
            break
        if 0xD0 <= marker <= 0xD7:
            out.extend(bytes((0xFF, marker)))
            continue
        if i + 2 > n:
            break
        seglen = int.from_bytes(data[i:i + 2], "big")
        body = data[i + 2:i + seglen]
        if marker == 0xE1 and body.startswith(b"Exif\x00\x00"):
            i += seglen
            continue
        out.extend(bytes((0xFF, marker)))
        out.extend(data[i:i + seglen])
        i += seglen
    return bytes(out)


def _gps_present(path: Path) -> bool:
    try:
        with Image.open(path) as img:
            exif = img.getexif()
            if not exif:
                return False
            if 0x8825 in exif:
                return True
            gps = exif.get_ifd(0x8825)
            return bool(gps)
    except Exception:
        return False


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


def apply_comment(src: Path, out_dir: Path, comment: str = "", where_from: str = "") -> Path:
    dest = _dest(src, out_dir)
    shutil.copy2(src, dest)
    write_comment(dest, comment)
    write_where_from(dest, where_from)
    return dest


def apply_hidden(src: Path, out_dir: Path, mode: str = "hide") -> Path:
    dest = _dest(src, out_dir)
    shutil.copy2(src, dest)
    m = (mode or "hide").strip().lower()
    if m in {"hide", "hidden", "1", "true", "on"}:
        set_hidden(dest, True)
    elif m in {"show", "unhide", "clear-hidden", "0", "false", "off"}:
        set_hidden(dest, False)
    elif m in {"clear-icon", "icon", "clear_icon"}:
        clear_icon(dest)
    else:
        raise ValueError(f"unknown hidden mode: {mode}")
    return dest


def apply_scrub(src: Path, out_dir: Path) -> Path:
    dest = _dest(src, out_dir)
    suffix = src.suffix.lower()
    if suffix in {".jpg", ".jpeg"}:
        dest.write_bytes(strip_jpeg_exif(src.read_bytes()))
    elif suffix in {".png", ".webp", ".tif", ".tiff"}:
        with Image.open(src) as img:
            px = img.copy()
            fmt = img.format or suffix.lstrip(".").upper()
            if fmt == "JPG":
                fmt = "JPEG"
            save_kw: dict = {}
            px.save(dest, format=fmt, **save_kw)
    else:
        shutil.copy2(src, dest)
    strip_xattr(dest)
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
    if suffix == ".7z":
        if not password:
            raise ValueError("password is required")
        import py7zr
        dest_dir = out_dir / "unlocked"
        dest_dir.mkdir(parents=True, exist_ok=True)
        try:
            with py7zr.SevenZipFile(src, mode="r", password=password) as zf:
                zf.extractall(path=dest_dir)
        except Exception as e:
            raise ValueError(f"wrong password or bad 7z: {e}") from e
        inner = [p for p in dest_dir.rglob("*") if p.is_file()]
        if not inner:
            raise ValueError("7z extracted nothing")
        if len(inner) == 1:
            return inner[0]
        zip_out = out_dir / (src.stem.removeprefix("input_") + "_unlocked.zip")
        with zipfile.ZipFile(zip_out, "w", compression=zipfile.ZIP_DEFLATED) as out:
            for p in inner:
                out.write(p, arcname=str(p.relative_to(dest_dir)))
        return zip_out
    if suffix in {".docx", ".xlsx", ".pptx"}:
        if not password:
            raise ValueError("password is required")
        import msoffcrypto
        from msoffcrypto.exceptions import InvalidKeyError, FileFormatError
        dest = out_dir / (src.stem.removeprefix("input_") + "_unlocked" + suffix)
        try:
            with src.open("rb") as fh:
                office = msoffcrypto.OfficeFile(fh)
                office.load_key(password=password, verify_password=True)
                with dest.open("wb") as out:
                    office.decrypt(out)
        except InvalidKeyError as e:
            raise ValueError("wrong password") from e
        except FileFormatError as e:
            raise ValueError(f"not an encrypted Office file: {e}") from e
        except Exception as e:
            raise ValueError(f"office decrypt failed: {e}") from e
        return dest
    raise ValueError(f"no decrypt for {suffix or '(no ext)'} — try unlock for quarantine")


def reveal(path: Path) -> None:
    if not is_darwin():
        raise ValueError("show in Finder is macOS-only")
    subprocess.run(["open", "-R", str(path)], check=True, timeout=15)


def _apply_one(
    src: Path,
    out_dir: Path,
    op: str,
    modified_at: str,
    tag: str,
    password: str,
    is_from_exif: bool,
    comment: str,
    where_from: str,
    mode: str,
) -> Path:
    if op == "date":
        return apply_date(src, out_dir, modified_at, is_from_exif=is_from_exif)
    if op == "tag":
        return apply_tag(src, out_dir, tag)
    if op == "unlock":
        return apply_unlock(src, out_dir)
    if op == "decrypt":
        return apply_decrypt(src, out_dir, password)
    if op == "scrub":
        return apply_scrub(src, out_dir)
    if op == "comment":
        return apply_comment(src, out_dir, comment, where_from)
    if op == "hidden":
        return apply_hidden(src, out_dir, mode)
    raise ValueError(f"unknown meta op: {op}")


def run_meta_job(
    job_id: str,
    input_path: Path | list[Path],
    op: str,
    downloads_dir: Path,
    modified_at: str = "",
    tag: str = "",
    password: str = "",
    is_from_exif: bool = False,
    comment: str = "",
    where_from: str = "",
    mode: str = "",
) -> None:
    job = jobs[job_id]
    out_dir = downloads_dir / job_id
    try:
        job["status"] = "processing"
        src_list = input_path if isinstance(input_path, list) else [input_path]
        result: list[Path] = []
        for src in src_list:
            result.append(_apply_one(
                src, out_dir, op, modified_at, tag, password,
                is_from_exif, comment, where_from, mode,
            ))
        for src in src_list:
            src.unlink(missing_ok=True)
        if len(result) == 1:
            target = result[0]
        else:
            zip_path = out_dir / "batch.zip"
            with zipfile.ZipFile(zip_path, "w", compression=zipfile.ZIP_DEFLATED) as zf:
                for p in result:
                    zf.write(p, arcname=p.name)
            target = zip_path
        mark_ready(job, target)
        job["copy"] = [p.name for p in result]
        job["is_reveal"] = is_darwin()
    except Exception as e:
        job["status"] = "error"
        job["error"] = str(e) or e.__class__.__name__
