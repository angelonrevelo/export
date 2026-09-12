import io
import os
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from datetime import datetime
from pathlib import Path

from PIL import Image
from fastapi.testclient import TestClient

os.environ.setdefault("EXPORT_PASSWORD", "1234")

from app.main import app
from app.meta import (
    apply_date,
    apply_tag,
    apply_unlock,
    apply_decrypt,
    is_darwin,
    parse_when,
    read_tag,
    QUARANTINE_XATTR,
)


class ParseWhenTest(unittest.TestCase):
    def test_naive_local(self):
        ts = parse_when("2020-06-15T12:30")
        dt = datetime.fromtimestamp(ts)
        self.assertEqual(dt.year, 2020)
        self.assertEqual(dt.month, 6)
        self.assertEqual(dt.day, 15)
        self.assertEqual(dt.hour, 12)
        self.assertEqual(dt.minute, 30)


class DateOpTest(unittest.TestCase):
    def test_sets_mtime(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_photo.png"
            Image.new("RGB", (8, 8), (1, 2, 3)).save(src, "PNG")
            out = apply_date(src, Path(d), modified_at="2020-06-15T12:30")
            want = parse_when("2020-06-15T12:30")
            got = out.stat().st_mtime
            self.assertLess(abs(got - want), 2)
            if is_darwin() and hasattr(out.stat(), "st_birthtime"):
                self.assertLess(abs(out.stat().st_birthtime - want), 2)


class TagOpTest(unittest.TestCase):
    @unittest.skipUnless(is_darwin(), "Finder tags are macOS xattrs")
    def test_roundtrip_red_and_custom(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_photo.png"
            Image.new("RGB", (8, 8), (9, 9, 9)).save(src, "PNG")
            out = apply_tag(src, Path(d), "red, work")
            tag = read_tag(out)
            self.assertIn("Red", tag)
            self.assertIn("work", tag)


class UnlockOpTest(unittest.TestCase):
    @unittest.skipUnless(is_darwin(), "quarantine xattr is macOS")
    def test_strips_quarantine(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_photo.png"
            Image.new("RGB", (8, 8), (4, 5, 6)).save(src, "PNG")
            from app.meta import _xattr_list, _xattr_set
            _xattr_set(src, QUARANTINE_XATTR, b"0000;test")
            self.assertIn(QUARANTINE_XATTR, _xattr_list(src))
            out = apply_unlock(src, Path(d))
            self.assertNotIn(QUARANTINE_XATTR, _xattr_list(out))


class DecryptOpTest(unittest.TestCase):
    @unittest.skipUnless(shutil.which("zip"), "zip binary missing")
    def test_zip_password(self):
        with tempfile.TemporaryDirectory() as d:
            inner = Path(d) / "secret.txt"
            inner.write_text("hello-export\n", encoding="utf-8")
            zpath = Path(d) / "input_locked.zip"
            r = subprocess.run(
                ["zip", "-P", "s3cret", str(zpath), inner.name],
                cwd=d, capture_output=True, text=True, timeout=15,
            )
            self.assertEqual(r.returncode, 0, r.stderr)
            out = apply_decrypt(zpath, Path(d), "s3cret")
            self.assertIn(b"hello-export", out.read_bytes())

    def test_pdf_password(self):
        import pikepdf
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_locked.pdf"
            pdf = pikepdf.new()
            pdf.add_blank_page()
            pdf.save(src, encryption=pikepdf.Encryption(owner="pw", user="pw", R=6))
            out = apply_decrypt(src, Path(d), "pw")
            with pikepdf.open(out) as unlocked:
                self.assertGreaterEqual(len(unlocked.pages), 1)


class MetaApiTest(unittest.TestCase):
    def setUp(self):
        self.client = TestClient(app)

    def test_capabilities_meta(self):
        d = self.client.get("/api/capabilities").json()
        self.assertIn("date", d["operations"]["meta_op"])
        self.assertIn("tag", d["meta"]["op"])
        self.assertEqual(d["meta"]["is_finder_tag"], is_darwin())
        self.assertEqual(d["meta"]["is_reveal"], is_darwin())

    def test_date_via_api(self):
        buf = io.BytesIO()
        Image.new("RGB", (8, 8), (1, 2, 3)).save(buf, "PNG")
        buf.seek(0)
        r = self.client.post(
            "/api/meta",
            headers={"X-Password": "1234"},
            files={"file": ("photo.png", buf, "image/png")},
            data={"op": "date", "modified_at": "2019-01-02T03:04"},
        )
        self.assertEqual(r.status_code, 200, r.text)
        job_id = r.json()["job_id"]
        st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
        self.assertEqual(st["status"], "ready", st)
        dl = self.client.get(f"/api/files/{job_id}", params={"token": st["token"]})
        self.assertEqual(dl.status_code, 200)
        self.assertEqual(dl.content[:8], b"\x89PNG\r\n\x1a\n")

    def test_ui_has_meta_actions(self):
        html = (Path(__file__).resolve().parents[1] / "static" / "index.html").read_text()
        self.assertIn("/api/meta", html)
        self.assertIn("Finder tags", html)
        self.assertIn("show in Finder", html)
        self.assertIn('"date"', html)

    def test_unknown_op_400(self):
        buf = io.BytesIO()
        Image.new("RGB", (4, 4), (0, 0, 0)).save(buf, "PNG")
        buf.seek(0)
        r = self.client.post(
            "/api/meta",
            headers={"X-Password": "1234"},
            files={"file": ("a.png", buf, "image/png")},
            data={"op": "explode"},
        )
        self.assertEqual(r.status_code, 400)
