"""Tests for `renkin.audit_route_report` (v0.32.0 Phase 2A, typed Python
report API) -- a pure-Python wrapper around the existing, unchanged
`renkin.audit_route(...) -> str`. See `python/renkin/audit_report.py`'s
module docstring for the absent-vs-null and enum-as-`str` design notes.

Same import/skip convention as `test_python_audit_route.py`: skip
entirely if `renkin` isn't importable.

The core correctness gate here is `_assert_report_matches_raw_json`: it
round-trips the typed `AuditRouteReport` back into a dict and asserts it
equals `json.loads()` of the exact same input's raw string output. This
is the drift-detection guarantee real JSON-Schema codegen would give,
achieved as a test instead of new build tooling (see the v0.32.0 Phase 2A
plan's own reasoning for this substitution) -- if `audit_report.py`'s
dataclasses ever drift from the real Rust JSON shape, this test fails
loud, for every fixture below, not just one.
"""

import dataclasses
import json
import subprocess
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

# Same target used by test_python_audit_route.py -- fast, reliable,
# forward-replays correctly against the default rule corpus.
TARGET = "CCOC(=O)c1ccccc1"

AIZYNTHFINDER_FIXTURE = (
    REPO_ROOT / "tests" / "fixtures" / "aizynthfinder" / "v4.4.1" / "single_trees.json"
)
SYNTHESEUS_FIXTURES = [
    REPO_ROOT / "tests" / "fixtures" / "syntheseus" / "0.7.2" / "linear_two_leaf_route.json",
    REPO_ROOT / "tests" / "fixtures" / "syntheseus" / "0.7.2" / "convergent_route.json",
    REPO_ROOT / "tests" / "fixtures" / "syntheseus" / "0.8.0" / "linear_two_leaf_route.json",
    REPO_ROOT / "tests" / "fixtures" / "syntheseus" / "0.8.0" / "convergent_route.json",
]


def _generate_renkin_native_fixture_json():
    """Same real-CLI-search approach as test_python_audit_route.py's
    _generate_route_fixture_json -- never a hand-authored fixture."""
    cli_bin = REPO_ROOT / "target" / "release" / "renkin"
    if not cli_bin.exists():
        return None
    out = subprocess.run(
        [str(cli_bin), "--target", TARGET, "--depth", "1"],
        capture_output=True,
        text=True,
        check=True,
        cwd=str(REPO_ROOT),
    )
    return out.stdout


def _fill_missing_keys_with_none(test_case, reference, target):
    """Recursively fills `target` (the raw JSON, which may legitimately
    omit optional keys via Rust's `skip_serializing_if`) with `None` for
    any key present in `reference` (the typed side's full dict, via
    `dataclasses.asdict()`, which always has every field) but missing
    from `target` -- this is the absent-vs-null collapse documented in
    `audit_report.py`'s module docstring, not a bug to paper over.

    Also asserts the reference's own value is actually `None` wherever a
    key gets filled in this way -- if the typed side ever has a non-None
    value for a field the raw JSON never sent, that's a real bug this
    must catch, not silently accept. And asserts `target` carries no key
    `reference` lacks entirely -- the collapse only goes one direction
    (typed side has the field, raw side may omit it), never the other;
    an unmodeled real field in the raw JSON is drift, not a variant of
    the documented simplification.
    """
    if isinstance(reference, dict) and isinstance(target, dict):
        extra = set(target) - set(reference)
        test_case.assertFalse(
            extra,
            f"raw JSON carries key(s) {sorted(extra)} the typed model "
            "doesn't represent at all -- this is real drift, not the "
            "documented absent-vs-null collapse",
        )
        filled = {}
        for key, ref_val in reference.items():
            if key in target:
                filled[key] = _fill_missing_keys_with_none(test_case, ref_val, target[key])
            else:
                test_case.assertIsNone(
                    ref_val,
                    f"key {key!r} is absent from the raw JSON but the typed "
                    f"side has a non-None value {ref_val!r} for it",
                )
                filled[key] = None
        return filled
    if isinstance(reference, list) and isinstance(target, list):
        return [_fill_missing_keys_with_none(test_case, r, t) for r, t in zip(reference, target)]
    return target


