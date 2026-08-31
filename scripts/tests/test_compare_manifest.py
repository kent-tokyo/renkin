import os
import sys
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


if __name__ == "__main__":
    unittest.main()
