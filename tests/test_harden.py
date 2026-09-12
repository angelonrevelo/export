import io
import os
import unittest
from pathlib import Path

from PIL import Image
from fastapi.testclient import TestClient

os.environ.setdefault("EXPORT_PASSWORD", "1234")

from app.jobs import _convert_one, _whisper_state
from app.main import app
from app.ssrf import SsrfRefused, refuse_ssrf
import app.limit as limit
import app.translate as tr


class SsrfTest(unittest.TestCase):
    def test_loopback_refused(self):
        with self.assertRaises(SsrfRefused):
            refuse_ssrf("http://127.0.0.1/")
        with self.assertRaises(SsrfRefused):
            refuse_ssrf("http://127.0.0.1:6379/foo")

    def test_metadata_refused(self):
        with self.assertRaises(SsrfRefused):
            refuse_ssrf("http://169.254.169.254/")
        with self.assertRaises(SsrfRefused):
            refuse_ssrf("http://169.254.169.254/latest/meta-data/")

    def test_unique_local_v6_refused(self):
        with self.assertRaises(SsrfRefused):
            refuse_ssrf("http://[fd12:3456:789a::1]/")

    def test_jobs_api_refuses_before_extract(self):
        client = TestClient(app)
        for url in ("http://127.0.0.1/", "http://169.254.169.254/"):
            r = client.post(
                "/api/jobs",
                headers={"X-Password": "1234", "Content-Type": "application/json"},
                json={"url": url, "format": "mp3"},
            )
            self.assertEqual(r.status_code, 400, r.text)
            self.assertNotIn("job_id", r.json())
            self.assertIn("refused", r.json()["detail"])


class DownloadTokenTest(unittest.TestCase):
    def setUp(self):
        self.client = TestClient(app)

    def _jpeg_job(self):
        buf = io.BytesIO()
        Image.new("RGB", (24, 16), (10, 20, 30)).save(buf, "PNG")
        buf.seek(0)
        r = self.client.post(
            "/api/upload",
            headers={"X-Password": "1234"},
            files={"file": ("photo.png", buf, "image/png")},
            data={"format": "jpeg"},
        )
        self.assertEqual(r.status_code, 200, r.text)
        job_id = r.json()["job_id"]
        st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
        self.assertEqual(st["status"], "ready", st)
        self.assertTrue(st.get("token"))
        return job_id, st["token"]

    def test_pw_query_is_rejected(self):
        job_id, token = self._jpeg_job()
        bad = self.client.get(f"/api/files/{job_id}", params={"pw": "1234"})
        self.assertEqual(bad.status_code, 401)
        good = self.client.get(f"/api/files/{job_id}", params={"token": token})
        self.assertEqual(good.status_code, 200)
        self.assertEqual(good.content[:2], b"\xff\xd8")

    def test_wrong_token_rejected(self):
        job_id, _token = self._jpeg_job()
        r = self.client.get(f"/api/files/{job_id}", params={"token": "nope"})
        self.assertEqual(r.status_code, 401)


class ImageBombTest(unittest.TestCase):
    def test_max_pixels_is_sixty_four_million(self):
        self.assertEqual(Image.MAX_IMAGE_PIXELS, 64_000_000)


class RateLimitTest(unittest.TestCase):
    def setUp(self):
        self.client = TestClient(app)

    def tearDown(self):
        os.environ.pop("EXPORT_RATE_LIMIT_PER_MINUTE", None)
        limit.reset()

    def test_over_limit_is_429(self):
        os.environ["EXPORT_RATE_LIMIT_PER_MINUTE"] = "2"
        limit.reset()
        def post():
            buf = io.BytesIO()
            Image.new("RGB", (8, 8), (1, 2, 3)).save(buf, "PNG")
            buf.seek(0)
            return self.client.post(
                "/api/upload",
                headers={"X-Password": "1234"},
                files={"file": ("a.png", buf, "image/png")},
                data={"format": "jpeg"},
            )
        self.assertEqual(post().status_code, 200)
        self.assertEqual(post().status_code, 200)
        r = post()
        self.assertEqual(r.status_code, 429, r.text)
        self.assertIn("too many", r.json()["detail"])


class CheapPathStillWorks(unittest.TestCase):
    def test_png_jpeg_and_yaml_skip_models(self):
        self.assertIsNone(_whisper_state["model"])
        before = tr._translator
        with __import__("tempfile").TemporaryDirectory() as d:
            src = Path(d) / "a.png"
            Image.new("RGB", (24, 16), (10, 20, 30)).save(src, "PNG")
            out = _convert_one(src, "jpeg", Path(d))
            self.assertEqual(out.read_bytes()[:2], b"\xff\xd8")
            y = Path(d) / "b.yaml"
            y.write_text("name: export\n", encoding="utf-8")
            js = _convert_one(y, "json", Path(d))
            self.assertIn(b"export", js.read_bytes())
        self.assertIsNone(_whisper_state["model"])
        self.assertIs(tr._translator, before)


class UiSourceTest(unittest.TestCase):
    def test_grouped_optgroup_and_no_pw_query(self):
        html = (Path(__file__).resolve().parents[1] / "static" / "index.html").read_text()
        self.assertIn("fillGrouped", html)
        self.assertIn("createElement(\"optgroup\")", html)
        self.assertNotIn("?pw=", html)
        self.assertIn("?token=", html)


class GroupedReachableTest(unittest.TestCase):
    def test_video_png_in_image_kind(self):
        from app.graph import grouped_format, reachable
        fmt_list = reachable(".mp4")
        kind = grouped_format(fmt_list)
        self.assertIn("png", kind["image"])
        self.assertEqual(kind["video"][0], "mp4")
        self.assertNotIn("woff2", fmt_list)

    def test_gif_mp4_in_video_kind(self):
        from app.graph import grouped_format, reachable
        kind = grouped_format(reachable(".gif"))
        self.assertIn("mp4", kind["video"])
        self.assertIn("jpeg", kind["image"])

    def test_api_kind_key(self):
        client = TestClient(app)
        r = client.get("/api/reachable", params={"ext": "mp4"})
        self.assertEqual(r.status_code, 200)
        body = r.json()
        self.assertIn("kind", body)
        self.assertIn("png", body["kind"]["image"])
        self.assertIn("mp4", body["kind"]["video"])
