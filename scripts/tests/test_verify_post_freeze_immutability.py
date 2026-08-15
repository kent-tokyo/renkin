import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import verify_post_freeze_immutability as verify  # noqa: E402


class IsAllowedTests(unittest.TestCase):
    def test_exact_file_match(self):
        self.assertTrue(verify.is_allowed("CHANGELOG.md"))
        self.assertTrue(verify.is_allowed("README.md"))
        self.assertTrue(verify.is_allowed("docs/design/coverage-mode-v0.md"))

    def test_results_directory_glob(self):
        self.assertTrue(
            verify.is_allowed("data/coverage_mode_formal_test/results/arm_a_rows.jsonl")
        )
        self.assertTrue(
            verify.is_allowed(
                "data/coverage_mode_formal_test/results/nested/deep_file.json"
            )
        )

    def test_frozen_paths_not_allowed(self):
        self.assertFalse(verify.is_allowed("src/coverage_mode.rs"))
        self.assertFalse(verify.is_allowed("Cargo.toml"))
        self.assertFalse(verify.is_allowed("data/coverage_mode_formal_test/protocol.md"))
        self.assertFalse(verify.is_allowed("data/coverage_mode_formal_test/cohort_manifest.json"))
        self.assertFalse(verify.is_allowed("scripts/coverage_mode_formal_test_gate.py"))
        self.assertFalse(
            verify.is_allowed(
                "data/phase_a5_template_scaling/templates/coverage_templates_provenance_manifest.json"
            )
        )

    def test_similarly_named_but_different_path_not_allowed(self):
        # "results" is only allowed under the exact coverage_mode_formal_test
        # directory -- a similarly-named file elsewhere must not slip through.
        self.assertFalse(verify.is_allowed("data/some_other_results/rows.jsonl"))
        self.assertFalse(verify.is_allowed("docs/design/coverage-mode-v0-old.md"))


class GitIntegrationTests(unittest.TestCase):
    """Real git repo, real commits -- not mocked -- since the whole point
    of this script is to be trustworthy against actual git plumbing."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.repo = self.tmp.name
        self._git("init", "-q")
        self._git("config", "user.email", "test@example.com")
        self._git("config", "user.name", "test")

    def tearDown(self):
        self.tmp.cleanup()

    def _git(self, *args):
        result = subprocess.run(
            ["git", "-C", self.repo] + list(args), capture_output=True, text=True
        )
        if result.returncode != 0:
            raise RuntimeError(f"git {args} failed: {result.stderr}")
        return result.stdout.strip()

    def _write(self, relpath, content):
        path = os.path.join(self.repo, relpath)
        os.makedirs(os.path.dirname(path), exist_ok=True)
        with open(path, "w", encoding="utf-8") as f:
            f.write(content)

    def _run_check(self, rc_sha, ref="HEAD"):
        old_cwd = os.getcwd()
        os.chdir(self.repo)
        try:
            return verify.check(rc_sha, ref)
        finally:
            os.chdir(old_cwd)

    def test_no_changes_since_freeze_is_immutable(self):
        self._write("Cargo.toml", "version = 1\n")
        self._git("add", ".")
        self._git("commit", "-q", "-m", "freeze")
        rc_sha = self._git("rev-parse", "HEAD")
        result = self._run_check(rc_sha)
        self.assertTrue(result["immutable"])
        self.assertEqual(result["changed_files"], [])
        self.assertEqual(result["tree_hash_mismatches"], [])

    def test_allowed_result_file_added_stays_immutable(self):
        self._write("Cargo.toml", "version = 1\n")
        self._git("add", ".")
        self._git("commit", "-q", "-m", "freeze")
        rc_sha = self._git("rev-parse", "HEAD")

        self._write("data/coverage_mode_formal_test/results/arm_a_rows.jsonl", '{"a": 1}\n')
        self._git("add", ".")
        self._git("commit", "-q", "-m", "add results")

        result = self._run_check(rc_sha)
        self.assertTrue(result["immutable"])
        self.assertEqual(
            result["changed_files"],
            ["data/coverage_mode_formal_test/results/arm_a_rows.jsonl"],
        )
        self.assertEqual(result["disallowed_changes"], [])

    def test_src_change_after_freeze_is_a_violation(self):
        self._write("Cargo.toml", "version = 1\n")
        self._write("src/lib.rs", "// original\n")
        self._git("add", ".")
        self._git("commit", "-q", "-m", "freeze")
        rc_sha = self._git("rev-parse", "HEAD")

        self._write("src/lib.rs", "// changed after freeze\n")
        self._git("add", ".")
        self._git("commit", "-q", "-m", "sneaky change")

        result = self._run_check(rc_sha)
        self.assertFalse(result["immutable"])
        self.assertIn("src/lib.rs", result["disallowed_changes"])
        mismatch_paths = [m["path"] for m in result["tree_hash_mismatches"]]
        self.assertIn("src", mismatch_paths)

    def test_protocol_md_change_after_freeze_is_a_violation(self):
        self._write("Cargo.toml", "version = 1\n")
        self._write("data/coverage_mode_formal_test/protocol.md", "original protocol\n")
        self._git("add", ".")
        self._git("commit", "-q", "-m", "freeze")
        rc_sha = self._git("rev-parse", "HEAD")

        self._write(
            "data/coverage_mode_formal_test/protocol.md", "quietly loosened threshold\n"
        )
        self._git("add", ".")
        self._git("commit", "-q", "-m", "sneaky change")

        result = self._run_check(rc_sha)
        self.assertFalse(result["immutable"])
        self.assertIn(
            "data/coverage_mode_formal_test/protocol.md", result["disallowed_changes"]
        )

    def test_unallowlisted_new_file_is_a_violation(self):
        # A brand-new file in a path nobody allowlisted -- must fail
        # closed (default-deny), not silently pass because it's "new"
        # rather than a change to something frozen.
        self._write("Cargo.toml", "version = 1\n")
        self._git("add", ".")
        self._git("commit", "-q", "-m", "freeze")
        rc_sha = self._git("rev-parse", "HEAD")

        self._write("scripts/a_new_unreviewed_script.py", "print('surprise')\n")
        self._git("add", ".")
        self._git("commit", "-q", "-m", "unreviewed addition")

        result = self._run_check(rc_sha)
        self.assertFalse(result["immutable"])
        self.assertIn("scripts/a_new_unreviewed_script.py", result["disallowed_changes"])


if __name__ == "__main__":
    unittest.main()
