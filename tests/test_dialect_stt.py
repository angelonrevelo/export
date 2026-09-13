import io
import os
import tempfile
import unittest
from pathlib import Path

from PIL import Image
from fastapi.testclient import TestClient

os.environ.setdefault("EXPORT_PASSWORD", "1234")

from app.jobs import _collect_transcript, _convert_one, _whisper_state
from app.language import (
    dialect_stt_model,
    is_ph_dialect,
    language_name,
    normalize_language_code,
    stt_model_id,
    use_language,
    whisper_language_arg,
)
from app.main import app
import app.translate as tr


class DialectCodeTest(unittest.TestCase):
    def test_not_mapped_to_tl(self):
        for code in ("ceb", "ilo", "hil", "war", "bcl"):
            self.assertEqual(normalize_language_code(code), code if code != "bik" else "bcl")
            self.assertTrue(is_ph_dialect(code))
            self.assertNotEqual(normalize_language_code(code), "tl")

    def test_bikol_alias(self):
        self.assertEqual(normalize_language_code("bik"), "bcl")
        self.assertTrue(is_ph_dialect("bik"))

    def test_bisaya_alias(self):
        self.assertEqual(normalize_language_code("bisaya"), "ceb")

    def test_filipino_still_tl(self):
        self.assertEqual(normalize_language_code("tl"), "tl")
        self.assertEqual(normalize_language_code("fil"), "tl")
        self.assertFalse(is_ph_dialect("tl"))
        self.assertFalse(is_ph_dialect("fil"))

    def test_names(self):
        self.assertEqual(language_name("ceb"), "Cebuano")
        self.assertEqual(language_name("ilo"), "Ilocano")
        self.assertEqual(language_name("hil"), "Hiligaynon")
        self.assertEqual(language_name("war"), "Waray")
        self.assertEqual(language_name("bcl"), "Bikol")


class SttRouterTest(unittest.TestCase):
    def test_cebuano_is_not_turbo_tl(self):
        with use_language("ceb"):
            self.assertEqual(stt_model_id(), dialect_stt_model())
            self.assertNotEqual(stt_model_id(), "large-v3-turbo")
            self.assertIsNone(whisper_language_arg(None))

    def test_ilocano_same_dialect_model(self):
        with use_language("ilo"):
            self.assertEqual(stt_model_id(), "rbcurzon/whisper-medium-ph")

    def test_tagalog_stays_turbo(self):
        with use_language("tl"):
            self.assertEqual(stt_model_id(), "large-v3-turbo")
            self.assertEqual(whisper_language_arg(None), "tl")

    def test_auto_stays_turbo(self):
        with use_language(None):
            self.assertEqual(stt_model_id(), "large-v3-turbo")


class MissingDialectModelTest(unittest.TestCase):
    def test_offline_missing_weights_errors(self):
        os.environ["EXPORT_OFFLINE"] = "1"
        empty = tempfile.mkdtemp()
        os.environ["EXPORT_WHISPER_PH_DIR"] = empty
        try:
            with use_language("ceb"):
                with tempfile.TemporaryDirectory() as d:
                    wav = Path(d) / "silence.wav"
                    wav.write_bytes(b"RIFF")  # not decoded — fail happens before ffmpeg
                    with self.assertRaises(RuntimeError) as cm:
                        _collect_transcript(wav, Path(d))
            msg = str(cm.exception)
            self.assertIn("missing dialect STT model", msg)
            self.assertIn("rbcurzon/whisper-medium-ph", msg)
        finally:
            os.environ.pop("EXPORT_OFFLINE", None)
            os.environ.pop("EXPORT_WHISPER_PH_DIR", None)


class CheapConvertStillSkipsWhisper(unittest.TestCase):
    def test_png_jpeg_and_yaml_json(self):
        self.assertIsNone(_whisper_state["model"])
        before = tr._translator
        with tempfile.TemporaryDirectory() as d:
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


class CapabilitiesDialectTest(unittest.TestCase):
    def test_ui_lists_dialects(self):
        client = TestClient(app)
        d = client.get("/api/capabilities").json()
        ui = {row["code"]: row["name"] for row in d["language"]["ui"]}
        self.assertEqual(ui["ceb"], "Cebuano")
        self.assertEqual(ui["ilo"], "Ilocano")
        self.assertEqual(ui["hil"], "Hiligaynon")
        self.assertEqual(ui["war"], "Waray")
        self.assertEqual(ui["bcl"], "Bikol")
        self.assertIn("tl", ui)
        self.assertEqual(d["language"]["whisper_model"], "large-v3-turbo")
        self.assertEqual(d["language"]["dialect_stt_model"], "rbcurzon/whisper-medium-ph")
        self.assertIn("ceb", d["language"]["dialect_code"])