def _assert_report_matches_raw_json(test_case, content, **kwargs):
    """The core parity check: audit the same content+kwargs via both the
    string API and the typed API, and assert the typed object -- flattened
    back to a dict -- reproduces the raw JSON's actual values exactly,
    modulo two deliberate, documented differences:

    1. `summary.pass` is renamed to `summary.passed` on the typed side
       (`pass` is a Python reserved word) -- remapped before comparison.
    2. Keys Rust omits entirely (`skip_serializing_if`) become an
       explicit `None` on the typed side -- `_fill_missing_keys_with_none`
       normalizes the raw side to match, while still verifying the typed
       side's value really is `None` in every such case (see above).
    """
    raw = renkin.audit_route(content, **kwargs)
    raw_json = json.loads(raw)

    report = renkin.audit_route_report(content, **kwargs)
    report_dict = dataclasses.asdict(report)

    # Undo the one deliberate rename so the rest of the dict compares
    # directly against the real wire shape.
    report_dict["summary"]["pass"] = report_dict["summary"].pop("passed")

    filled_raw_json = _fill_missing_keys_with_none(test_case, report_dict, raw_json)
    test_case.assertEqual(
        report_dict,
        filled_raw_json,
        "audit_route_report()'s dataclasses must reproduce audit_route()'s "
        "raw JSON values exactly (modulo the two documented differences)",
    )
    return report, raw_json


@requires_renkin_module
class TestPythonAuditRouteReportParity(unittest.TestCase):
    def test_renkin_native_route(self):
        content = _generate_renkin_native_fixture_json()
        if content is None:
            self.skipTest("requires a built renkin binary at target/release/renkin")
        _assert_report_matches_raw_json(self, content)

    def test_renkin_native_route_strict_policy(self):
        content = _generate_renkin_native_fixture_json()
        if content is None:
            self.skipTest("requires a built renkin binary at target/release/renkin")
        _assert_report_matches_raw_json(self, content, policy="strict")

    def test_aizynthfinder_route(self):
        if not AIZYNTHFINDER_FIXTURE.exists():
            self.skipTest(f"missing fixture: {AIZYNTHFINDER_FIXTURE}")
        content = AIZYNTHFINDER_FIXTURE.read_text(encoding="utf-8")
        _assert_report_matches_raw_json(self, content, format="aizynthfinder")

    def test_syntheseus_routes_both_verified_versions(self):
        for fixture_path in SYNTHESEUS_FIXTURES:
            if not fixture_path.exists():
                self.skipTest(f"missing fixture: {fixture_path}")
                continue
            with self.subTest(fixture=fixture_path.name):
                content = fixture_path.read_text(encoding="utf-8")
                _assert_report_matches_raw_json(self, content, format="syntheseus")

    def test_invalid_policy_raises_value_error(self):
        content = _generate_renkin_native_fixture_json()
        if content is None:
            self.skipTest("requires a built renkin binary at target/release/renkin")
        with self.assertRaises(ValueError):
            renkin.audit_route_report(content, policy="bogus")

    def test_malformed_json_raises_value_error(self):
        with self.assertRaises(ValueError):
            renkin.audit_route_report("not json")

    # Attribute-access sanity matching the spec's own literal examples --
    # report.audit_manifest.policy / report.routes[0].findings /
    # report.routes[0].steps[0].forward_validation.
    def test_attribute_access_matches_spec_examples(self):
        if not AIZYNTHFINDER_FIXTURE.exists():
            self.skipTest(f"missing fixture: {AIZYNTHFINDER_FIXTURE}")
        content = AIZYNTHFINDER_FIXTURE.read_text(encoding="utf-8")
        report = renkin.audit_route_report(content, format="aizynthfinder")

        self.assertEqual(report.audit_manifest.policy, "standard")
        self.assertIsInstance(report.routes[0].findings, list)
        if report.routes[0].steps:
            fv = report.routes[0].steps[0].forward_validation
            self.assertIn(fv.status, ("pass", "fail", "not_evaluable"))

    def test_source_field_reflects_input_format(self):
        if not AIZYNTHFINDER_FIXTURE.exists():
            self.skipTest(f"missing fixture: {AIZYNTHFINDER_FIXTURE}")
        content = AIZYNTHFINDER_FIXTURE.read_text(encoding="utf-8")
        report = renkin.audit_route_report(content, format="aizynthfinder")
        self.assertEqual(report.routes[0].source, "ai_zynth_finder")


if __name__ == "__main__":
    unittest.main()
