"""Tests for scripts/asset_fetch_common.py -- the SHA-256 verification/
download primitives shared by fetch_reranker_model.py and
fetch_coverage_templates.py. Asset-specific behavior (build_checks(), main())
is tested in each script's own test file instead.

No network calls in any test here -- fetch_one/curl is mocked or bypassed
entirely."""

import hashlib
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import asset_fetch_common as afc  # noqa: E402


class Sha256OfTests(unittest.TestCase):
    def test_matches_hashlib_directly(self):
        with tempfile.NamedTemporaryFile(delete=False) as f:
            f.write(b"hello world")
            path = f.name
        try:
            expected = "sha256:" + hashlib.sha256(b"hello world").hexdigest()
            self.assertEqual(afc.sha256_of(path), expected)
        finally:
            os.remove(path)


class CheckAssetManifestVersionTests(unittest.TestCase):
    def test_matching_tag_is_a_no_op(self):
        afc.check_asset_manifest_version(
            {"release_tag": "v0.22.0"}, "v0.22.0", "test_manifest.json"
        )  # no raise

    def test_mismatched_tag_raises(self):
        with self.assertRaises(RuntimeError):
            afc.check_asset_manifest_version(
                {"release_tag": "v0.22.0"}, "v0.23.0", "test_manifest.json"
            )

    def test_error_message_names_the_given_manifest_label(self):
        # manifest_label is what lets one function serve every asset-fetch
        # script's manifest -- confirm it actually appears in the message,
        # not just that some RuntimeError was raised.
        with self.assertRaises(RuntimeError) as ctx:
            afc.check_asset_manifest_version(
                {"release_tag": "v0.22.0"}, "v0.23.0", "coverage_templates_release_asset_manifest.json"
            )
        self.assertIn("coverage_templates_release_asset_manifest.json", str(ctx.exception))


class LoadJsonManifestTests(unittest.TestCase):
    def test_missing_file_raises_runtime_error(self):
        with self.assertRaises(RuntimeError):
            afc.load_json_manifest("/nonexistent/path/manifest.json", "test manifest")

    def test_invalid_json_raises_runtime_error(self):
        tmp = tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False, encoding="utf-8"
        )
        tmp.write("{not valid json")
        tmp.close()
        try:
            with self.assertRaises(RuntimeError):
                afc.load_json_manifest(tmp.name, "test manifest")
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
                afc.load_json_manifest(tmp.name, "test manifest"), {"key": "value"}
            )
        finally:
            os.remove(tmp.name)


class FetchAndVerifyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def test_success_returns_verified_path(self):
        content = b"a real model file"
        expected = "sha256:" + hashlib.sha256(content).hexdigest()
        checks = [("whole-file hash", expected, afc.sha256_of)]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(content)

        with mock.patch.object(afc, "fetch_one", side_effect=fake_fetch_one):
            path = afc.fetch_and_verify(
                "model.txt", checks, "kent-tokyo/renkin", "v0.23.0", self.tmp.name
            )
        self.assertEqual(path, os.path.join(self.tmp.name, "model.txt"))
        self.assertTrue(os.path.exists(path))

    def test_mismatch_raises_and_deletes_file(self):
        expected = "sha256:" + hashlib.sha256(b"the real content").hexdigest()
        checks = [("whole-file hash", expected, afc.sha256_of)]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(b"corrupted or wrong content")

        with mock.patch.object(afc, "fetch_one", side_effect=fake_fetch_one):
            with self.assertRaises(RuntimeError) as ctx:
                afc.fetch_and_verify(
                    "model.txt", checks, "kent-tokyo/renkin", "v0.23.0", self.tmp.name
                )
        self.assertIn("failed check", str(ctx.exception))
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp.name, "model.txt")),
            "a mismatched download must not be left on disk",
        )

    def test_second_check_failing_also_deletes_file(self):
        # First check passes, second doesn't -- must still clean up, not
        # just on the first check's failure path.
        content = b"looks right at first glance"
        expected_first = "sha256:" + hashlib.sha256(content).hexdigest()
        checks = [
            ("whole-file hash", expected_first, afc.sha256_of),
            ("a second, stricter check", "sha256:never-matches", lambda p: "sha256:nope"),
        ]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(content)

        with mock.patch.object(afc, "fetch_one", side_effect=fake_fetch_one):
            with self.assertRaises(RuntimeError):
                afc.fetch_and_verify(
                    "frequency_table.json",
                    checks,
                    "kent-tokyo/renkin",
                    "v0.23.0",
                    self.tmp.name,
                )
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp.name, "frequency_table.json"))
        )

    def test_non_runtime_error_from_verifier_still_deletes_file(self):
        # A verifier can fail in ways other than a clean SHA-256 mismatch or
        # a RuntimeError -- e.g. a corrupted download isn't valid UTF-8, and
        # decoding it raises UnicodeDecodeError. Cleanup must not depend on
        # the specific exception type, or a failed-verification file is left
        # on disk, contradicting this function's own "never returns a path
        # that failed any check" docstring promise.
        def broken_verifier(path):
            raise UnicodeDecodeError("utf-8", b"\xff", 0, 1, "invalid start byte")

        checks = [("a verifier that raises something unexpected", "sha256:x", broken_verifier)]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(b"\xff")

        with mock.patch.object(afc, "fetch_one", side_effect=fake_fetch_one):
            with self.assertRaises(RuntimeError):
                afc.fetch_and_verify(
                    "frequency_table.json",
                    checks,
                    "kent-tokyo/renkin",
                    "v0.23.0",
                    self.tmp.name,
                )
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp.name, "frequency_table.json")),
            "a verifier crash must still delete the unverified download",
        )


class FetchOneTests(unittest.TestCase):
    def test_failed_curl_leaves_no_partial_file(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            dest = os.path.join(tmp_dir, "model.txt")

            class FakeResult:
                returncode = 22  # curl's -f exit code for an HTTP error

            with mock.patch.object(afc.subprocess, "run", return_value=FakeResult()):
                with self.assertRaises(RuntimeError):
                    afc.fetch_one("https://example.invalid/model.txt", dest)
            self.assertFalse(os.path.exists(dest))
            self.assertFalse(os.path.exists(dest + ".part"))


if __name__ == "__main__":
    unittest.main()
