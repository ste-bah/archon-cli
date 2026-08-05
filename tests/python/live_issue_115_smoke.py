#!/usr/bin/env python3
"""Exercise Issue 115's Marker HTTP boundary over a live TCP listener."""

import json
import socket
import sys
import tempfile
import threading
import time
import unittest
import urllib.error
import urllib.request
from pathlib import Path
from unittest import mock

import uvicorn

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
import archon_marker_server as server  # noqa: E402


class Issue115LiveSmoke(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.corpus = Path(self.temp_dir.name) / "corpus"
        self.corpus.mkdir()
        self.inside_pdf = self.corpus / "inside.pdf"
        self.inside_pdf.write_bytes(b"%PDF-1.4\n")
        self.outside_pdf = self.corpus.parent / "outside.pdf"
        self.outside_pdf.write_bytes(b"%PDF-1.4\n")
        self.fail_pdf = self.corpus / "fail.pdf"
        self.fail_pdf.write_bytes(b"%PDF-1.4\n")
        self.inside_pdf_id = server.pdf_id_for_path(self.inside_pdf.resolve())
        self.fail_pdf_id = server.pdf_id_for_path(self.fail_pdf.resolve())
        self.port = self._preallocate_port()
        self.server = None
        self.server_exception = None
        self.thread = None

    def tearDown(self):
        try:
            if self.server is not None:
                self.server.should_exit = True
            if self.thread is not None:
                self.thread.join(timeout=5)
                self.assertFalse(self.thread.is_alive(), "Uvicorn did not stop")
                self._raise_server_exception()
        finally:
            self.temp_dir.cleanup()

    def test_live_tcp_security_boundaries(self):
        expected_tree = {"children": [], "source": "live-smoke"}

        def fake_converter(pdf_path, device, page_range, artifact_dict):
            if Path(pdf_path).name == "fail.pdf":
                raise RuntimeError("secret-live-error")
            self.assertEqual(Path(pdf_path), self.inside_pdf.resolve())
            self.assertEqual(device, "cpu")
            self.assertIsNone(page_range)
            self.assertEqual(artifact_dict, {})
            return expected_tree

        with (
            mock.patch.object(server, "run_marker", side_effect=fake_converter),
            mock.patch.object(server, "empty_cuda_cache"),
        ):
            self._start_server()
            self._wait_for_health()

            status, body = self._request("/convert", {"pdf_id": self.inside_pdf_id})
            self.assertEqual(status, 200)
            self.assertEqual(body, expected_tree)

            unknown_id = "0" * 64
            status, body = self._request("/convert", {"pdf_id": unknown_id})
            self.assertEqual(status, 400)
            self.assertEqual(body, {"error": "invalid pdf_id"})
            self.assertNotIn(unknown_id, json.dumps(body))
            self.assertNotIn(str(self.outside_pdf), json.dumps(body))

            status, body = self._request(
                "/convert", {"pdf_id": self.inside_pdf_id, "pdf_path": str(self.outside_pdf)}
            )
            self.assertEqual(status, 400)
            self.assertEqual(body, {"error": "invalid request"})
            self.assertNotIn(str(self.outside_pdf), json.dumps(body))

            status, body = self._request(
                "/convert", {"pdf_id": self.inside_pdf_id, "page_range": "0-1000000000"}
            )
            self.assertEqual(status, 400)
            self.assertEqual(body, {"error": "invalid page_range"})

            sensitive_path = "/private/customer-records.pdf"
            status, body = self._request(
                "/convert", {"pdf_id": self.inside_pdf_id, "page_range": {"path": sensitive_path}}
            )
            self.assertEqual(status, 400)
            self.assertEqual(body, {"error": "invalid request"})
            self.assertNotIn(sensitive_path, json.dumps(body))

            status, body = self._request("/convert", {"pdf_id": self.fail_pdf_id})
            self.assertEqual(status, 500)
            self.assertEqual(body, {"error": "conversion failed"})
            self.assertNotIn("secret-live-error", json.dumps(body))

            with self.assertRaises(ValueError):
                server.validate_bind_host("0.0.0.0", False)

    def _preallocate_port(self):
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
            listener.bind(("127.0.0.1", 0))
            return listener.getsockname()[1]

    def _start_server(self):
        app = server.build_app("cpu", {}, server.build_pdf_catalogue(self.corpus.resolve()))
        config = uvicorn.Config(app, host="127.0.0.1", port=self.port, log_level="error")
        self.server = uvicorn.Server(config)
        self.thread = threading.Thread(target=self._run_server, daemon=True)
        self.thread.start()

    def _run_server(self):
        try:
            self.server.run()
        except BaseException as exception:
            self.server_exception = exception

    def _raise_server_exception(self):
        if self.server_exception is not None:
            raise AssertionError("Uvicorn server thread failed") from self.server_exception

    def _wait_for_health(self):
        deadline = time.monotonic() + 5
        health_url = f"http://127.0.0.1:{self.port}/health"
        while time.monotonic() < deadline:
            try:
                with urllib.request.urlopen(health_url, timeout=0.2) as response:
                    self.assertEqual(response.status, 200)
                    self.assertEqual(json.load(response), {
                        "status": "ok",
                        "device": "cpu",
                        "models_loaded": True,
                    })
                    return
            except (OSError, urllib.error.URLError):
                if self.thread is not None and not self.thread.is_alive():
                    self._raise_server_exception()
                    self.fail("Uvicorn stopped before becoming healthy")
                time.sleep(0.05)
        self._raise_server_exception()
        self.fail("Timed out waiting for Uvicorn /health")

    def _request(self, path, payload):
        request = urllib.request.Request(
            f"http://127.0.0.1:{self.port}{path}",
            data=json.dumps(payload).encode(),
            headers={"Content-Type": "application/json"},
            method="POST",
        )
        try:
            with urllib.request.urlopen(request, timeout=2) as response:
                return response.status, json.load(response)
        except urllib.error.HTTPError as error:
            return error.code, json.load(error)


if __name__ == "__main__":
    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(Issue115LiveSmoke)
    )
    if not result.wasSuccessful():
        raise SystemExit(1)
    print(
        "ISSUE115_LIVE_SMOKE_PASS "
        "catalogue_id=accepted unknown_id=blocked request_schema=blocked "
        "page_validation=blocked conversion_disclosure=blocked bind=blocked"
    )
