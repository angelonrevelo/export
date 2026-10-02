import os
import unittest

from fastapi.testclient import TestClient

os.environ.setdefault("EXPORT_PASSWORD", "1234")

from app.main import app  # noqa: E402

ORIGIN = "http://127.0.0.1:10100"


def _preflight(client, origin, pna=False):
    header = {
        "Origin": origin,
        "Access-Control-Request-Method": "POST",
        "Access-Control-Request-Headers": "x-password, content-type",
    }
    if pna:
        header["Access-Control-Request-Private-Network"] = "true"
    return client.options("/api/jobs", headers=header)


class CorsOffByDefaultTest(unittest.TestCase):
    def setUp(self):
        self.prev = os.environ.pop("EXPORT_CORS_ORIGIN", None)
        self.client = TestClient(app)

    def tearDown(self):
        if self.prev is not None:
            os.environ["EXPORT_CORS_ORIGIN"] = self.prev

    def test_unset_sends_no_cors(self):
        r = _preflight(self.client, ORIGIN, pna=True)
        self.assertNotIn("access-control-allow-origin", r.headers)
        self.assertNotIn("access-control-allow-private-network", r.headers)
        g = self.client.get("/api/capabilities", headers={"Origin": ORIGIN})
        self.assertEqual(g.status_code, 200)
        self.assertNotIn("access-control-allow-origin", g.headers)


class CorsAllowListTest(unittest.TestCase):
    def setUp(self):
        self.prev = os.environ.get("EXPORT_CORS_ORIGIN")
        os.environ["EXPORT_CORS_ORIGIN"] = f"https://cut.example, {ORIGIN}/"
        self.client = TestClient(app)

    def tearDown(self):
        if self.prev is None:
            os.environ.pop("EXPORT_CORS_ORIGIN", None)
        else:
            os.environ["EXPORT_CORS_ORIGIN"] = self.prev

    def test_preflight_allows_password_header(self):
        r = _preflight(self.client, ORIGIN)
        self.assertEqual(r.status_code, 204)
        self.assertEqual(r.headers["access-control-allow-origin"], ORIGIN)
        self.assertIn("X-Password", r.headers["access-control-allow-headers"])
        self.assertIn("Content-Type", r.headers["access-control-allow-headers"])
        self.assertIn("POST", r.headers["access-control-allow-methods"])
        self.assertNotIn("access-control-allow-private-network", r.headers)
        self.assertNotIn("access-control-allow-credentials", r.headers)

    def test_private_network_preflight(self):
        r = _preflight(self.client, "https://cut.example", pna=True)
        self.assertEqual(r.status_code, 204)
        self.assertEqual(r.headers["access-control-allow-private-network"], "true")

    def test_other_origin_refused(self):
        r = _preflight(self.client, "https://evil.example", pna=True)
        self.assertNotIn("access-control-allow-origin", r.headers)
        self.assertNotIn("access-control-allow-private-network", r.headers)

    def test_actual_response_carries_origin_even_on_401(self):
        g = self.client.get("/api/capabilities", headers={"Origin": ORIGIN})
        self.assertEqual(g.headers["access-control-allow-origin"], ORIGIN)
        self.assertIn("Origin", g.headers["vary"])
        bad = self.client.get("/api/jobs/nope", headers={"Origin": ORIGIN, "X-Password": "wrong"})
        self.assertEqual(bad.status_code, 401)
        self.assertEqual(bad.json()["detail"], "bad password")
        self.assertEqual(bad.headers["access-control-allow-origin"], ORIGIN)


if __name__ == "__main__":
    unittest.main()
