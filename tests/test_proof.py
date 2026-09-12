"""Live proofs on the shipped convert/translate path. No translator mock."""
import os
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

from app.jobs import _convert_one
from app.translate import translate_text


def _speech_clip(dir_path: Path, seconds: int = 3) -> Path:
    """2–3 s color video + spoken English via macOS say (or ffmpeg sine fallback)."""
    out = dir_path / "input_clip.mp4"
    audio = dir_path / "_speech.aiff"
    if shutil.which("say"):
        r = subprocess.run(
            ["say", "-o", str(audio), "Hello this is a short test for export transcription"],
            capture_output=True, text=True, timeout=30,
        )
        if r.returncode != 0:
            audio.unlink(missing_ok=True)
    if audio.exists():
        cmd = [
            "ffmpeg", "-y",
            "-f", "lavfi", "-i", f"color=c=blue:s=320x240:d={seconds}:r=10",
            "-i", str(audio),
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-b:a", "64k",
            "-shortest", str(out),
        ]
    else:
        cmd = [
            "ffmpeg", "-y",
            "-f", "lavfi", "-i", f"color=c=blue:s=320x240:d={seconds}:r=10",
            "-f", "lavfi", "-i", f"sine=frequency=440:duration={seconds}",
            "-c:v", "libx264", "-pix_fmt", "yuv420p",
            "-c:a", "aac", "-shortest", str(out),
        ]
    r = subprocess.run(cmd, capture_output=True, text=True, timeout=60)
    if r.returncode != 0:
        raise RuntimeError(f"ffmpeg clip failed: {r.stderr[-400:]}")
    return out


class LiveTranslateTest(unittest.TestCase):
    def setUp(self):
        # Must hit the real NLLB path, not the unit-test fake.
        import app.translate as tr
        self.assertIsNone(tr._translate_batch_impl)

    def test_english_to_spanish_differs(self):
        src = "The weather is nice today and the children are playing outside."
        out = translate_text(src, "en", "es")
        self.assertTrue(out.strip(), "translation was empty")
        self.assertNotEqual(out.strip(), src.strip())
        # Spanish should not be a byte-for-byte copy of the English sentence.
        self.assertNotIn("children are playing outside", out.lower())


@unittest.skipUnless(shutil.which("ffmpeg"), "ffmpeg missing")
class VideoProofTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        os.environ.pop("EXPORT_GROQ_API_KEY", None)
        cls.tmp = tempfile.TemporaryDirectory()
        cls.clip = _speech_clip(Path(cls.tmp.name), seconds=3)

    @classmethod
    def tearDownClass(cls):
        cls.tmp.cleanup()

    def test_frames_zip_png(self):
        out_dir = Path(self.tmp.name) / "frames"
        out_dir.mkdir(exist_ok=True)
        out = _convert_one(self.clip, "frames", out_dir)
        self.assertTrue(out.name.endswith("_frames.zip"), out.name)
        with zipfile.ZipFile(out) as zf:
            name = zf.namelist()
            png = [n for n in name if n.endswith(".png")]
            self.assertGreaterEqual(len(png), 1, name)
            data = zf.read(png[0])
            self.assertEqual(data[:8], b"\x89PNG\r\n\x1a\n")

    def test_kit_zip_transcript(self):
        out_dir = Path(self.tmp.name) / "kit"
        out_dir.mkdir(exist_ok=True)
        out = _convert_one(self.clip, "kit", out_dir)
        self.assertTrue(out.name.endswith("_kit.zip"), out.name)
        with zipfile.ZipFile(out) as zf:
            name = zf.namelist()
            self.assertTrue(any(n.endswith(".txt") for n in name), name)
            self.assertTrue(any(n.endswith(".srt") for n in name), name)
            self.assertTrue(any(n.endswith(".vtt") for n in name), name)
            self.assertTrue(any("notes" in n or n.endswith(".md") for n in name), name)

    def test_captioned_h264_mp4(self):
        out_dir = Path(self.tmp.name) / "captioned"
        out_dir.mkdir(exist_ok=True)
        out = _convert_one(self.clip, "captioned", out_dir)
        self.assertTrue(out.name.endswith("_captioned.mp4"), out.name)
        self.assertGreater(out.stat().st_size, 100)
        # ftyp box is the ISO-BMFF/mp4 signature after optional size.
        head = out.read_bytes()[:12]
        self.assertIn(b"ftyp", head)
