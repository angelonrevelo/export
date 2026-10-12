#!/usr/bin/env python3
"""Host-only byte compare. The export-docs crate never calls this.

LibreOffice (soffice) and pandoc are optional oracles. If neither is on
PATH, this prints a MISS line and exits 0. A compare of two files is a
strict byte compare and does not invoke either tool.
"""

import hashlib
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

MARKDOWN = "# Notes\n\nA **word** here.\n"


def miss(message: str) -> int:
    print(f"MISS: {message}")
    return 0


def compare(left: Path, right: Path) -> int:
    a = left.read_bytes()
    b = right.read_bytes()
    if a == b:
        print(f"match {len(a)} bytes")
        return 0
    print(f"differ left={len(a)} right={len(b)}")
    print(f"sha256 left={hashlib.sha256(a).hexdigest()}")
    print(f"sha256 right={hashlib.sha256(b).hexdigest()}")
    return 1


def pandoc_html() -> int:
    pandoc = shutil.which("pandoc")
    if pandoc is None:
        return miss("pandoc is not on PATH; host oracle skipped")
    with tempfile.TemporaryDirectory(prefix="export-docs-oracle-") as tmp:
        src = Path(tmp) / "note.md"
        dst = Path(tmp) / "note.html"
        src.write_text(MARKDOWN, encoding="utf-8")
        run = subprocess.run(
            [pandoc, str(src), "-t", "html", "-o", str(dst)],
            check=False,
            capture_output=True,
        )
        if run.returncode != 0:
            sys.stderr.write(run.stderr.decode("utf-8", "replace"))
            return miss("pandoc failed; host oracle skipped")
        data = dst.read_bytes()
    print(f"oracle pandoc html bytes {len(data)} sha256 {hashlib.sha256(data).hexdigest()}")
    return 0


def main(argv: list[str]) -> int:
    if len(argv) == 4 and argv[1] == "compare":
        return compare(Path(argv[2]), Path(argv[3]))
    soffice = shutil.which("soffice")
    pandoc = shutil.which("pandoc")
    if pandoc is None and soffice is None:
        return miss("pandoc and soffice are not on PATH; host oracle skipped")
    if soffice is not None and pandoc is None:
        print(f"oracle soffice present at {soffice}; no product path calls it")
        return 0
    return pandoc_html()


if __name__ == "__main__":
    sys.exit(main(sys.argv))
