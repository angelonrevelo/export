import io
import os
import unittest
from PIL import Image
from fastapi.testclient import TestClient

os.environ.setdefault("EXPORT_PASSWORD", "1234")

from app.main import app  # noqa: E402


class CapabilitiesTest(unittest.TestCase):
    def setUp(self):
        self.client = TestClient(app)

    def test_capabilities_shape(self):
        r = self.client.get("/api/capabilities")
        self.assertEqual(r.status_code, 200)
        d = r.json()
        self.assertIn("kit", d["url_extraction"]["url_outputs"])
        self.assertIn("notes", d["url_extraction"]["url_outputs"])
        self.assertIn("translate", d["url_extraction"]["url_outputs"])
        self.assertIn("translate", d["convert_outputs"]["pdf"])
        self.assertIn("language", d)
        self.assertIn("offline", d)
        self.assertTrue(d["language"]["is_whisper_multilingual"])
        self.assertEqual(d["language"]["whisper_model"], "large-v3-turbo")
        self.assertIn("yaml", d["convert_outputs"]["spreadsheet"])
        self.assertIn("kit", d["convert_outputs"]["video"])
        self.assertIn("captioned", d["convert_outputs"]["video"])
        self.assertFalse(d["offline"]["is_offline"])
        self.assertTrue(d["offline"]["is_url_enabled"])

    def test_png_to_jpeg_upload(self):
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
        # BackgroundTasks run inline under TestClient
        st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"})
        self.assertEqual(st.status_code, 200)
        body = st.json()
        self.assertEqual(body["status"], "ready", body)
        self.assertTrue(body["filename"].endswith(".jpg"))
        dl = self.client.get(f"/api/files/{job_id}", params={"pw": "1234"})
        self.assertEqual(dl.status_code, 200)
        self.assertGreater(len(dl.content), 50)

    def test_yaml_to_json_upload(self):
        r = self.client.post(
            "/api/upload",
            headers={"X-Password": "1234"},
            files={"file": ("cfg.yaml", b"name: export\ncount: 2\n", "text/yaml")},
            data={"format": "json"},
        )
        self.assertEqual(r.status_code, 200, r.text)
        job_id = r.json()["job_id"]
        st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
        self.assertEqual(st["status"], "ready", st)
        dl = self.client.get(f"/api/files/{job_id}", params={"pw": "1234"})
        self.assertIn(b"export", dl.content)

    def test_txt_translate_upload(self):
        from app import translate as tr
        prev = tr._translate_batch_impl
        tr._translate_batch_impl = lambda texts, s, t: [f"TR-{t}-{x}" for x in texts]
        try:
            r = self.client.post(
                "/api/upload",
                headers={"X-Password": "1234"},
                files={"file": ("note.txt", b"Hello there my friend this is a test sentence.\n", "text/plain")},
                data={"format": "txt", "translate_to": "tl"},
            )
            self.assertEqual(r.status_code, 200, r.text)
            job_id = r.json()["job_id"]
            st = self.client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
            self.assertEqual(st["status"], "ready", st)
            self.assertEqual(st.get("translate_to"), "tl")
            dl = self.client.get(f"/api/files/{job_id}", params={"pw": "1234"})
            self.assertIn(b"TR-tgl_Latn-", dl.content)
            self.assertIn(b"Hello there", dl.content)
        finally:
            tr._translate_batch_impl = prev

    def test_offline_url_rejected(self):
        os.environ["EXPORT_OFFLINE"] = "1"
        try:
            # is_offline reads env at call time
            r = self.client.post(
                "/api/jobs",
                headers={"X-Password": "1234", "Content-Type": "application/json"},
                json={"url": "https://example.com/watch?v=1", "format": "mp3"},
            )
            self.assertEqual(r.status_code, 503)
            self.assertIn("Offline", r.json()["detail"])
        finally:
            del os.environ["EXPORT_OFFLINE"]


if __name__ == "__main__":
    unittest.main()
