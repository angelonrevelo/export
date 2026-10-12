#!/usr/bin/env python3
"""Outside score for real uploads in this git repository or a local corpus.

Real uploads are private, so they do not have to be committed: files under
`corpus/` (gitignored; or --corpus / EXPORT_CORPUS) are read from disk and
counted exactly like committed ones.

The Python app is the reference. This script is a host oracle: it may call
that app, and it may call a separate rust program named by EXPORT_RUST_ORACLE.
It is not linked into library code, and it does not generate substitute files.

N is how many counted files the rust conversion matches, or keeps inside the
tolerance the Python tests already use. That tolerance is not widened here.
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

# Byte cap per shrink kind. The advertised 20 MB maximum
# (app/main.py shrink_target_byte) is larger than almost every phone photo, so
# the app copies the source and a converter that only copies would score. Score
# against caps that force a real re-encode: 1 MB for a photo, and 5 MB for
# video (the smallest preset in static/index.html).
SHRINK_CAP = {"phone-jpeg": 1_000_000, "heic": 1_000_000, "mp4": 5_000_000}

# tests/test_meta.py: abs(mtime - want) < 2. Filesystem clocks only.
# Not applied to converted bytes.
MTIME_SLACK_SECONDS = 2

# Camera-phone EXIF markers. A JPEG without one of these is not counted as a
# phone JPEG. Desktop and synthetic files stay out of M.
PHONE_MARKERS = (
    "iphone",
    "ipad",
    "apple",
    "samsung",
    "pixel",
    "google",
    "huawei",
    "xiaomi",
    "redmi",
    "oppo",
    "vivo",
    "oneplus",
    "motorola",
    "nokia",
    "sony xperia",
    "fairphone",
    "nothing phone",
    "realme",
    "honor",
)

# Counted kinds. Anything else in git is reported and not added to M.
COUNTED_KINDS = ("phone-jpeg", "heic", "docx", "pdf", "xlsx", "mp4")

# Recorded on export-image (cursor/export-image-053d). The bench was reverted,
# so the PNG is not a repository upload and is not part of M. Listed first
# because it is the real-sized miss. No timing is stored here.
REAL_SIZED_MISSES = (
    {
        "id": "6144x4608-png-shrink",
        "pixels": "6144×4608",
        "cap_bytes": 8000,
        "area_scales": 6,
        "result": "OverBudget",
        "output_bytes": None,
        "where": "export-image shrink_png",
        "in_m": False,
    },
)

TOLERANCE = (
    "Byte-target shrink, source larger than the cap: the output size equals "
    "the cap exactly (tests/test_shrink.py assertEqual on the byte count). "
    "The image output is a JPEG (format JPEG, leading FF D8). Those tests do "
    "not compare pixel payloads, so two JPEGs of that same size meet this "
    "check. Byte-target shrink, source at or under the cap: the output size "
    "equals the source size (assertEqual against the source) and the bytes "
    "are the copy the app writes. Other conversions: exact bytes "
    "(tests/test_convert.py assertEqual on structure, suffix, and pixel "
    "size). The only numeric slack in the Python tests is filesystem time, "
    "abs(got - want) < 2 seconds (tests/test_meta.py). That slack is not "
    "applied to file bytes. Timing bounds are not a match tolerance."
)


def is_generated(path: str) -> bool:
    """Fixture, bench, and test trees are not uploads."""
    lowered = path.replace("\\", "/").lower()
    if lowered.startswith("tests/") or "/tests/" in lowered:
        return True
    if lowered.startswith("benches/") or "/benches/" in lowered:
        return True
    if "/fixtures/" in lowered or lowered.startswith("fixtures/"):
        return True
    if "/testdata/" in lowered or lowered.startswith("testdata/"):
        return True
    return False


def suffix_kind(path: str) -> str | None:
    suffix = Path(path).suffix.lower()
    return {
        ".jpg": "jpeg",
        ".jpeg": "jpeg",
        ".heic": "heic",
        ".heif": "heic",
        ".docx": "docx",
        ".pdf": "pdf",
        ".xlsx": "xlsx",
        ".mp4": "mp4",
    }.get(suffix)


def is_phone_exif(make: str, model: str) -> bool:
    blob = f"{make} {model}".lower()
    return any(marker in blob for marker in PHONE_MARKERS)


def classify(path: str, make: str = "", model: str = "") -> str:
    """Return a counted kind, 'jpeg-not-phone', 'generated', or 'other'."""
    kind = suffix_kind(path)
    if kind is None:
        return "other"
    if is_generated(path):
        return "generated"
    if kind == "jpeg":
        if is_phone_exif(make, model):
            return "phone-jpeg"
        return "jpeg-not-phone"
    if kind == "heic":
        return "heic"
    return kind


def within_tolerance(reference: bytes, candidate: bytes, mode: str) -> bool:
    """Compare rust bytes to the Python output. mode is 'exact' or 'shrink-cap'.

    'exact' is byte identity, including an under-cap shrink (the app copies).
    'shrink-cap' is the shrink test: same length, and the same container
    magic the tests already require. Pixel error is not a tolerance.
    """
    if mode == "exact":
        return reference == candidate
    if mode != "shrink-cap":
        raise ValueError(mode)
    if len(reference) != len(candidate) or not reference:
        return False
    if reference[:2] == b"\xff\xd8":
        return candidate[:2] == b"\xff\xd8"
    if reference[:4] == b"%PDF":
        return candidate[:4] == b"%PDF"
    if b"ftyp" in reference[:16]:
        return b"ftyp" in candidate[:16]
    return reference == candidate


def repository_blobs(repo: Path) -> list[tuple[str, str]]:
    """Every path reachable from this clone, with one blob id."""
    out = subprocess.check_output(
        ["git", "rev-list", "--all", "--objects"],
        cwd=repo,
        text=True,
    )
    found: dict[str, str] = {}
    for line in out.splitlines():
        sha, _, path = line.partition(" ")
        if path and path not in found:
            found[path] = sha
    return sorted(found.items())


def corpus_files(corpus: Path) -> list[tuple[str, Path]]:
    """Every file under the local corpus, as (corpus/relative/path, file)."""
    if not corpus.is_dir():
        return []
    found = []
    for file in corpus.rglob("*"):
        if file.is_file():
            rel = file.relative_to(corpus).as_posix()
            found.append((f"corpus/{rel}", file))
    return sorted(found)


def _read_source(repo: Path, row: dict) -> bytes:
    if "file" in row:
        return Path(row["file"]).read_bytes()
    return subprocess.check_output(["git", "cat-file", "blob", row["sha"]], cwd=repo)


def _exif_make_model(data: bytes) -> tuple[str, str]:
    import io

    from PIL import Image

    with Image.open(io.BytesIO(data)) as image:
        exif = image.getexif()
        make = str(exif.get(271, "") or "")
        model = str(exif.get(272, "") or "")
    return make, model


def collect(repo: Path, corpus: Path | None = None) -> dict:
    """Count real uploads. Generated paths are listed and left out of M."""
    counts = {kind: 0 for kind in COUNTED_KINDS}
    counts["jpeg-not-phone"] = 0
    counted: list[dict] = []
    generated: list[str] = []
    source = [(path, {"sha": sha}) for path, sha in repository_blobs(repo)]
    if corpus is not None:
        source += [(path, {"file": str(file)}) for path, file in corpus_files(corpus)]
    for path, origin in source:
        rough = suffix_kind(path)
        if rough is None:
            continue
        make = model = ""
        if rough == "jpeg" and not is_generated(path):
            try:
                make, model = _exif_make_model(_read_source(repo, origin))
            except Exception:
                make = model = ""
        kind = classify(path, make, model)
        if kind == "generated":
            generated.append(path)
            continue
        if kind == "other":
            continue
        if kind == "jpeg-not-phone":
            counts["jpeg-not-phone"] += 1
            continue
        counts[kind] += 1
        counted.append({"path": path, **origin, "kind": kind, "make": make, "model": model})
    counted.sort(key=lambda row: (row["kind"], row["path"]))
    return {
        "counts": counts,
        "counted": counted,
        "generated": sorted(generated),
        "m": sum(counts[kind] for kind in COUNTED_KINDS),
    }


def python_reference(src: Path, kind: str, out_dir: Path) -> tuple[bytes, str]:
    """Run the app. Returns (bytes, mode) where mode is exact or shrink-cap.

    Imported here so inventory tests do not load the web app.
    """
    from app.jobs import _convert_one, _pdf_to_text, _shrink_image, _shrink_video

    out_dir.mkdir(parents=True, exist_ok=True)
    if kind in {"phone-jpeg", "heic", "mp4"}:
        cap = SHRINK_CAP[kind]
        shrink = _shrink_video if kind == "mp4" else _shrink_image
        out = shrink(src, "medium", out_dir, target_byte=cap)
        copied = src.stat().st_size <= cap and out.stat().st_size == src.stat().st_size
        return out.read_bytes(), "exact" if copied else "shrink-cap"
    if kind == "docx":
        out = _convert_one(src, "txt", out_dir)
        return out.read_bytes(), "exact"
    if kind == "pdf":
        out = _pdf_to_text(src, out_dir)
        return out.read_bytes(), "exact"
    if kind == "xlsx":
        out = _convert_one(src, "csv", out_dir)
        return out.read_bytes(), "exact"
    raise ValueError(kind)


def _rust_output(src: Path, kind: str, out_dir: Path) -> bytes:
    oracle = os.environ.get("EXPORT_RUST_ORACLE", "").strip()
    if not oracle:
        raise RuntimeError(
            "no rust converter in this tree; host tools stay outside library code"
        )
    dest = out_dir / "rust-out"
    proc = subprocess.run(
        [oracle, str(src), str(dest), kind],
        capture_output=True,
        timeout=900,
    )
    if proc.returncode != 0:
        err = proc.stderr.decode("utf-8", "replace")[-400:]
        raise RuntimeError(err or f"rust oracle exit {proc.returncode}")
    if not dest.is_file():
        raise RuntimeError("rust oracle wrote no file")
    return dest.read_bytes()


def _probe_duration(data: bytes, directory: Path, name: str) -> float | None:
    path = directory / name
    path.write_bytes(data)
    proc = subprocess.run(
        ["ffprobe", "-v", "error", "-show_entries", "format=duration",
         "-of", "default=nw=1:nk=1", str(path)],
        capture_output=True,
        text=True,
        timeout=120,
    )
    try:
        return float(proc.stdout.strip())
    except ValueError:
        return None


def decodes_like(reference: bytes, candidate: bytes, kind: str, directory: Path) -> bool:
    """A shrink-cap match must be a real, decodable file, not padding behind a magic.

    Images decode to the same pixel size as the app's output; video probes to
    the same duration within half a second.
    """
    if kind == "mp4":
        want = _probe_duration(reference, directory, "reference.mp4")
        got = _probe_duration(candidate, directory, "candidate.mp4")
        return want is not None and got is not None and abs(want - got) <= 0.5
    import io

    from PIL import Image

    try:
        with Image.open(io.BytesIO(reference)) as want, Image.open(io.BytesIO(candidate)) as got:
            got.load()
            return want.size == got.size
    except Exception:
        return False


def _materialize(repo: Path, row: dict, directory: Path) -> Path:
    """Copy the source into the scratch dir; a corpus file is never written to."""
    dest = directory / Path(row["path"]).name
    if "file" in row:
        shutil.copyfile(row["file"], dest)
    else:
        dest.write_bytes(_read_source(repo, row))
    return dest


def score_file(repo: Path, row: dict) -> dict:
    kind = row["kind"]
    # Windows: a converter that still holds an output file open must not abort
    # the whole run during scratch cleanup.
    with tempfile.TemporaryDirectory(ignore_cleanup_errors=True) as tmp:
        root = Path(tmp)
        src = _materialize(repo, row, root)
        size = src.stat().st_size
        try:
            reference, mode = python_reference(src, kind, root / "py")
        except Exception as exc:
            return {
                "path": row["path"],
                "kind": kind,
                "bytes": size,
                "match": False,
                "reason": f"python reference failed: {exc}",
            }
        try:
            rust = _rust_output(src, kind, root)
        except Exception as exc:
            return {
                "path": row["path"],
                "kind": kind,
                "bytes": size,
                "match": False,
                "reason": str(exc),
            }
        ok = within_tolerance(reference, rust, mode)
        if ok and mode == "shrink-cap":
            ok = decodes_like(reference, rust, kind, root)
        return {
            "path": row["path"],
            "kind": kind,
            "bytes": size,
            "match": ok,
            "reason": "match" if ok else f"outside {mode} tolerance",
        }


def failure_list(file_rows: list[dict]) -> list[dict]:
    """Real-sized misses first, then counted files that did not match, largest first."""
    ordered = [dict(item) for item in REAL_SIZED_MISSES]
    misses = [row for row in file_rows if not row["match"]]
    misses.sort(key=lambda row: (-int(row.get("bytes") or 0), row["path"]))
    for row in misses:
        ordered.append(
            {
                "id": row["path"],
                "kind": row["kind"],
                "bytes": row["bytes"],
                "result": row["reason"],
                "in_m": True,
            }
        )
    return ordered


def score_repository(repo: Path, corpus: Path | None = None) -> dict:
    # python_reference imports `app`; run as `python scripts/outside_score.py`,
    # only scripts/ is on sys.path.
    if str(repo.resolve()) not in sys.path:
        sys.path.insert(0, str(repo.resolve()))
    inventory = collect(repo, corpus)
    rows = [score_file(repo, row) for row in inventory["counted"]]
    matched = sum(1 for row in rows if row["match"])
    missing = [kind for kind in COUNTED_KINDS if inventory["counts"][kind] == 0]
    reference_failed = sum(
        1 for row in rows if row["reason"].startswith("python reference failed")
    )
    return {
        "n": matched,
        "reference_failed": reference_failed,
        "m": inventory["m"],
        "tolerance": TOLERANCE,
        "mtime_slack_seconds": MTIME_SLACK_SECONDS,
        "mtime_slack_applies_to_bytes": False,
        "shrink_cap": SHRINK_CAP,
        "counts": inventory["counts"],
        "missing_kinds": missing,
        "generated_excluded": inventory["generated"],
        "counted": [row["path"] for row in inventory["counted"]],
        "failures": failure_list(rows),
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Score real uploads against the Python app")
    parser.add_argument("--repo", type=Path, default=Path(__file__).resolve().parents[1])
    parser.add_argument(
        "--corpus",
        type=Path,
        default=None,
        help="local real files (default: EXPORT_CORPUS, else <repo>/corpus)",
    )
    args = parser.parse_args()
    corpus = args.corpus or Path(os.environ.get("EXPORT_CORPUS") or args.repo / "corpus")
    report = score_repository(args.repo, corpus)
    print(json.dumps(report, indent=2))


if __name__ == "__main__":
    main()
