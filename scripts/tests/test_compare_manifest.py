import os
import json
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_manifest as cm  # noqa: E402


class TestRedactHomeDir(unittest.TestCase):
    """A committed manifest previously leaked the local user's home
    directory -- both directly (macOS `ps -o comm` reports each process's
    full executable path) and via Claude Code's scratchpad temp-dir naming
    (which flattens the home path into the session directory name, e.g.
    /Users/name -> -Users-name-...). redact_home_dir must strip both forms
    without touching unrelated paths."""

    @patch("os.path.expanduser", return_value="/Users/exampleuser")
    def test_redacts_direct_home_path(self, _mock):
        text = "/Users/exampleuser/.rustup/toolchains/stable/bin/rustfmt"
        result = cm.redact_home_dir(text)
        self.assertNotIn("exampleuser", result)
        self.assertIn("<redacted-home>", result)

    @patch("os.path.expanduser", return_value="/Users/exampleuser")
    def test_redacts_scratchpad_flattened_form(self, _mock):
        text = (
            "/private/tmp/claude-501/-Users-exampleuser-Documents-repo/"
            "e0a01fc7-2c55/scratchpad/x.jsonl"
        )
        result = cm.redact_home_dir(text)
        self.assertNotIn("exampleuser", result)
        self.assertIn("<redacted-scratchpad-session>", result)

    @patch("os.path.expanduser", return_value="/Users/exampleuser")
    def test_leaves_unrelated_paths_unchanged(self, _mock):
        text = "data/templates_extracted_5000.smi"
        self.assertEqual(cm.redact_home_dir(text), text)

    @patch("os.path.expanduser", return_value="/Users/exampleuser")
    def test_empty_string_unchanged(self, _mock):
        self.assertEqual(cm.redact_home_dir(""), "")

    @patch("os.path.expanduser", return_value="~")
    def test_unresolved_home_is_a_noop_not_a_universal_match(self, _mock):
        # If $HOME can't be resolved, expanduser("~") returns "~" itself --
        # must not treat that as a wildcard that redacts everything.
        text = "some/normal/path.json"
        self.assertEqual(cm.redact_home_dir(text), text)

    @patch("os.path.expanduser", return_value="/Users/exampleuser")
    def test_command_line_args_are_redacted_in_start_manifest(self, _mock):
        manifest = cm.capture_start_manifest(
            tool="renkin",
            comparison_mode="shared_stock",
            ring_context_policy=None,
            command_line=[
                "scripts/compare_run.py",
                "--sample-list",
                "/Users/exampleuser/scratchpad/list.jsonl",
            ],
            repo_root=".",
            binary_path=None,
            docker_image=None,
            input_files={},
        )
        self.assertNotIn("exampleuser", " ".join(manifest["command_line"]))

    @patch("compare_manifest.sha256_file", return_value="sha256:test")
    def test_manifest_records_security_contract_and_budget(self, _mock):
        manifest = cm.capture_start_manifest(
            tool="renkin",
            comparison_mode="native",
            ring_context_policy="conservative",
            command_line=["renkin"],
            repo_root=".",
            binary_path=None,
            docker_image=None,
            input_files={"sample_list": "samples.jsonl"},
            resource_budget={"depth": 5, "beam_width": 100, "timeout_s": 30},
        )
        contract = manifest["security_contract"]
        self.assertEqual(contract["version"], cm.SECURITY_CONTRACT_VERSION)
        self.assertEqual(contract["resource_budget"]["timeout_s"], 30)
        self.assertTrue(contract["threat_cases"])
        self.assertTrue(all("security_case_id" in case for case in contract["threat_cases"]))
        cm.validate_security_contract(manifest)

    @patch("compare_manifest.sha256_file", return_value="sha256:test")
    def test_security_contract_rejects_missing_threat_case_field(self, _mock):
        manifest = cm.capture_start_manifest(
            tool="renkin",
            comparison_mode="native",
            ring_context_policy=None,
            command_line=["renkin"],
            repo_root=".",
            binary_path=None,
            docker_image=None,
            input_files={},
        )
        del manifest["security_contract"]["threat_cases"][0]["release_blocker"]
        with self.assertRaisesRegex(ValueError, "release_blocker"):
            cm.validate_security_contract(manifest)

    @patch("compare_manifest.sha256_file", return_value="sha256:test")
    def test_security_contract_rejects_duplicate_case_id(self, _mock):
        manifest = cm.capture_start_manifest(
            tool="renkin",
            comparison_mode="native",
            ring_context_policy=None,
            command_line=["renkin"],
            repo_root=".",
            binary_path=None,
            docker_image=None,
            input_files={},
        )
        cases = manifest["security_contract"]["threat_cases"]
        cases[1]["security_case_id"] = cases[0]["security_case_id"]
        with self.assertRaisesRegex(ValueError, "duplicate"):
            cm.validate_security_contract(manifest)

    @patch("compare_manifest.sha256_file", return_value="sha256:test")
    def test_security_contract_rejects_unknown_version(self, _mock):
        manifest = cm.capture_start_manifest(
            tool="renkin",
            comparison_mode="native",
            ring_context_policy=None,
            command_line=["renkin"],
            repo_root=".",
            binary_path=None,
            docker_image=None,
            input_files={},
        )
        manifest["security_contract"]["version"] = cm.SECURITY_CONTRACT_VERSION + 1
        with self.assertRaisesRegex(ValueError, "unsupported"):
            cm.validate_security_contract(manifest)

    @patch("compare_manifest.sha256_file", return_value="sha256:test")
    def test_load_and_validate_manifest_rejects_schema_drift_before_resume(self, _mock):
        manifest = cm.capture_start_manifest(
            tool="renkin",
            comparison_mode="native",
            ring_context_policy=None,
            command_line=["renkin"],
            repo_root=".",
            binary_path=None,
            docker_image=None,
            input_files={},
        )
        manifest["security_contract"]["version"] += 1
        with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
            json.dump(manifest, handle)
            handle.flush()
            with self.assertRaisesRegex(ValueError, "unsupported"):
                cm.load_and_validate_manifest(handle.name)

    @patch("compare_manifest.sha256_file", return_value="sha256:test")
    def test_load_and_validate_manifest_rejects_oversized_file(self, _mock):
        manifest = cm.capture_start_manifest(
            tool="renkin",
            comparison_mode="native",
            ring_context_policy=None,
            command_line=["renkin"],
            repo_root=".",
            binary_path=None,
            docker_image=None,
            input_files={},
        )
        with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
            json.dump(manifest, handle)
            handle.write(" " * 40)
            handle.flush()
            with patch.object(cm, "MAX_MANIFEST_BYTES", 32):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    cm.load_and_validate_manifest(handle.name)

    @patch("compare_manifest.sha256_file", return_value="sha256:test")
    def test_load_and_validate_manifest_rejects_symlink(self, _mock):
        manifest = cm.capture_start_manifest(
            tool="renkin",
            comparison_mode="native",
            ring_context_policy=None,
            command_line=["renkin"],
            repo_root=".",
            binary_path=None,
            docker_image=None,
            input_files={},
        )
        with tempfile.TemporaryDirectory() as directory:
            target = os.path.join(directory, "manifest.json")
            link = os.path.join(directory, "manifest-link.json")
            with open(target, "w", encoding="utf-8") as handle:
                json.dump(manifest, handle)
            try:
                os.symlink(target, link)
            except (NotImplementedError, OSError) as exc:
                self.skipTest(f"symlinks unavailable: {exc}")
            with self.assertRaisesRegex(ValueError, "symlink"):
                cm.load_and_validate_manifest(link)

    def test_manifest_input_hash_rejects_oversized_file(self):
        with tempfile.NamedTemporaryFile("wb") as handle:
            handle.write(b"x" * 40)
            handle.flush()
            with patch.object(cm, "MAX_MANIFEST_BYTES", 32):
                with self.assertRaisesRegex(ValueError, "exceeds"):
                    cm.sha256_file(handle.name)

    def test_json_depth_ignores_brackets_in_strings(self):
        self.assertEqual(cm._json_depth('{"note": "[[[not nesting]]]"}'), 1)

    def test_json_depth_rejects_deep_manifest(self):
        with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
            handle.write("{" * 4 + "\"security_contract\": {}" + "}" * 4)
            handle.flush()
            with patch.object(cm, "MAX_MANIFEST_JSON_DEPTH", 3):
                with self.assertRaisesRegex(ValueError, "JSON levels"):
                    cm.load_and_validate_manifest(handle.name)

    def test_json_structure_tokens_ignore_strings_and_bound_manifest(self):
        self.assertEqual(cm._json_structure_tokens('{"note": "[],:{}"}'), 3)
        with tempfile.NamedTemporaryFile("w", encoding="utf-8") as handle:
            handle.write('{"security_contract": {}}')
            handle.flush()
            with patch.object(cm, "MAX_MANIFEST_JSON_TOKENS", 3):
                with self.assertRaisesRegex(ValueError, "JSON tokens"):
                    cm.load_and_validate_manifest(handle.name)


if __name__ == "__main__":
    unittest.main()
