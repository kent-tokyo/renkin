"""Tests for `renkin.find_routes`'s coverage-mode kwargs (Phase 41.18B),
against the actual **installed** `renkin` Python package -- not
`cargo test --features python --lib`, which never links the cdylib
`extension-module` build and so exercises none of `src/python.rs`'s actual
behavior regardless of whether that feature is enabled (confirmed: the
`--lib` test count is identical with or without `--features python`).

Skips entirely if `renkin` isn't importable (e.g. no wheel installed in
this environment) -- mirrors `test_compare_renkin_adapter.py`'s
`requires_renkin_bin` pattern for an optional prerequisite, not a hard
dependency of this repo's default `python3 -m unittest discover`. To
actually run these: `maturin develop --features python` (or install a
built wheel) first. CI's "Python smoke" job does install a real wheel
already (for the pre-existing minimal existence/callability checks); this
file is invoked from that same job, after installation, from the repo
checkout (not /tmp, unlike the pre-existing smoke check) so the
repo-relative fixture path below resolves.
"""

import json
import os
import unittest
from pathlib import Path

try:
    import renkin

    RENKIN_IMPORTABLE = True
except ImportError:
    RENKIN_IMPORTABLE = False

requires_renkin_module = unittest.skipUnless(
    RENKIN_IMPORTABLE,
    "requires an installed renkin Python package (maturin develop / pip install a built wheel)",
)

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
FIXTURE_TEMPLATES = REPO_ROOT / "tests" / "fixtures" / "coverage_mode_templates.smi"

BUILDING_BLOCK = "CC(=O)O"  # acetic acid: depth-0 building block
# Same fixture target/config as tests/coverage_mode_cli.rs -- see that
# file's module doc and tests/fixtures/coverage_mode_templates.smi's own
# header comment for why this target/depth/beam_width combination.
STAGE1_UNSOLVED_AT_FIXTURE = "O=C1CCC(=O)N1c1ccccc1"


