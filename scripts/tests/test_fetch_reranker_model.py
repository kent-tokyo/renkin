"""Tests for fetch_reranker_model.py's verification logic (no network calls
-- fetch_one/curl is mocked or bypassed entirely in every test here)."""

import hashlib
import json
import os
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
import fetch_reranker_model as frm  # noqa: E402


class Sha256OfTests(unittest.TestCase):
    def test_matches_hashlib_directly(self):
        with tempfile.NamedTemporaryFile(delete=False) as f:
            f.write(b"hello world")
            path = f.name
        try:
            expected = "sha256:" + hashlib.sha256(b"hello world").hexdigest()
            self.assertEqual(frm.sha256_of(path), expected)
        finally:
            os.remove(path)


class EmbeddedJsonSha256Tests(unittest.TestCase):
    """frequency_table.json's freeze_manifest.json entry is a hash of the
    file's inner `table` data, not the whole file (see
    phase3e_export_frequency_table.py) -- these tests guard the regression
    this script actually hit while being written: naively whole-file-hashing
    frequency_table.json never matches that manifest entry, even for a
    perfectly valid, uncorrupted download. (The separate whole-file check
    lives in release_asset_manifest.json instead -- see
    BuildChecksTests below.)"""

    def write_json(self, obj):
        tmp = tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False, encoding="utf-8"
        )
        json.dump(obj, tmp)
        tmp.close()
        return tmp.name

    def test_reads_embedded_field_not_whole_file_hash(self):
        path = self.write_json(
            {"_purpose": "...", "sha256": "sha256:deadbeef", "entries": 3, "table": {}}
        )
        try:
            self.assertEqual(frm.embedded_json_sha256(path), "sha256:deadbeef")
            self.assertNotEqual(frm.embedded_json_sha256(path), frm.sha256_of(path))
        finally:
            os.remove(path)

    def test_missing_sha256_field_raises(self):
        path = self.write_json({"table": {}})
        try:
            with self.assertRaises(RuntimeError):
                frm.embedded_json_sha256(path)
        finally:
            os.remove(path)

    def test_invalid_json_raises_runtime_error(self):
        tmp = tempfile.NamedTemporaryFile(
            mode="w", suffix=".json", delete=False, encoding="utf-8"
        )
        tmp.write("{not valid json")
        tmp.close()
        try:
            with self.assertRaises(RuntimeError):
                frm.embedded_json_sha256(tmp.name)
        finally:
            os.remove(tmp.name)


class ManifestLookupTests(unittest.TestCase):
    def test_dotted_key(self):
        manifest = {"model_artifact": {"sha256": "sha256:abc123"}}
        self.assertEqual(
            frm.manifest_lookup(manifest, "model_artifact.sha256"), "sha256:abc123"
        )

    def test_missing_key_raises(self):
        with self.assertRaises(KeyError):
            frm.manifest_lookup({}, "model_artifact.sha256")


class BuildChecksTests(unittest.TestCase):
    """model.txt gets exactly one check (whole-file vs freeze_manifest.json).
    frequency_table.json gets two: whole-file vs release_asset_manifest.json
    (download authenticity) AND embedded inner-table hash vs
    freeze_manifest.json (content self-consistency) -- this is the "double
    check" the user explicitly asked for so "SHA-256 verified download" is
    unambiguously true for both files."""

    def setUp(self):
        self.freeze_manifest = {
            "model_artifact": {"sha256": "sha256:model-whole-file"},
            "feature_schema": {
                "template_frequency_table_sha256": "sha256:freq-inner-table"
            },
        }
        self.asset_manifest = {
            "release_tag": "v0.22.0",
            "assets": {
                "model.txt": {"sha256": "sha256:model-whole-file"},
                "frequency_table.json": {"sha256": "sha256:freq-whole-file"},
            },
        }

    def test_model_txt_has_exactly_one_check(self):
        checks = frm.build_checks(self.freeze_manifest, self.asset_manifest)
        self.assertEqual(len(checks["model.txt"]), 1)
        _, expected, verifier = checks["model.txt"][0]
        self.assertEqual(expected, "sha256:model-whole-file")
        self.assertIs(verifier, frm.sha256_of)

    def test_frequency_table_json_has_exactly_two_checks(self):
        checks = frm.build_checks(self.freeze_manifest, self.asset_manifest)
        self.assertEqual(len(checks["frequency_table.json"]), 2)
        expecteds = {expected for _, expected, _ in checks["frequency_table.json"]}
        self.assertEqual(expecteds, {"sha256:freq-whole-file", "sha256:freq-inner-table"})
        verifiers = {verifier for _, _, verifier in checks["frequency_table.json"]}
        self.assertEqual(verifiers, {frm.sha256_of, frm.embedded_json_sha256})


