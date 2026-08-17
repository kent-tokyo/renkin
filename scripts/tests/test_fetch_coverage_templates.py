"""Tests for fetch_coverage_templates.py's asset-specific logic
(build_checks, main()'s CLI wiring). Shared verification/download
primitives are tested in test_asset_fetch_common.py. Mirrors
test_fetch_reranker_model.py's structure, simplified for a single asset
file instead of two.

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
import fetch_coverage_templates as fct  # noqa: E402


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
        self.assertEqual(verifiers, {afc.sha256_of})

    def test_missing_provenance_entry_raises_runtime_error(self):
        del self.provenance_manifest["asset_sha256"]
        with self.assertRaises(RuntimeError):
            fct.build_checks(self.provenance_manifest, self.asset_manifest)

    def test_missing_asset_manifest_entry_raises_runtime_error(self):
        del self.asset_manifest["assets"]["templates_2000.smi"]
        with self.assertRaises(RuntimeError):
            fct.build_checks(self.provenance_manifest, self.asset_manifest)


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

        # fetch_coverage_templates.main() -> asset_fetch_common.fetch_and_verify()
        # -> asset_fetch_common.fetch_one() -- patch where it's looked up.
        with mock.patch.object(afc, "fetch_one", side_effect=fake_fetch_one):
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


if __name__ == "__main__":
    unittest.main()
