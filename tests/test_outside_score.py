"""The outside score counts real uploads only, and it keeps the app's tolerance."""

import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "scripts"))

import outside_score as score  # noqa: E402


class ClassifyTest(unittest.TestCase):
    def test_fixture_jpeg_is_not_a_phone_upload(self):
        path = "crates/export-image/tests/fixtures/baseline_420.jpg"
        self.assertTrue(score.is_generated(path))
        self.assertEqual(score.classify(path, "Lavc", "Lavc"), "generated")

    def test_phone_exif_is_counted(self):
        path = "uploads/IMG_2041.jpg"
        self.assertEqual(score.classify(path, "Apple", "iPhone 15 Pro"), "phone-jpeg")
        self.assertTrue(score.is_phone_exif("samsung", "SM-S918B"))

    def test_jpeg_without_phone_exif_stays_out_of_m(self):
        self.assertEqual(score.classify("uploads/scan.jpg", "", ""), "jpeg-not-phone")
        self.assertFalse(score.is_phone_exif("SONY", "ILCE-7M4"))

    def test_documents_and_video(self):
        self.assertEqual(score.classify("papers/lease.docx"), "docx")
        self.assertEqual(score.classify("papers/lease.pdf"), "pdf")
        self.assertEqual(score.classify("sheets/budget.xlsx"), "xlsx")
        self.assertEqual(score.classify("clips/birthday.mp4"), "mp4")
        self.assertEqual(score.classify("camera/IMG.HEIC"), "heic")
        self.assertEqual(score.classify("tests/test_shrink.py"), "other")


class ToleranceTest(unittest.TestCase):
    def test_exact_bytes(self):
        self.assertTrue(score.within_tolerance(b"abc", b"abc", "exact"))
        self.assertFalse(score.within_tolerance(b"abc", b"abd", "exact"))
        self.assertFalse(score.within_tolerance(b"abc", b"ab", "exact"))

    def test_shrink_cap_is_size_and_container_not_pixels(self):
        reference = b"\xff\xd8" + b"pillow-payload" + b"\xff\xd9"
        same_size = b"\xff\xd8" + b"rust-payload!!" + b"\xff\xd9"
        self.assertEqual(len(reference), len(same_size))
        self.assertTrue(score.within_tolerance(reference, same_size, "shrink-cap"))
        shorter = b"\xff\xd8" + b"no"
        self.assertFalse(score.within_tolerance(reference, shorter, "shrink-cap"))
        self.assertFalse(score.within_tolerance(reference, same_size, "exact"))

    def test_mtime_slack_is_not_a_byte_tolerance(self):
        self.assertEqual(score.MTIME_SLACK_SECONDS, 2)
        self.assertIn("not applied to file bytes", score.TOLERANCE)
        self.assertIn("assertEqual", score.TOLERANCE)
        self.assertNotIn("mean absolute", score.TOLERANCE.lower())


class FailureOrderTest(unittest.TestCase):
    def test_real_sized_miss_leads_and_large_files_follow(self):
        rows = [
            {"path": "small.pdf", "kind": "pdf", "bytes": 100, "match": False, "reason": "miss"},
            {"path": "big.mp4", "kind": "mp4", "bytes": 50_000_000, "match": False, "reason": "miss"},
            {"path": "ok.jpg", "kind": "phone-jpeg", "bytes": 9, "match": True, "reason": "match"},
        ]
        failures = score.failure_list(rows)
        self.assertEqual(failures[0]["id"], "6144x4608-png-shrink")
        self.assertEqual(failures[0]["pixels"], "6144×4608")
        self.assertEqual(failures[0]["cap_bytes"], 8000)
        self.assertEqual(failures[0]["area_scales"], 6)
        self.assertEqual(failures[0]["result"], "OverBudget")
        self.assertIsNone(failures[0]["output_bytes"])
        self.assertFalse(failures[0]["in_m"])
        self.assertEqual(failures[1]["id"], "big.mp4")
        self.assertEqual(failures[2]["id"], "small.pdf")
        self.assertNotIn("ok.jpg", [item["id"] for item in failures])
        blob = json.dumps(failures)
        self.assertNotIn("p50", blob)
        self.assertNotIn("p99", blob)


class RepositoryScoreTest(unittest.TestCase):
    def test_this_repository(self):
        repo = Path(__file__).resolve().parents[1]
        report = score.score_repository(repo)
        self.assertEqual(report["n"], 0)
        self.assertEqual(report["m"], 0)
        for kind in score.COUNTED_KINDS:
            self.assertEqual(report["counts"][kind], 0, kind)
        self.assertEqual(report["missing_kinds"], list(score.COUNTED_KINDS))
        self.assertEqual(report["failures"][0]["id"], "6144x4608-png-shrink")
        self.assertFalse(report["mtime_slack_applies_to_bytes"])
        self.assertEqual(report["shrink_cap"], 20_000_000)
        for path in report["generated_excluded"]:
            self.assertTrue(score.is_generated(path), path)
        for path in report["counted"]:
            self.assertFalse(score.is_generated(path), path)
        self.assertNotIn("p50", json.dumps(report))


if __name__ == "__main__":
    unittest.main()
