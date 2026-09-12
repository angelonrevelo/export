import tempfile
import time
import unittest
from pathlib import Path

from PIL import Image

from app.budget import pad_to_byte, predicted_byte, video_bit_rate
from app.jobs import _convert_one, _whisper_state
import app.translate as tr


class PredictedByteTest(unittest.TestCase):
    def test_over_cap_is_the_cap(self):
        self.assertEqual(predicted_byte(500_000, 20_000_000), 500_000)
        self.assertEqual(predicted_byte(50_000_000, 20_000_000), 20_000_000)

    def test_under_cap_does_not_grow(self):
        self.assertEqual(predicted_byte(100, 1000), 100)

    def test_api_twenty_mb_constant(self):
        self.assertEqual(predicted_byte(25_000_000, 20_000_000), 20_000_000)


class PadToByteTest(unittest.TestCase):
    def test_jpeg_exact(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a.jpg"
            Image.new("RGB", (40, 30), (10, 20, 30)).save(p, "JPEG", quality=85)
            cap = p.stat().st_size + 137
            pad_to_byte(p, cap)
            self.assertEqual(p.stat().st_size, cap)
            with Image.open(p) as img:
                self.assertEqual(img.format, "JPEG")
                self.assertEqual(img.size, (40, 30))

    def test_png_exact(self):
        with tempfile.TemporaryDirectory() as d:
            p = Path(d) / "a.png"
            Image.new("RGB", (24, 24), (200, 10, 10)).save(p, "PNG")
            cap = p.stat().st_size + 80
            pad_to_byte(p, cap)
            self.assertEqual(p.stat().st_size, cap)
            with Image.open(p) as img:
                self.assertEqual(img.format, "PNG")

    def test_video_bit_rate_positive(self):
        br = video_bit_rate(20_000_000, 60.0, 96_000)
        self.assertGreater(br, 8_000)


class CheapFirstTest(unittest.TestCase):
    def test_png_jpeg_and_yaml_skip_models(self):
        self.assertIsNone(_whisper_state["model"])
        self.assertIsNone(tr._translator)
        with tempfile.TemporaryDirectory() as d:
            png = Path(d) / "input_a.png"
            Image.new("RGB", (32, 24), (8, 16, 32)).save(png, "PNG")
            t0 = time.perf_counter()
            out = _convert_one(png, "jpeg", Path(d))
            png_ms = (time.perf_counter() - t0) * 1000
            self.assertGreater(out.stat().st_size, 0)
            yaml_path = Path(d) / "input_b.yaml"
            yaml_path.write_text("name: export\ncount: 2\n", encoding="utf-8")
            t1 = time.perf_counter()
            yout = _convert_one(yaml_path, "json", Path(d))
            yaml_ms = (time.perf_counter() - t1) * 1000
            self.assertIn(b"export", yout.read_bytes())
        print(f"CHEAP_PNG_JPEG_MS {png_ms:.1f}")
        print(f"CHEAP_YAML_JSON_MS {yaml_ms:.1f}")
        self.assertIsNone(_whisper_state["model"])
        self.assertIsNone(tr._translator)
        self.assertLess(png_ms, 5_000)
        self.assertLess(yaml_ms, 5_000)
