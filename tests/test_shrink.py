import io
import os
import shutil
import subprocess
import tempfile
import time
import unittest
from pathlib import Path

from PIL import Image
from fastapi.testclient import TestClient

os.environ.setdefault("EXPORT_PASSWORD", "1234")

from app.jobs import (  # noqa: E402
    _shrink_image,
    _shrink_video,
    _whisper_state,
)
from app.main import app  # noqa: E402


def _noisy_png(path: Path, size=(320, 240)) -> Path:
    import random
    rng = random.Random(0)
    img = Image.new("RGB", size)
    pix = img.load()
    for y in range(size[1]):
        for x in range(size[0]):
            pix[x, y] = (rng.randint(0, 255), rng.randint(0, 255), rng.randint(0, 255))
    img.save(path, "PNG")
    return path


def _loud_clip(path: Path, seconds: int = 3) -> Path:
    r = subprocess.run(
        ["ffmpeg", "-y",
         "-f", "lavfi", "-i", f"testsrc=size=640x360:rate=24:duration={seconds}",
         "-f", "lavfi", "-i", f"sine=frequency=440:duration={seconds}",
         "-c:v", "libx264", "-pix_fmt", "yuv420p", "-b:v", "1500k",
         "-c:a", "aac", "-b:a", "128k", "-shortest", str(path)],
        capture_output=True, text=True, timeout=60,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr[-400:])
    return path