class CheckAssetManifestVersionTests(unittest.TestCase):
    def test_matching_tag_is_a_no_op(self):
        frm.check_asset_manifest_version({"release_tag": "v0.22.0"}, "v0.22.0")  # no raise

    def test_mismatched_tag_raises(self):
        with self.assertRaises(RuntimeError):
            frm.check_asset_manifest_version({"release_tag": "v0.22.0"}, "v0.23.0")


class VersionFromCargoTomlTests(unittest.TestCase):
    def write_cargo_toml(self, content):
        tmp = tempfile.NamedTemporaryFile(
            mode="w", suffix=".toml", delete=False, encoding="utf-8"
        )
        tmp.write(content)
        tmp.close()
        return tmp.name

    def test_reads_package_version(self):
        path = self.write_cargo_toml('[package]\nname = "renkin"\nversion = "0.22.0"\n')
        try:
            self.assertEqual(frm.version_from_cargo_toml(path), "v0.22.0")
        finally:
            os.remove(path)

    def test_does_not_pick_up_dependency_version(self):
        # A [package] version must win even when a later section (e.g.
        # [dependencies]) also has a `version = "..."` line -- this is the
        # actual shape of this project's real Cargo.toml.
        path = self.write_cargo_toml(
            '[package]\nname = "renkin"\nversion = "0.22.0"\n\n'
            '[dependencies]\nserde = { version = "1.0" }\n'
        )
        try:
            self.assertEqual(frm.version_from_cargo_toml(path), "v0.22.0")
        finally:
            os.remove(path)

    def test_no_package_section_returns_none(self):
        path = self.write_cargo_toml('[dependencies]\nserde = { version = "1.0" }\n')
        try:
            self.assertIsNone(frm.version_from_cargo_toml(path))
        finally:
            os.remove(path)


class FetchAndVerifyTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.tmp.cleanup()

    def test_success_returns_verified_path(self):
        content = b"a real model file"
        expected = "sha256:" + hashlib.sha256(content).hexdigest()
        checks = [("whole-file hash", expected, frm.sha256_of)]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(content)

        with mock.patch.object(frm, "fetch_one", side_effect=fake_fetch_one):
            path = frm.fetch_and_verify(
                "model.txt", checks, "kent-tokyo/renkin", "v0.23.0", self.tmp.name
            )
        self.assertEqual(path, os.path.join(self.tmp.name, "model.txt"))
        self.assertTrue(os.path.exists(path))

    def test_mismatch_raises_and_deletes_file(self):
        expected = "sha256:" + hashlib.sha256(b"the real content").hexdigest()
        checks = [("whole-file hash", expected, frm.sha256_of)]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(b"corrupted or wrong content")

        with mock.patch.object(frm, "fetch_one", side_effect=fake_fetch_one):
            with self.assertRaises(RuntimeError) as ctx:
                frm.fetch_and_verify(
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
            ("whole-file hash", expected_first, frm.sha256_of),
            ("a second, stricter check", "sha256:never-matches", lambda p: "sha256:nope"),
        ]

        def fake_fetch_one(url, dest_path):
            with open(dest_path, "wb") as f:
                f.write(content)

        with mock.patch.object(frm, "fetch_one", side_effect=fake_fetch_one):
            with self.assertRaises(RuntimeError):
                frm.fetch_and_verify(
                    "frequency_table.json",
                    checks,
                    "kent-tokyo/renkin",
                    "v0.23.0",
                    self.tmp.name,
                )
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp.name, "frequency_table.json"))
        )


class FetchOneTests(unittest.TestCase):
    def test_failed_curl_leaves_no_partial_file(self):
        with tempfile.TemporaryDirectory() as tmp_dir:
            dest = os.path.join(tmp_dir, "model.txt")

            class FakeResult:
                returncode = 22  # curl's -f exit code for an HTTP error

            with mock.patch.object(frm.subprocess, "run", return_value=FakeResult()):
                with self.assertRaises(RuntimeError):
                    frm.fetch_one("https://example.invalid/model.txt", dest)
            self.assertFalse(os.path.exists(dest))
            self.assertFalse(os.path.exists(dest + ".part"))


if __name__ == "__main__":
    unittest.main()
