import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

from fastapi.testclient import TestClient

ROOT = Path(__file__).resolve().parents[2]
SCRIPTS = ROOT / "scripts"
sys.path.insert(0, str(SCRIPTS))
import archon_marker_server as server  # noqa: E402


class MarkerServerSecurityTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name) / "corpus"
        self.root.mkdir()
        self.pdf = self.root / "paper.pdf"
        self.pdf.write_bytes(b"%PDF-1.4\n")

    def tearDown(self):
        self.tmp.cleanup()

    def build_client(self):
        return TestClient(server.build_app("cpu", {}, self.root.resolve()))

    def test_canonical_pdf_root_returns_resolved_directory(self):
        self.assertEqual(
            server.canonical_pdf_root(str(self.root)),
            self.root.resolve(),
        )

    def test_canonical_pdf_root_rejects_missing_path_and_file(self):
        with self.assertRaises(ValueError):
            server.canonical_pdf_root(str(self.root / "missing"))
        with self.assertRaises(ValueError):
            server.canonical_pdf_root(str(self.pdf))

    def test_resolve_pdf_path_accepts_existing_pdf_inside_root(self):
        self.assertEqual(
            server.resolve_pdf_path(self.root.resolve(), str(self.pdf)),
            self.pdf.resolve(),
        )

    def test_resolve_pdf_path_rejects_unsafe_requests_without_disclosure(self):
        outside_pdf = self.root.parent / "outside.pdf"
        outside_pdf.write_bytes(b"%PDF-1.4\n")
        prefix_pdf = self.root.parent / "corpus-old" / "paper.pdf"
        prefix_pdf.parent.mkdir()
        prefix_pdf.write_bytes(b"%PDF-1.4\n")
        missing_pdf = self.root / "missing.pdf"
        notes = self.root / "notes.txt"
        notes.write_text("not a PDF")

        relative_traversal = str(self.root / ".." / outside_pdf.name)
        rejected_paths = [outside_pdf, prefix_pdf, relative_traversal, missing_pdf, notes]
        escape = self.root / "escape.pdf"
        try:
            escape.symlink_to(outside_pdf)
        except OSError:
            pass
        else:
            rejected_paths.append(escape)

        for requested in rejected_paths:
            with self.subTest(requested=requested):
                with self.assertRaisesRegex(ValueError, r"^invalid pdf_path$"):
                    server.resolve_pdf_path(self.root.resolve(), str(requested))

    def test_validate_bind_host_allows_loopback_hosts_without_opt_in(self):
        for host in ("127.0.0.1", "127.0.0.42", "::1", "localhost"):
            with self.subTest(host=host):
                server.validate_bind_host(host, allow_non_loopback=False)

    def test_validate_bind_host_requires_opt_in_for_non_loopback_hosts(self):
        for host in ("0.0.0.0", "::", "192.168.1.10", "marker.internal"):
            with self.subTest(host=host):
                with self.assertRaises(ValueError):
                    server.validate_bind_host(host, allow_non_loopback=False)
                server.validate_bind_host(host, allow_non_loopback=True)

    def test_convert_passes_canonical_pdf_path_to_marker(self):
        (self.root / "nested").mkdir()
        requested_path = str(self.root / "nested" / ".." / self.pdf.name)
        self.assertNotEqual(requested_path, str(self.pdf.resolve()))

        with mock.patch.object(server, "run_marker", return_value={"children": []}) as run_marker:
            response = self.build_client().post("/convert", json={"pdf_path": requested_path})

        self.assertEqual(response.status_code, 200)
        run_marker.assert_called_once_with(
            str(self.pdf.resolve()), "cpu", None, artifact_dict={}
        )

    def test_convert_rejects_invalid_paths_without_disclosing_them(self):
        outside_pdf = self.root.parent / "outside.pdf"
        outside_pdf.write_bytes(b"%PDF-1.4\n")
        missing_pdf = self.root / "missing.pdf"
        notes = self.root / "notes.txt"
        notes.write_text("not a PDF")

        for requested in (outside_pdf, missing_pdf, notes):
            with self.subTest(requested=requested):
                with mock.patch.object(server, "run_marker") as run_marker:
                    response = self.build_client().post(
                        "/convert", json={"pdf_path": str(requested)}
                    )

                self.assertEqual(response.status_code, 400)
                self.assertEqual(response.json(), {"error": "invalid pdf_path"})
                self.assertNotIn(str(requested), response.text)
                self.assertNotIn(requested.name, response.text)
                run_marker.assert_not_called()

    def test_convert_rejects_malformed_and_reversed_page_ranges(self):
        for page_range in ("not-a-range", "3-1"):
            with self.subTest(page_range=page_range):
                with mock.patch.object(server, "run_marker") as run_marker:
                    response = self.build_client().post(
                        "/convert",
                        json={"pdf_path": str(self.pdf), "page_range": page_range},
                    )

                self.assertEqual(response.status_code, 400)
                self.assertEqual(response.json(), {"error": "invalid page_range"})
                run_marker.assert_not_called()

    def test_convert_hides_non_oom_conversion_error_details(self):
        with mock.patch.object(
            server, "run_marker", side_effect=RuntimeError("sensitive conversion detail")
        ):
            response = self.build_client().post("/convert", json={"pdf_path": str(self.pdf)})

        self.assertEqual(response.status_code, 500)
        self.assertEqual(response.json(), {"error": "conversion failed"})
        self.assertNotIn("sensitive conversion detail", response.text)

    @mock.patch.object(server, "is_cuda_oom", return_value=True)
    def test_convert_hides_gpu_and_cpu_fallback_error_details(self, is_cuda_oom):
        with mock.patch.object(
            server,
            "run_marker",
            side_effect=[RuntimeError("sensitive GPU detail"), RuntimeError("sensitive CPU detail")],
        ) as run_marker:
            response = TestClient(server.build_app("cuda", {}, self.root.resolve())).post(
                "/convert", json={"pdf_path": str(self.pdf)}
            )

        self.assertEqual(response.status_code, 500)
        self.assertEqual(response.json(), {"error": "conversion failed"})
        self.assertNotIn("sensitive GPU detail", response.text)
        self.assertNotIn("sensitive CPU detail", response.text)
        self.assertEqual(
            run_marker.call_args_list,
            [
                mock.call(str(self.pdf.resolve()), "cuda", None, artifact_dict={}),
                mock.call(str(self.pdf.resolve()), "cpu", None, artifact_dict=None),
            ],
        )
        is_cuda_oom.assert_called_once()


if __name__ == "__main__":
    unittest.main()