class ImageTargetByteTest(unittest.TestCase):
    def test_shrink_hits_cap_exactly(self):
        with tempfile.TemporaryDirectory() as d:
            src = _noisy_png(Path(d) / "input_photo.png")
            cap = max(8_000, src.stat().st_size // 4)
            self.assertGreater(src.stat().st_size, cap)
            out = _shrink_image(src, "medium", Path(d), target_byte=cap)
            self.assertEqual(out.stat().st_size, cap)
            with Image.open(out) as img:
                self.assertEqual(img.format, "JPEG")

    def test_resize_to_cap_without_max_dim(self):
        with tempfile.TemporaryDirectory() as d:
            src = _noisy_png(Path(d) / "input_photo.png")
            cap = max(8_000, src.stat().st_size // 4)
            self.assertGreater(src.stat().st_size, cap)
            out = _shrink_image(src, "resize", Path(d), target_byte=cap)
            self.assertEqual(out.stat().st_size, cap)
            with Image.open(out) as img:
                self.assertEqual(img.format, "JPEG")

    def test_under_cap_does_not_grow(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_tiny.png"
            Image.new("RGB", (8, 8), (1, 2, 3)).save(src, "PNG")
            before = src.stat().st_size
            cap = before + 50_000
            out = _shrink_image(src, "medium", Path(d), target_byte=cap)
            self.assertEqual(out.stat().st_size, before)
            self.assertLessEqual(out.stat().st_size, cap)


@unittest.skipUnless(shutil.which("ffmpeg") and shutil.which("ffprobe"), "ffmpeg/ffprobe missing")
class VideoTargetByteTest(unittest.TestCase):
    def test_shrink_hits_cap_and_plays(self):
        with tempfile.TemporaryDirectory() as d:
            src = _loud_clip(Path(d) / "input_clip.mp4")
            src_size = src.stat().st_size
            # 3 s of AAC-96k is ~36 KB; keep the cap above mux+audio floor.
            cap = max(100_000, src_size // 3)
            if src_size <= cap:
                cap = max(64_000, src_size - 20_000)
            self.assertGreater(src_size, cap)
            t0 = time.perf_counter()
            out = _shrink_video(src, "medium", Path(d), target_byte=cap)
            shrink_ms = (time.perf_counter() - t0) * 1000
            print(f"CHEAP_CLIP_SHRINK_MS {shrink_ms:.1f}")
            self.assertEqual(out.stat().st_size, cap)
            head = out.read_bytes()[:12]
            self.assertIn(b"ftyp", head)
            probe = subprocess.run(
                ["ffprobe", "-v", "error", "-select_streams", "v:0",
                 "-show_entries", "stream=codec_name,pix_fmt",
                 "-of", "default=nw=1", str(out)],
                capture_output=True, text=True, timeout=15,
            )
            self.assertEqual(probe.returncode, 0, probe.stderr)
            self.assertIn("codec_name=h264", probe.stdout)
            self.assertIn("pix_fmt=yuv420p", probe.stdout)
            audio = subprocess.run(
                ["ffprobe", "-v", "error", "-select_streams", "a:0",
                 "-show_entries", "stream=codec_name", "-of", "csv=p=0", str(out)],
                capture_output=True, text=True, timeout=15,
            )
            self.assertTrue((audio.stdout or "").strip(), "expected audio stream")


class ShrinkApiTest(unittest.TestCase):
    def setUp(self):
        self.client = TestClient(app)

    def test_accepts_twenty_mb_target(self):
        buf = io.BytesIO()
        Image.new("RGB", (16, 16), (9, 9, 9)).save(buf, "PNG")
        buf.seek(0)
        r = self.client.post(
            "/api/shrink",
            headers={"X-Password": "1234"},
            files={"file": ("tiny.png", buf, "image/png")},
            data={"level": "medium", "target_byte": "20000000"},
        )
        self.assertEqual(r.status_code, 200, r.text)
        job_id = r.json()["job_id"]
        st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
        self.assertEqual(st["status"], "ready", st)
        # Source is under 20 MB — must not grow to 20 MB.
        self.assertLess(st["output_size"], 20_000_000)
        self.assertEqual(st["predicted_byte"], st["output_size"])

    def test_shrink_cap_via_api(self):
        with tempfile.TemporaryDirectory() as d:
            raw_path = _noisy_png(Path(d) / "big.png", size=(280, 200))
            raw = raw_path.read_bytes()
        cap = max(8_000, len(raw) // 4)
        self.assertGreater(len(raw), cap)
        r = self.client.post(
            "/api/shrink",
            headers={"X-Password": "1234"},
            files={"file": ("big.png", io.BytesIO(raw), "image/png")},
            data={"level": "medium", "target_byte": str(cap)},
        )
        self.assertEqual(r.status_code, 200, r.text)
        job_id = r.json()["job_id"]
        st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
        self.assertEqual(st["status"], "ready", st)
        self.assertEqual(st["output_size"], cap)
        self.assertEqual(st["predicted_byte"], cap)
        dl = self.client.get(f"/api/files/{job_id}", params={"pw": "1234"})
        self.assertEqual(len(dl.content), cap)
        self.assertEqual(dl.content[:2], b"\xff\xd8")

    def test_resize_to_cap_via_api(self):
        with tempfile.TemporaryDirectory() as d:
            raw_path = _noisy_png(Path(d) / "photo.png", size=(280, 200))
            raw = raw_path.read_bytes()
        cap = max(8_000, len(raw) // 4)
        self.assertGreater(len(raw), cap)
        r = self.client.post(
            "/api/shrink",
            headers={"X-Password": "1234"},
            files={"file": ("photo.png", io.BytesIO(raw), "image/png")},
            data={"level": "resize", "target_byte": str(cap)},
        )
        self.assertEqual(r.status_code, 200, r.text)
        job_id = r.json()["job_id"]
        st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
        self.assertEqual(st["status"], "ready", st)
        self.assertNotEqual(st["status"], "error")
        self.assertEqual(st["output_size"], cap)
        self.assertEqual(st["predicted_byte"], cap)
        dl = self.client.get(f"/api/files/{job_id}", params={"pw": "1234"})
        self.assertEqual(len(dl.content), cap)
        self.assertEqual(dl.content[:2], b"\xff\xd8")

    def test_capabilities_advertise_twenty_mb(self):
        d = self.client.get("/api/capabilities").json()
        self.assertEqual(d["operations"]["shrink_target_byte"], 20_000_000)
