import os
import shutil
import tempfile
import unittest
import zipfile
from pathlib import Path

from PIL import Image

from app.jobs import (
    IMAGE_FORMATS,
    SPREADSHEET_FORMATS,
    _convert_one,
    _load_structured,
    _open_raster,
)


def _png(path: Path, size=(32, 24), color=(20, 80, 180)) -> Path:
    Image.new("RGB", size, color).save(path, "PNG")
    return path


class StructuredLoadTest(unittest.TestCase):
    def test_json(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "n.json"
            p.write_text('{"a": 1, "b": [2, 3]}', encoding="utf-8")
            self.assertEqual(_load_structured(p), {"a": 1, "b": [2, 3]})

    def test_yaml(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "n.yaml"
            p.write_text("name: export\ncount: 3\n", encoding="utf-8")
            self.assertEqual(_load_structured(p)["name"], "export")

    def test_toml(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "n.toml"
            p.write_text('name = "export"\ncount = 3\n', encoding="utf-8")
            self.assertEqual(_load_structured(p)["name"], "export")

    def test_xml(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "n.xml"
            p.write_text("<root><item id=\"1\">hi</item></root>", encoding="utf-8")
            data = _load_structured(p)
            self.assertIn("root", data)


class ImageConvertTest(unittest.TestCase):
    def test_png_to_jpeg(self):
        with tempfile.TemporaryDirectory() as d:
            src = _png(Path(d) / "input_photo.png")
            out = _convert_one(src, "jpeg", Path(d))
            self.assertTrue(out.exists())
            self.assertEqual(out.suffix, ".jpg")
            with Image.open(out) as img:
                self.assertEqual(img.format, "JPEG")

    def test_png_to_webp(self):
        with tempfile.TemporaryDirectory() as d:
            src = _png(Path(d) / "input_photo.png")
            out = _convert_one(src, "webp", Path(d))
            self.assertTrue(out.exists())
            with Image.open(out) as img:
                self.assertEqual(img.format, "WEBP")

    def test_jxl_listed_only_if_plugin(self):
        from app import jobs
        if jobs._HAS_JXL:
            self.assertIn("jxl", IMAGE_FORMATS)
        else:
            self.assertNotIn("jxl", IMAGE_FORMATS)

    def test_png_to_jxl(self):
        from app import jobs
        if not jobs._HAS_JXL:
            self.skipTest("pillow-jxl-plugin not installed")
        with tempfile.TemporaryDirectory() as d:
            src = _png(Path(d) / "input_photo.png")
            out = _convert_one(src, "jxl", Path(d))
            self.assertTrue(out.exists())
            self.assertEqual(out.suffix, ".jxl")
            self.assertGreater(out.stat().st_size, 0)
            magic = out.read_bytes()[:2]
            self.assertIn(magic, {b"\xff\x0a", b"\x00\x00"})

    def test_rawpy_imported(self):
        from app import jobs
        self.assertTrue(jobs._HAS_RAW, "rawpy extra should import after pip install .[media]")


@unittest.skipUnless(shutil.which("pandoc"), "pandoc missing")
class SpreadsheetConvertTest(unittest.TestCase):
    def test_yaml_to_json(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_cfg.yaml"
            src.write_text("- name: a\n  n: 1\n- name: b\n  n: 2\n", encoding="utf-8")
            out = _convert_one(src, "json", Path(d))
            text = out.read_text(encoding="utf-8")
            self.assertIn("name", text)
            self.assertTrue(text.strip().startswith("[") or text.strip().startswith("{"))

    def test_csv_to_yaml(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_t.csv"
            src.write_text("name,n\na,1\nb,2\n", encoding="utf-8")
            out = _convert_one(src, "yaml", Path(d))
            self.assertIn("name", out.read_text(encoding="utf-8"))
        self.assertIn("yaml", SPREADSHEET_FORMATS)


class ComicConvertTest(unittest.TestCase):
    def test_cbz_to_pdf(self):
        with tempfile.TemporaryDirectory() as d:
            page_a = _png(Path(d) / "p1.png", color=(200, 30, 30))
            page_b = _png(Path(d) / "p2.png", color=(30, 200, 30))
            cbz = Path(d) / "input_comic.cbz"
            with zipfile.ZipFile(cbz, "w") as zf:
                zf.write(page_a, arcname="p1.png")
                zf.write(page_b, arcname="p2.png")
            out = _convert_one(cbz, "pdf", Path(d))
            self.assertEqual(out.suffix, ".pdf")
            self.assertGreater(out.stat().st_size, 100)


class OfflineUrlTest(unittest.TestCase):
    def test_url_job_refuses_offline(self):
        from app.jobs import _require_network
        os.environ["EXPORT_OFFLINE"] = "1"
        try:
            with self.assertRaises(RuntimeError) as ctx:
                _require_network("URL extraction")
            self.assertIn("Offline", str(ctx.exception))
        finally:
            del os.environ["EXPORT_OFFLINE"]


class OpenRasterTest(unittest.TestCase):
    def test_png_roundtrip(self):
        with tempfile.TemporaryDirectory() as d:
            src = _png(Path(d) / "x.png")
            img = _open_raster(src)
            self.assertEqual(img.size, (32, 24))


if __name__ == "__main__":
    unittest.main()
