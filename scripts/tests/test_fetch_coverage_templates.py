"""Tests for fetch_coverage_templates.py's verification logic (no network
calls -- fetch_one/curl is mocked or bypassed entirely in every test here).
Mirrors test_fetch_reranker_model.py's structure, simplified for a single
asset file instead of two."""

import hashlib
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import fetch_coverage_templates as fct  # noqa: E402


class Sha256OfTests(unittest.TestCase):
    def test_matches_hashlib_directly(self):
        with tempfile.NamedTemporaryFile(delete=False) as f:
            f.write(b"hello world")
            path = f.name
        try:
            expected = "sha256:" + hashlib.sha256(b"hello world").hexdigest()
            self.assertEqual(fct.sha256_of(path), expected)
        finally:
            os.remove(path)


class BuildChecksTests(unittest.TestCase):
    """The asset gets exactly two checks: whole-file vs the release-asset
    manifest (download authenticity) AND whole-file vs the provenance
    manifest (derivation identity) -- both whole-file hashes here (unlike
    the reranker's frequency_table.json, which needs an embedded-field
    check for one of the two), but still two independent manifests."""

    def setUp(self):
        self.provenance_manifest = {"asset_sha256": "sha256:templates-whole-file"}
        self.asset_manifest = {
            "release_tag": "v0.24.0",
            "assets": {"templates_2000.smi": {"sha256": "sha256:templates-whole-file"}},
        }

    def test_has_exactly_two_checks(self):
        checks = fct.build_checks(self.provenance_manifest, self.asset_manifest)
        self.assertEqual(len(checks), 2)
        expecteds = {expected for _, expected, _ in checks}
        self.assertEqual(expecteds, {"sha256:templates-whole-file"})
        verifiers = {verifier for _, _, verifier in checks}
        self.assertEqual(verifiers, {fct.sha256_of})

    def test_missing_provenance_entry_raises_runtime_error(self):
        del self.provenance_manifest["asset_sha256"]
        with self.assertRaises(RuntimeError):
            fct.build_checks(self.provenance_manifest, self.asset_manifest)

    def test_missing_asset_manifest_entry_raises_runtime_error(self):
        del self.asset_manifest["assets"]["templates_2000.smi"]
        with self.assertRaises(RuntimeError):
            fct.build_checks(self.provenance_manifest, self.asset_manifest)


class CheckAssetManifestVersionTests(unittest.TestCase):
    def test_matching_tag_is_a_no_op(self):
        fct.check_asset_manifest_version({"release_tag": "v0.24.0"}, "v0.24.0")  # no raise

    def test_mismatched_tag_raises(self):
        with self.assertRaises(RuntimeError):
            fct.check_asset_manifest_version({"release_tag": "v0.24.0"}, "v0.25.0")


class LoadJsonManifestTests(unittest.TestCase):
    def test_missing_file_raises_runtime_error(self):
        with self.assertRaises(RuntimeError):
            fct.load_json_manifest("/nonexistent/path/manifest.json", "test manifest")

    def test_invalid_json_raises_runtime_error(self):
        tmp = tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False, encoding="utf-8"
        )
        tmp.write("{not valid json")
        tmp.close()
        try:
            with self.assertRaises(RuntimeError):
                fct.load_json_manifest(tmp.name, "test manifest")
        finally:
            os.remove(tmp.name)

    def test_valid_file_returns_parsed_json(self):
        tmp = tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False, encoding="utf-8"
        )
        json.dump({"key": "value"}, tmp)
        tmp.close()
        try:
            self.assertEqual(
                fct.load_json_manifest(tmp.name, "test manifest"), {"key": "value"}
            )
        finally:
            os.remove(tmp.name)