@requires_renkin_module
class TestPythonCoverageMode(unittest.TestCase):
    def _find_routes(self, target, **kwargs):
        return json.loads(renkin.find_routes(target, **kwargs))

    def test_standard_mode_output_has_no_coverage_mode_keys(self):
        r = self._find_routes(BUILDING_BLOCK, depth=2, max_routes=1)
        for key in (
            "search_mode",
            "selected_stage",
            "stage2_invoked",
            "stage1_timeout",
            "stage2_timeout",
            "stage1_elapsed_ms",
            "stage2_elapsed_ms",
            "total_elapsed_ms",
        ):
            self.assertNotIn(key, r, f"standard mode must never emit {key}")

    def test_invalid_search_mode_raises(self):
        with self.assertRaises(ValueError):
            renkin.find_routes(BUILDING_BLOCK, search_mode="bogus")

    def test_missing_coverage_templates_path_raises(self):
        with self.assertRaises(ValueError):
            renkin.find_routes(BUILDING_BLOCK, search_mode="coverage")

    def test_coverage_flags_in_standard_mode_raise(self):
        with self.assertRaises(ValueError):
            renkin.find_routes(
                BUILDING_BLOCK, coverage_templates_path=str(FIXTURE_TEMPLATES)
            )
        with self.assertRaises(ValueError):
            renkin.find_routes(BUILDING_BLOCK, coverage_timeout_seconds=10)

    def test_coverage_timeout_seconds_zero_raises(self):
        with self.assertRaises(ValueError):
            renkin.find_routes(
                BUILDING_BLOCK,
                search_mode="coverage",
                coverage_templates_path=str(FIXTURE_TEMPLATES),
                coverage_timeout_seconds=0,
            )

    def test_stage1_solved_reports_stage2_invoked_false(self):
        r = self._find_routes(
            BUILDING_BLOCK,
            depth=2,
            max_routes=1,
            search_mode="coverage",
            coverage_templates_path=str(FIXTURE_TEMPLATES),
        )
        self.assertEqual(r["routes_found"], 1)
        self.assertEqual(r["selected_stage"], "stage1")
        self.assertFalse(r["stage2_invoked"])

    def test_stage1_unsolved_reports_stage2_invoked_true(self):
        r = self._find_routes(
            STAGE1_UNSOLVED_AT_FIXTURE,
            depth=2,
            max_routes=1,
            beam_width=100,
            search_mode="coverage",
            coverage_templates_path=str(FIXTURE_TEMPLATES),
        )
        self.assertEqual(r["selected_stage"], "stage2")
        self.assertTrue(r["stage2_invoked"])
        self.assertEqual(r["routes_found"], 1)

    # Requirement: Python top_templates applies only to Stage 1, never to
    # coverage_templates_path. top_templates=1 cripples Stage 1 down to a
    # single (arbitrary, by-weight) template from the 2-line fixture --
    # nowhere near enough to solve STAGE1_UNSOLVED_AT_FIXTURE, which needs
    # both lines together (see the fixture file's header). If Stage 2 were
    # (wrongly) also subject to top_templates, it would fail too, and
    # stage2_invoked would still be true but routes_found would be 0.
    def test_top_templates_applies_only_to_stage1(self):
        r = self._find_routes(
            STAGE1_UNSOLVED_AT_FIXTURE,
            depth=2,
            max_routes=1,
            beam_width=100,
            templates_path=str(FIXTURE_TEMPLATES),
            top_templates=1,
            search_mode="coverage",
            coverage_templates_path=str(FIXTURE_TEMPLATES),
        )
        self.assertTrue(
            r["stage2_invoked"],
            "Stage 1, crippled to 1 template by top_templates, must NOT have solved this target",
        )
        self.assertEqual(
            r["routes_found"],
            1,
            "Stage 2 (coverage_templates_path, unaffected by top_templates) must still solve it",
        )

    # Requirement: CLI/Python parity for the fields coverage mode *adds* --
    # not full top-level schema parity (Python's output has a pre-existing,
    # unrelated gap where it lacks `joint_success_probability` on the
    # route-found branch, nothing to do with coverage mode, not addressed
    # here). Only the coverage-mode-added field names/types/omission rules
    # need to match between CLI and Python.
    def test_cli_python_coverage_field_parity(self):
        import subprocess

        cli_bin = REPO_ROOT / "target" / "release" / "renkin"
        if not cli_bin.exists():
            self.skipTest(f"requires a built renkin binary at {cli_bin}")
        cli_out = subprocess.run(
            [
                str(cli_bin),
                "--target",
                STAGE1_UNSOLVED_AT_FIXTURE,
                "--depth",
                "2",
                "--max-routes",
                "1",
                "--beam-width",
                "100",
                "--search-mode",
                "coverage",
                "--coverage-templates",
                str(FIXTURE_TEMPLATES),
            ],
            capture_output=True,
            text=True,
            check=True,
            cwd=str(REPO_ROOT),
        ).stdout
        cli_json = json.loads(cli_out)
        py_json = self._find_routes(
            STAGE1_UNSOLVED_AT_FIXTURE,
            depth=2,
            max_routes=1,
            beam_width=100,
            search_mode="coverage",
            coverage_templates_path=str(FIXTURE_TEMPLATES),
        )
        coverage_keys = {
            "search_mode",
            "selected_stage",
            "stage2_invoked",
            "stage1_timeout",
            "stage2_timeout",
            "stage1_elapsed_ms",
            "stage2_elapsed_ms",
            "total_elapsed_ms",
        }
        self.assertTrue(coverage_keys <= cli_json.keys())
        self.assertTrue(coverage_keys <= py_json.keys())
        for key in coverage_keys:
            self.assertEqual(
                type(cli_json[key]),
                type(py_json[key]),
                f"{key}: CLI={cli_json[key]!r} ({type(cli_json[key])}) vs. "
                f"Python={py_json[key]!r} ({type(py_json[key])})",
            )
        self.assertEqual(cli_json["selected_stage"], py_json["selected_stage"])


if __name__ == "__main__":
    unittest.main()
