import os
import unittest

from app.language import (
    detect_language_code,
    detect_script,
    is_offline,
    is_whisper_english_only,
    language_name,
    normalize_language_code,
    pick_tess_lang,
    tess_code_for,
    use_language,
    whisper_language_arg,
)


class NormalizeTest(unittest.TestCase):
    def test_auto_is_none(self):
        self.assertIsNone(normalize_language_code("auto"))
        self.assertIsNone(normalize_language_code(""))
        self.assertIsNone(normalize_language_code(None))

    def test_filipino_aliases(self):
        self.assertEqual(normalize_language_code("tl"), "tl")
        self.assertEqual(normalize_language_code("fil"), "tl")
        self.assertEqual(normalize_language_code("FIL"), "tl")

    def test_chinese_bcp47(self):
        self.assertEqual(normalize_language_code("zh-TW"), "zh-tw")
        self.assertEqual(normalize_language_code("zh-CN"), "zh")
        self.assertEqual(normalize_language_code("zh-cn"), "zh")
        self.assertEqual(normalize_language_code("pt-BR"), "pt")

    def test_tess_name_roundtrip(self):
        self.assertEqual(normalize_language_code("chi_sim"), "zh")
        self.assertEqual(normalize_language_code("jpn"), "ja")


class ScriptDetectTest(unittest.TestCase):
    def test_latin(self):
        self.assertEqual(detect_script("The quick brown fox"), "latin")

    def test_han(self):
        self.assertEqual(detect_script("汉字测试页面"), "han")

    def test_hangul(self):
        self.assertEqual(detect_script("안녕하세요"), "hangul")

    def test_kana(self):
        self.assertEqual(detect_script("ひらがなカタカナ"), "kana")

    def test_arabic(self):
        self.assertEqual(detect_script("مرحبا بالعالم"), "arabic")

    def test_cyrillic(self):
        self.assertEqual(detect_script("Привет мир"), "cyrillic")

    def test_thai(self):
        self.assertEqual(detect_script("สวัสดีครับ"), "thai")

    def test_empty(self):
        self.assertEqual(detect_script(""), "other")


class LanguageDetectTest(unittest.TestCase):
    def test_forced_wins(self):
        with use_language("ja"):
            self.assertEqual(detect_language_code("hello this is english text"), "ja")

    def test_han_without_langdetect(self):
        with use_language(None):
            self.assertEqual(detect_language_code("这是一段中文内容用于识别"), "zh")

    def test_english_latin(self):
        with use_language(None):
            code = detect_language_code(
                "This is a reasonably long English sentence used for language detection."
            )
            self.assertEqual(code, "en")


class TessPickTest(unittest.TestCase):
    def test_export_ocr_lang_override(self):
        os.environ["EXPORT_OCR_LANG"] = "eng+fil"
        try:
            self.assertEqual(pick_tess_lang(), "eng+fil")
        finally:
            del os.environ["EXPORT_OCR_LANG"]

    def test_tess_code_for_known(self):
        self.assertEqual(tess_code_for("en"), "eng")
        self.assertEqual(tess_code_for("tl"), "fil")
        self.assertIn(tess_code_for("zh"), {"chi_sim", "chi_tra"})


class WhisperTest(unittest.TestCase):
    def test_distil_is_english_only(self):
        self.assertTrue(is_whisper_english_only("distil-large-v3"))
        self.assertTrue(is_whisper_english_only("small.en"))
        self.assertFalse(is_whisper_english_only("large-v3-turbo"))
        self.assertFalse(is_whisper_english_only("medium"))

    def test_whisper_arg(self):
        with use_language("tl"):
            self.assertEqual(whisper_language_arg(None), "tl")
        with use_language("zh-tw"):
            self.assertEqual(whisper_language_arg(None), "zh")


class OfflineTest(unittest.TestCase):
    def test_flag(self):
        os.environ["EXPORT_OFFLINE"] = "1"
        try:
            self.assertTrue(is_offline())
        finally:
            del os.environ["EXPORT_OFFLINE"]
        self.assertFalse(is_offline())

    def test_language_name(self):
        self.assertEqual(language_name("tl"), "Filipino")
        self.assertEqual(language_name("en"), "English")


if __name__ == "__main__":
    unittest.main()