class MainDefaultVersionTests(unittest.TestCase):
    """--version must default to the release-asset manifest's release_tag,
    not to Cargo.toml's crate version -- same regression class
    fetch_reranker_model.py's own review caught for that script."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.provenance_manifest_path = os.path.join(
            self.tmp.name, "coverage_templates_provenance_manifest.json"
        )
        self.asset_manifest_path = os.path.join(
            self.tmp.name, "coverage_templates_release_asset_manifest.json"
        )

        self.templates_bytes = b"a fake templates file\n"
        templates_sha256 = "sha256:" + hashlib.sha256(self.templates_bytes).hexdigest()

        with open(self.provenance_manifest_path, "w", encoding="utf-8") as f:
            json.dump({"asset_sha256": templates_sha256}, f)
        with open(self.asset_manifest_path, "w", encoding="utf-8") as f:
            json.dump(
                {
                    "release_tag": "v0.24.0",  # deliberately not derived from Cargo.toml
                    "assets": {"templates_2000.smi": {"sha256": templates_sha256}},
                },
                f,
            )

    def tearDown(self):
        self.tmp.cleanup()

    def test_zero_arg_invocation_fetches_the_asset_manifests_release_tag(self):
        requested_urls = []

        def fake_fetch_one(url, dest_path):
            requested_urls.append(url)
            with open(dest_path, "wb") as out:
                out.write(self.templates_bytes)

        with mock.patch.object(fct, "fetch_one", side_effect=fake_fetch_one):
            fct.main(
                [
                    "--provenance-manifest",
                    self.provenance_manifest_path,
                    "--asset-manifest",
                    self.asset_manifest_path,
                    "--output-dir",
                    self.tmp.name,
                ]
            )

        self.assertEqual(len(requested_urls), 1)
        self.assertIn(
            "/releases/download/v0.24.0/",
            requested_urls[0],
            "the zero-arg invocation must use the release-asset manifest's own "
            "release_tag, not Cargo.toml's crate version",
        )


class FetchAndVerifyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def test_success_returns_verified_path(self):
        content = b"a real templates file"
        expected = "sha256:" + hashlib.sha256(content).hexdigest()
        checks = [("whole-file hash", expected, fct.sha256_of)]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(content)

        with mock.patch.object(fct, "fetch_one", side_effect=fake_fetch_one):
            path = fct.fetch_and_verify(
                "templates_2000.smi", checks, "kent-tokyo/renkin", "v0.24.0", self.tmp.name
            )
        self.assertEqual(path, os.path.join(self.tmp.name, "templates_2000.smi"))
        self.assertTrue(os.path.exists(path))

    def test_mismatch_raises_and_deletes_file(self):
        expected = "sha256:" + hashlib.sha256(b"the real content").hexdigest()
        checks = [("whole-file hash", expected, fct.sha256_of)]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(b"corrupted or wrong content")

        with mock.patch.object(fct, "fetch_one", side_effect=fake_fetch_one):
            with self.assertRaises(RuntimeError) as ctx:
                fct.fetch_and_verify(
                    "templates_2000.smi",
                    checks,
                    "kent-tokyo/renkin",
                    "v0.24.0",
                    self.tmp.name,
                )
        self.assertIn("failed check", str(ctx.exception))
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp.name, "templates_2000.smi")),
            "a mismatched download must not be left on disk",
        )

    def test_second_check_failing_also_deletes_file(self):
        content = b"looks right at first glance"
        expected_first = "sha256:" + hashlib.sha256(content).hexdigest()
        checks = [
            ("whole-file hash (asset manifest)", expected_first, fct.sha256_of),
            ("whole-file hash (provenance manifest)", "sha256:never-matches", fct.sha256_of),
        ]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(content)

        with mock.patch.object(fct, "fetch_one", side_effect=fake_fetch_one):
            with self.assertRaises(RuntimeError):
                fct.fetch_and_verify(
                    "templates_2000.smi",
                    checks,
                    "kent-tokyo/renkin",
                    "v0.24.0",
                    self.tmp.name,
                )
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp.name, "templates_2000.smi"))
        )


class FetchOneTests(unittest.TestCase):
    def test_failed_curl_leaves_no_partial_file(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            dest = os.path.join(tmp_dir, "templates_2000.smi")

            class FakeResult:
                returncode = 22  # curl's -f exit code for an HTTP error

            with mock.patch.object(fct.subprocess, "run", return_value=FakeResult()):
                with self.assertRaises(RuntimeError):
                    fct.fetch_one("https://example.invalid/templates_2000.smi", dest)
            self.assertFalse(os.path.exists(dest))
            self.assertFalse(os.path.exists(dest + ".part"))


if __name__ == "__main__":
    unittest.main()
