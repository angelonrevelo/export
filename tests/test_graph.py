import io
import os
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

from PIL import Image

os.environ.setdefault("EXPORT_PASSWORD", "1234")

from app.graph import reachable, shortest_path
from app.jobs import _convert_one, _whisper_state
import app.translate as tr
from app.main import app
from fastapi.testclient import TestClient


def _tiny_clip(path: Path, size="160x120") -> Path:
    r = subprocess.run(
        ["ffmpeg", "-y", "-f", "lavfi",
         "-i", f"color=c=blue:s={size}:d=1:r=10",
         "-c:v", "libx264", "-pix_fmt", "yuv420p", str(path)],
        capture_output=True, text=True, timeout=30,
    )
    if r.returncode != 0:
        raise RuntimeError(r.stderr[-400:])
    return path


class ReachableSetTest(unittest.TestCase):
    def test_video_png_is_reachable(self):
        r = reachable(".mp4")
        self.assertIn("png", r)
        self.assertIn("jpeg", r)
        self.assertIn("pdf", r)

    def test_video_woff2_is_not_reachable(self):
        r = reachable(".mp4")
        self.assertNotIn("woff2", r)
        self.assertNotIn("ttf", r)
        self.assertIsNone(shortest_path(".mp4", "woff2"))

    def test_image_jpeg_stays_direct(self):
        r = reachable(".png")
        self.assertIn("jpeg", r)
        self.assertEqual(shortest_path(".png", "jpeg"), ["jpeg"])

    def test_video_pdf_is_one_hop_not_frames(self):
        self.assertEqual(shortest_path(".mp4", "pdf"), ["pdf"])

    def test_video_jpeg_hops_through_png(self):
        self.assertEqual(shortest_path(".mp4", "jpeg"), ["png", "jpeg"])

    def test_video_png_is_direct(self):
        self.assertEqual(shortest_path(".mp4", "png"), ["png"])

    def test_gif_mp4_is_one_hop(self):
        r = reachable(".gif")
        self.assertIn("mp4", r)
        self.assertIn("mov", r)
        self.assertIn("webm", r)
        self.assertIn("mkv", r)
        self.assertIn("jpeg", r)
        self.assertEqual(shortest_path(".gif", "mp4"), ["mp4"])
        self.assertEqual(shortest_path(".gif", "jpeg"), ["jpeg"])


class HopConvertTest(unittest.TestCase):
    def test_png_jpeg_still_works(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_a.png"
            Image.new("RGB", (24, 16), (10, 20, 30)).save(src, "PNG")
            out = _convert_one(src, "jpeg", Path(d))
            self.assertEqual(out.read_bytes()[:2], b"\xff\xd8")

    def test_yaml_json_skips_models(self):
        self.assertIsNone(_whisper_state["model"])
        before = tr._translator
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_b.yaml"
            src.write_text("name: export\n", encoding="utf-8")
            out = _convert_one(src, "json", Path(d))
            self.assertIn(b"export", out.read_bytes())
        self.assertIsNone(_whisper_state["model"])
        self.assertIs(tr._translator, before)

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg missing")
    def test_video_to_png_still(self):
        with tempfile.TemporaryDirectory() as d:
            clip = _tiny_clip(Path(d) / "input_clip.mp4")
            out = _convert_one(clip, "png", Path(d))
            self.assertGreater(out.stat().st_size, 0)
            self.assertEqual(out.read_bytes()[:8], b"\x89PNG\r\n\x1a\n")
            with Image.open(out) as img:
                self.assertEqual(img.format, "PNG")
                self.assertEqual(img.size, (160, 120))

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg missing")
    def test_gif_to_mp4_one_hop(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "input_anim.gif"
            Image.new("RGB", (16, 12), (0, 0, 255)).save(src, "GIF")
            out = _convert_one(src, "mp4", Path(d))
            self.assertGreater(out.stat().st_size, 0)
            self.assertTrue(out.name.endswith(".mp4"))

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg missing")
    def test_video_to_jpeg_is_frame_not_transcript_page(self):
        with tempfile.TemporaryDirectory() as d:
            clip = _tiny_clip(Path(d) / "input_clip.mp4")
            out = _convert_one(clip, "jpeg", Path(d))
            self.assertEqual(out.read_bytes()[:2], b"\xff\xd8")
            with Image.open(out) as img:
                self.assertEqual(img.format, "JPEG")
                self.assertEqual(img.size, (160, 120))


class ReachableApiTest(unittest.TestCase):
    def setUp(self):
        self.client = TestClient(app)

    def test_reachable_mp4(self):
        r = self.client.get("/api/reachable", params={"ext": "mp4"})
        self.assertEqual(r.status_code, 200)
        body = r.json()
        self.assertIn("format", body)
        self.assertNotIn("formats", body)
        self.assertIn("png", body["format"])
        self.assertIn("jpeg", body["format"])
        self.assertIn("pdf", body["format"])
        self.assertNotIn("woff2", body["format"])

    def test_capabilities_video_png(self):
        d = self.client.get("/api/capabilities").json()
        self.assertIn("png", d["convert_outputs"]["video"])

    def test_reachable_gif_includes_mp4(self):
        r = self.client.get("/api/reachable", params={"ext": "gif"})
        self.assertEqual(r.status_code, 200)
        body = r.json()
        self.assertIn("mp4", body["format"])
        self.assertIn("jpeg", body["format"])

    @unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg missing")
    def test_upload_clip_to_png(self):
        with tempfile.TemporaryDirectory() as d:
            clip = _tiny_clip(Path(d) / "clip.mp4")
            data = clip.read_bytes()
        r = self.client.post(
            "/api/upload",
            headers={"X-Password": "1234"},
            files={"file": ("clip.mp4", io.BytesIO(data), "video/mp4")},
            data={"format": "png"},
        )
        self.assertEqual(r.status_code, 200, r.text)
        job_id = r.json()["job_id"]
        st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
        self.assertEqual(st["status"], "ready", st)
        dl = self.client.get(f"/api/files/{job_id}", params={"token": st["token"]})
        self.assertEqual(dl.status_code, 200)
        self.assertEqual(dl.content[:8], b"\x89PNG\r\n\x1a\n")
        with Image.open(io.BytesIO(dl.content)) as img:
            self.assertEqual(img.format, "PNG")
            self.assertGreater(img.size[0], 0)
