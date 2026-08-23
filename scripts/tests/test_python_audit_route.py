"""Tests for `renkin.audit_route` (v0.29.0 Audit Policy Profiles, PR2) --
the first Python binding for route auditing -- against the actual
**installed** `renkin` Python package, same convention as
`test_python_coverage_mode.py` (see that file's own module doc for why
`cargo test --features python --lib` can't exercise this: it never links
the cdylib `extension-module` build).

Skips entirely if `renkin` isn't importable, mirroring
`test_python_coverage_mode.py`'s `requires_renkin_module` pattern. To
actually run these: `maturin develop --features python` (or install a
built wheel) first. CI's "Python smoke" job does this already.
"""

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

# A route that forward-replays correctly against the real default rule
# corpus (co_aliphatic_cleavage) -- same target this Bridge program's own
# Rust/CLI tests already use (see tests/audit_route_cli.rs's
# generate_route_fixture doc comment for why this target/depth is fast
# and reliable).
TARGET = "CCOC(=O)c1ccccc1"


def _generate_route_fixture_json():
    """Runs the real CLI's own search path to produce a genuine
    `--format json` route export -- not a hand-authored fixture that
    could silently drift from what the CLI really emits, same reasoning
    as `tests/audit_route_cli.rs::generate_route_fixture`."""
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


@requires_renkin_module
class TestPythonAuditRoute(unittest.TestCase):
    def setUp(self):
        content = _generate_route_fixture_json()
        if content is None:
            self.skipTest("requires a built renkin binary at target/release/renkin")
        self.content = content

    def test_default_policy_is_standard(self):
        report = json.loads(renkin.audit_route(self.content))
        self.assertEqual(report["audit_manifest"]["policy"], "standard")

    def test_no_stock_is_not_evaluable_partial_under_standard(self):
        report = json.loads(renkin.audit_route(self.content))
        self.assertEqual(
            report["routes"][0]["stock_validation"]["status"], "not_evaluable"
        )
        self.assertEqual(report["routes"][0]["status"], "partial")

    def test_strict_policy_hardens_not_evaluable_to_fail(self):
        report = json.loads(renkin.audit_route(self.content, policy="strict"))
        self.assertEqual(report["audit_manifest"]["policy"], "strict")
        self.assertEqual(report["routes"][0]["status"], "fail")

    def test_informational_policy_stays_partial(self):
        report = json.loads(renkin.audit_route(self.content, policy="informational"))
        self.assertEqual(report["routes"][0]["status"], "partial")

    def test_invalid_policy_raises_value_error(self):
        with self.assertRaises(ValueError):
            renkin.audit_route(self.content, policy="bogus")

    def test_invalid_format_raises_value_error(self):
        with self.assertRaises(ValueError):
            renkin.audit_route(self.content, format="bogus")

    def test_malformed_json_raises_value_error(self):
        with self.assertRaises(ValueError):
            renkin.audit_route("not json")

    def test_stock_text_parses_smi_style_lines(self):
        # Full stock covering the route's own real leaves (read from the
        # fixture's own building_blocks, not guessed -- co_aliphatic_cleavage
        # splits the C-O bond generically, so the actual leaves aren't
        # necessarily the "obvious" ester-hydrolysis products) -> a
        # genuine forward-replayable Pass, exercising stock_text end to
        # end (not just the no-stock not_evaluable path above).
        route = json.loads(self.content)["routes"][0]
        stock_text = "".join(f"{bb} leaf\n" for bb in route["building_blocks"])
        report = json.loads(renkin.audit_route(self.content, stock_text=stock_text))
        self.assertEqual(report["routes"][0]["stock_validation"]["status"], "pass")
        self.assertEqual(report["routes"][0]["status"], "pass")

    # Cross-surface parity: CLI `--policy` and Python `policy=` must
    # produce the identical report for the identical input, since both
    # are thin wrappers over the same bridge::build_audit_route_report_with_policy.
    def test_cli_python_parity_across_all_three_policies(self):
        cli_bin = REPO_ROOT / "target" / "release" / "renkin"
        route_path = REPO_ROOT / "target" / "test_python_audit_route_fixture.json"
        route_path.write_text(self.content, encoding="utf-8")
        try:
            for policy in ("informational", "standard", "strict"):
                cli_out = subprocess.run(
                    [
                        str(cli_bin),
                        "audit-route",
                        str(route_path),
                        "--policy",
                        policy,
                        "--output",
                        "json",
                    ],
                    capture_output=True,
                    text=True,
                    check=True,
                    cwd=str(REPO_ROOT),
                ).stdout
                cli_json = json.loads(cli_out)
                py_json = json.loads(renkin.audit_route(self.content, policy=policy))
                self.assertEqual(
                    cli_json,
                    py_json,
                    f"policy={policy}: CLI and Python reports must be byte-for-byte "
                    "identical (input_sha256 included -- both hash the exact same "
                    "content string)",
                )
        finally:
            route_path.unlink(missing_ok=True)


if __name__ == "__main__":
    unittest.main()
