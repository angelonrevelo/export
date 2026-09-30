import json
import os
import tempfile
import unittest
import wave
from pathlib import Path
from types import SimpleNamespace
from unittest import mock

from fastapi.testclient import TestClient

os.environ.setdefault("EXPORT_PASSWORD", "1234")

import app.jobs as J  # noqa: E402
from app.language import default_whisper_model, use_language  # noqa: E402
from app.main import app  # noqa: E402


def _wav(path: Path, second: float = 1.0) -> Path:
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(16000)
        w.writeframes(b"\x00\x00" * int(16000 * second))
    return path


def _word(text, start, end, p):
    return SimpleNamespace(word=text, start=start, end=end, probability=p)


class FakeWhisper:
    """Stands in for faster_whisper.WhisperModel — records the transcribe kwargs."""

    def __init__(self):
        self.kw: dict = {}

    def transcribe(self, path, **kw):
        self.kw = kw
        is_word = kw.get("word_timestamps")
        seg = [
            SimpleNamespace(start=0.0, end=1.0, text=" Kumusta ka na",
                            words=[_word(" Kumusta", 0.0, 0.42, 0.91),
                                   _word(" ka", 0.42, 0.6, 0.8),
                                   _word(" na", 0.6, 1.0, 0.77)] if is_word else None),
            SimpleNamespace(start=1.2, end=2.0, text=" salamat",
                            words=[_word(" salamat", 1.2, 1.9, 0.99)] if is_word else None),
        ]
        return iter(seg), SimpleNamespace(duration=2.0, language=kw.get("language") or "tl")


class _FakeModelMixin:
    def install(self, name):
        self.fake = FakeWhisper()
        self.prev = dict(J._whisper_state)
        J._whisper_state["model"] = self.fake
        J._whisper_state["name"] = name

    def tearDown(self):
        J._whisper_state.clear()
        J._whisper_state.update(self.prev)


class WordsLocalTest(_FakeModelMixin, unittest.TestCase):
    def test_turbo_words_json(self):
        self.install(default_whisper_model())
        with tempfile.TemporaryDirectory() as d:
            src = _wav(Path(d) / "input_clip.wav")
            with use_language("tl"):
                out = J._convert_one(src, "words", Path(d))
            self.assertEqual(out.name, "clip_words.json")
            body = json.loads(out.read_text(encoding="utf-8"))
        self.assertTrue(self.fake.kw["word_timestamps"])
        self.assertEqual(self.fake.kw["language"], "tl")
        self.assertEqual(body["language"], "tl")
        self.assertEqual([w["text"] for w in body["word"]], ["Kumusta", "ka", "na", "salamat"])
        self.assertEqual(body["word"][0], {"text": "Kumusta", "start": 0.0, "end": 0.42, "confidence": 0.91})
        self.assertEqual(body["word"][3]["start"], 1.2)

    def test_dialect_model_gets_word_timestamps(self):
        ct2 = Path(tempfile.mkdtemp()) / "whisper-medium-ph-ct2"
        self.install(str(ct2))
        with mock.patch.object(J, "ensure_dialect_ct2", return_value=ct2):
            with tempfile.TemporaryDirectory() as d:
                src = _wav(Path(d) / "input_bisaya.wav")
                with use_language("ceb"):
                    out = J._convert_one(src, "words", Path(d))
                body = json.loads(out.read_text(encoding="utf-8"))
        self.assertTrue(self.fake.kw["word_timestamps"])
        self.assertNotIn("language", self.fake.kw)  # dialect: no turbo `tl` token
        self.assertEqual(body["language"], "ceb")
        self.assertEqual(len(body["word"]), 4)

    def test_srt_path_unchanged(self):
        self.install(default_whisper_model())
        with tempfile.TemporaryDirectory() as d:
            src = _wav(Path(d) / "input_clip.wav")
            out = J._convert_one(src, "srt", Path(d))
            self.assertIn("Kumusta ka na", out.read_text(encoding="utf-8"))
        self.assertNotIn("word_timestamps", self.fake.kw)


class WordsFallbackTest(unittest.TestCase):
    def test_segment_without_words_spreads_by_length(self):
        with tempfile.TemporaryDirectory() as d:
            out = J._write_words([{"start": 1.0, "end": 2.0, "text": " ab abcd"}], "x", Path(d), "en")
            body = json.loads(out.read_text(encoding="utf-8"))
        self.assertEqual(body["word"], [
            {"text": "ab", "start": 1.0, "end": 1.333, "confidence": None},
            {"text": "abcd", "start": 1.333, "end": 2.0, "confidence": None},
        ])


class WordsGroqTest(unittest.TestCase):
    def setUp(self):
        os.environ["EXPORT_GROQ_API_KEY"] = "gsk_test"
        os.environ.pop("EXPORT_OFFLINE", None)

    def tearDown(self):
        os.environ.pop("EXPORT_GROQ_API_KEY", None)

    def _run(self, payload):
        resp = mock.Mock(status_code=200)
        resp.json.return_value = payload
        with mock.patch("requests.post", return_value=resp) as post:
            with tempfile.TemporaryDirectory() as d:
                src = _wav(Path(d) / "input_talk.wav")
                out = J._convert_one(src, "words", Path(d))
                body = json.loads(out.read_text(encoding="utf-8"))
        return post, body

    def test_groq_word_granularity(self):
        post, body = self._run({
            "language": "english",
            "segments": [{"start": 0.0, "end": 1.0, "text": " hello there"}],
            "words": [{"word": "hello", "start": 0.1, "end": 0.4},
                      {"word": "there", "start": 0.5, "end": 0.9}],
        })
        sent = post.call_args.kwargs["data"]
        self.assertEqual(sent["timestamp_granularities[]"], ["word", "segment"])
        self.assertEqual(body["word"], [
            {"text": "hello", "start": 0.1, "end": 0.4, "confidence": None},
            {"text": "there", "start": 0.5, "end": 0.9, "confidence": None},
        ])

    def test_groq_without_words_falls_back(self):
        _, body = self._run({"segments": [{"start": 0.0, "end": 1.0, "text": "one two"}]})
        self.assertEqual([w["text"] for w in body["word"]], ["one", "two"])
        self.assertEqual(body["word"][1]["end"], 1.0)


class WordsApiTest(_FakeModelMixin, unittest.TestCase):
    def test_upload_words_and_capabilities(self):
        self.install(default_whisper_model())
        client = TestClient(app)
        with tempfile.TemporaryDirectory() as d:
            src = _wav(Path(d) / "clip.wav")
            r = client.post(
                "/api/upload",
                headers={"X-Password": "1234"},
                files={"file": ("clip.wav", src.read_bytes(), "audio/wav")},
                data={"format": "words", "language": "en"},
            )
        self.assertEqual(r.status_code, 200, r.text)
        job_id = r.json()["job_id"]
        st = client.get(f"/api/jobs/{job_id}", headers={"X-Password": "1234"}).json()
        self.assertEqual(st["status"], "ready", st)
        self.assertTrue(st["filename"].endswith("_words.json"))
        dl = client.get(f"/api/files/{job_id}", params={"token": st["token"]})
        body = dl.json()
        self.assertEqual(body["language"], "en")
        self.assertEqual(len(body["word"]), 4)

        cap = client.get("/api/capabilities").json()
        self.assertIn("words", cap["convert_outputs"]["audio"])
        self.assertIn("words", cap["convert_outputs"]["video"])
        self.assertIn("words", cap["url_extraction"]["url_outputs"])
        self.assertIn("words", client.get("/api/reachable", params={"ext": "mp4"}).json()["format"])


if __name__ == "__main__":
    unittest.main()
