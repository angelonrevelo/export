import unittest

from app.language import use_language, use_translate_to
from app import translate as tr


class FloresMapTest(unittest.TestCase):
    def test_filipino(self):
        self.assertEqual(tr.flores_code("tl"), "tgl_Latn")
        self.assertEqual(tr.flores_code("fil"), "tgl_Latn")

    def test_chinese(self):
        self.assertEqual(tr.flores_code("zh"), "zho_Hans")
        self.assertEqual(tr.flores_code("zh-tw"), "zho_Hant")

    def test_unknown(self):
        self.assertIsNone(tr.flores_code("xx"))


class ChunkTest(unittest.TestCase):
    def test_short_passthrough(self):
        self.assertEqual(tr._split_chunk("hello"), ["hello"])

    def test_paragraph_split(self):
        a = "a" * 400
        b = "b" * 400
        piece = tr._split_chunk(f"{a}\n\n{b}")
        self.assertGreaterEqual(len(piece), 2)


class TranslateHookTest(unittest.TestCase):
    def setUp(self):
        self._prev = tr._translate_batch_impl

        def fake(text_list, src, tgt):
            return [f"[{src}->{tgt}]{t}" for t in text_list]

        tr._translate_batch_impl = fake

    def tearDown(self):
        tr._translate_batch_impl = self._prev

    def test_noop_without_target(self):
        with use_translate_to(None):
            self.assertEqual(tr.maybe_translate_text("bonjour"), "bonjour")

    def test_translates_when_pinned(self):
        with use_translate_to("en"):
            out = tr.maybe_translate_text("Hola amigos como estas hoy")
        self.assertIn("[", out)
        self.assertIn("Hola", out)

    def test_same_language_skips_model(self):
        with use_translate_to("en"):
            out = tr.maybe_translate_text(
                "This is a reasonably long English sentence used for language detection."
            )
        self.assertNotIn("[eng_Latn", out)

    def test_segment_batch(self):
        with use_translate_to("es"):
            out = tr.maybe_translate_segment([
                {"start": 0, "end": 1, "text": "Hello there friend"},
                {"start": 1, "end": 2, "text": "How are you today"},
            ])
        self.assertEqual(len(out), 2)
        self.assertIn("spa_Latn", out[0]["text"])
        self.assertEqual(out[0]["start"], 0)


if __name__ == "__main__":
    unittest.main()
