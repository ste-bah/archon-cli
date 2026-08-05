import hashlib
import sys
import tempfile
import unittest
from pathlib import Path
from types import MappingProxyType
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

    def test_pdf_catalogue_uses_lowercase_sha256_of_exact_canonical_utf8_path(self):
        nested = self.root / "nested"
        nested.mkdir()
        pdf = nested / "résumé.pdf"
        pdf.write_bytes(b"%PDF-1.4\n")
        canonical = pdf.resolve()
        expected_id = hashlib.sha256(str(canonical).encode("utf-8")).hexdigest()

        self.assertEqual(server.pdf_id_for_path(canonical), expected_id)
        self.assertEqual(expected_id, expected_id.lower())
        self.assertEqual(len(expected_id), 64)

    def test_pdf_catalogue_recurses_freezes_paths_and_preserves_duplicate_basenames(self):
        nested = self.root / "nested"
        duplicate = self.root / "other"
        nested.mkdir()
        duplicate.mkdir()
        nested_pdf = nested / "report.pdf"
        duplicate_pdf = duplicate / "report.pdf"
        nested_pdf.write_bytes(b"%PDF-1.4\n")
        duplicate_pdf.write_bytes(b"%PDF-1.4\n")
        (self.root / "notes.txt").write_text("not a PDF")

        catalogue = server.build_pdf_catalogue(self.root.resolve())
        expected_paths = {self.pdf.resolve(), nested_pdf.resolve(), duplicate_pdf.resolve()}
        expected_ids = {
            hashlib.sha256(str(path).encode("utf-8")).hexdigest(): path
            for path in expected_paths
        }

        self.assertIsInstance(catalogue, MappingProxyType)
        self.assertEqual(dict(catalogue), expected_ids)
        self.assertNotEqual(
            server.pdf_id_for_path(nested_pdf.resolve()),
            server.pdf_id_for_path(duplicate_pdf.resolve()),
        )
        with self.assertRaises(TypeError):
            catalogue["new-id"] = self.pdf.resolve()

    def test_pdf_catalogue_excludes_non_pdfs_and_symlinks_escaping_root(self):
        (self.root / "notes.txt").write_text("not a PDF")
        outside = self.root.parent / "outside.pdf"
        outside.write_bytes(b"%PDF-1.4\n")
        escape = self.root / "escape.pdf"
        try:
            escape.symlink_to(outside)
        except OSError:
            self.skipTest("symlinks are unavailable on this platform")

        catalogue = server.build_pdf_catalogue(self.root.resolve())

        self.assertEqual(set(catalogue.values()), {self.pdf.resolve()})
        self.assertNotIn(server.pdf_id_for_path(outside.resolve()), catalogue)

    def test_convert_accepts_only_catalogue_pdf_id_and_hides_unknown_id(self):
        pdf_id = hashlib.sha256(str(self.pdf.resolve()).encode("utf-8")).hexdigest()
        app = server.build_app("cpu", {}, server.build_pdf_catalogue(self.root.resolve()))

        with mock.patch.object(server, "run_marker") as run_marker:
            response = TestClient(app).post("/convert", json={"pdf_id": "not-a-real-id"})

        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.json(), {"error": "invalid pdf_id"})
        self.assertNotIn("not-a-real-id", response.text)
        self.assertNotIn(pdf_id, response.text)
        run_marker.assert_not_called()

    def test_convert_uses_trusted_catalogue_path_and_rejects_pdf_path_schema(self):
        pdf_id = hashlib.sha256(str(self.pdf.resolve()).encode("utf-8")).hexdigest()
        trusted_path = self.pdf.resolve()
        app = server.build_app("cpu", {}, MappingProxyType({pdf_id: trusted_path}))

        with mock.patch.object(server, "run_marker", return_value={"children": []}) as run_marker:
            client = TestClient(app)
            success = client.post("/convert", json={"pdf_id": pdf_id})
            legacy = client.post("/convert", json={"pdf_path": str(trusted_path)})

        self.assertEqual(success.status_code, 200)
        run_marker.assert_called_once_with(str(trusted_path), "cpu", None, artifact_dict={})
        self.assertEqual(legacy.status_code, 400)
        self.assertEqual(legacy.json(), {"error": "invalid request"})

    def test_convert_rejects_pdf_path_when_valid_pdf_id_is_also_present(self):
        pdf_id = hashlib.sha256(str(self.pdf.resolve()).encode("utf-8")).hexdigest()
        app = server.build_app("cpu", {}, MappingProxyType({pdf_id: self.pdf.resolve()}))

        with (
            mock.patch.object(server, "resolve_pdf_path", return_value=self.pdf.resolve()),
            mock.patch.object(server, "run_marker", return_value={"children": []}) as run_marker,
        ):
            response = TestClient(app).post(
                "/convert",
                json={"pdf_id": pdf_id, "pdf_path": str(self.pdf.resolve())},
            )

        self.assertEqual(
            (response.status_code, response.json(), run_marker.call_count),
            (400, {"error": "invalid request"}, 0),
        )

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

    def test_resolve_pdf_path_normalizes_before_path_construction(self):
        (self.root / "nested").mkdir()
        requested = str(self.root / "nested" / ".." / self.pdf.name)
        root = str(self.root.resolve())
        candidate = str(self.pdf.resolve())
        pdf_root = self.root.resolve()
        events = []
        original_path = server.Path

        def guarded_path(value, *args, **kwargs):
            events.append(("Path", value))
            if value == requested:
                raise AssertionError("raw requested path passed to Path")
            return original_path(value, *args, **kwargs)

        def record_realpath(value, *args, **kwargs):
            events.append(("realpath", value, args, kwargs))
            return original_realpath(value, *args, **kwargs)

        def record_commonpath(values):
            events.append(("commonpath", values))
            return original_commonpath(values)

        original_realpath = server.os.path.realpath
        original_commonpath = server.os.path.commonpath
        with (
            mock.patch.object(server.os.path, "realpath", side_effect=record_realpath),
            mock.patch.object(server.os.path, "commonpath", side_effect=record_commonpath),
            mock.patch.object(server, "Path", side_effect=guarded_path),
        ):
            resolved = server.resolve_pdf_path(pdf_root, requested)

        self.assertEqual(resolved, self.pdf.resolve())
        self.assertEqual(events[:3], [
            ("realpath", root, (), {"strict": True}),
            ("realpath", requested, (), {"strict": True}),
            ("commonpath", (root, candidate)),
        ])
        self.assertEqual(events[3], ("Path", candidate))

    def test_resolve_pdf_path_accepts_in_root_symlink_to_pdf_target(self):
        target = self.root / "target.pdf"
        target.write_bytes(b"%PDF-1.4\\n")
        alias = self.root / "alias.pdf"
        try:
            alias.symlink_to(target)
        except OSError:
            self.skipTest("symlinks are unavailable on this platform")

        self.assertEqual(
            server.resolve_pdf_path(self.root.resolve(), str(alias)),
            target.resolve(),
        )

    def test_resolve_pdf_path_rejects_missing_candidate_during_canonicalization(self):
        missing_pdf = self.root / "missing.pdf"
        pdf_root = self.root.resolve()
        realpath = server.os.path.realpath
        with (
            mock.patch.object(server.os.path, "realpath", wraps=realpath) as mocked_realpath,
            mock.patch.object(server.os.path, "commonpath", wraps=server.os.path.commonpath) as commonpath,
            mock.patch.object(server, "Path", wraps=server.Path) as path,
        ):
            with self.assertRaisesRegex(ValueError, r"^invalid pdf_path$"):
                server.resolve_pdf_path(pdf_root, str(missing_pdf))

        mocked_realpath.assert_any_call(str(missing_pdf), strict=True)
        commonpath.assert_not_called()
        path.assert_not_called()

    def test_resolve_pdf_path_rejects_commonpath_match_without_ancestor_identity(self):
        root = str(self.root.resolve())
        with (
            mock.patch.object(server.os.path, "commonpath", return_value=root),
            mock.patch.object(server.os.path, "samefile", return_value=False) as samefile,
        ):
            with self.assertRaisesRegex(ValueError, r"^invalid pdf_path$"):
                server.resolve_pdf_path(self.root.resolve(), str(self.pdf))

        self.assertGreater(samefile.call_count, 0)

    def test_resolve_pdf_path_accepts_commonpath_root_casing_with_ancestor_identity(self):
        root = str(self.root.resolve())
        with mock.patch.object(server.os.path, "commonpath", return_value=root):
            with mock.patch.object(server.os.path, "samefile", wraps=server.os.path.samefile) as samefile:
                self.assertEqual(
                    server.resolve_pdf_path(self.root.resolve(), str(self.pdf)),
                    self.pdf.resolve(),
                )

        samefile.assert_any_call(root, root)

    def test_resolve_pdf_path_maps_commonpath_value_error_to_invalid_path(self):
        with mock.patch.object(server.os.path, "commonpath", side_effect=ValueError):
            with self.assertRaisesRegex(ValueError, r"^invalid pdf_path$"):
                server.resolve_pdf_path(self.root.resolve(), str(self.pdf))

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

        alias_to_non_pdf = self.root / "alias.pdf"
        try:
            alias_to_non_pdf.symlink_to(notes)
        except OSError:
            self.skipTest("symlinks are unavailable on this platform")
        with self.assertRaisesRegex(ValueError, r"^invalid pdf_path$"):
            server.resolve_pdf_path(self.root.resolve(), str(alias_to_non_pdf))

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
        for page_range in ("not-a-range", "3-1", "0-1000000000"):
            with self.subTest(page_range=page_range):
                with mock.patch.object(server, "run_marker") as run_marker:
                    response = self.build_client().post(
                        "/convert",
                        json={"pdf_path": str(self.pdf), "page_range": page_range},
                    )

                self.assertEqual(response.status_code, 400)
                self.assertEqual(response.json(), {"error": "invalid page_range"})
                run_marker.assert_not_called()

    def test_convert_hides_request_validation_details(self):
        sensitive_path = "/private/customer-records.pdf"
        response = self.build_client().post(
            "/convert", json={"pdf_path": str(self.pdf), "page_range": {"path": sensitive_path}}
        )

        self.assertEqual(response.status_code, 400)
        self.assertEqual(response.json(), {"error": "invalid request"})
        self.assertNotIn(sensitive_path, response.text)
        self.assertNotIn("page_range", response.text)

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
