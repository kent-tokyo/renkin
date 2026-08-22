"""Distribution-metadata tests for the `renkin[syntheseus]` optional
extra (v0.31.0 Phase 1 PR2). Builds a real wheel via `maturin build` and
reads its `METADATA` file structurally -- `email.message_from_string`,
the same RFC822 parsing `importlib.metadata`/`packaging.metadata` use
internally -- rather than grepping strings, so a reformatted-but-
equivalent line can't silently slip past these checks.

Confirms the declared dependency interval is exactly what
`pyproject.toml` says (`>=0.7.2,<=0.8.0`, per the real dual-version
verification in `docs/design/syntheseus-0.8-compatibility-spike.md`),
correctly gated behind the `syntheseus` extra, and that no stale exact
`==0.7.2` pin survives from before PR2.

Skips if `maturin` isn't on `PATH` -- this test builds its own wheel
rather than depending on one already existing, so the skip condition is
build-tooling availability, not an already-installed package (contrast
`test_python_syntheseus_exporter.py`'s `requires_syntheseus`).
"""

import email
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent.parent

requires_maturin = unittest.skipUnless(
    shutil.which("maturin") is not None,
    "requires maturin on PATH to build a real wheel",
)


def _build_wheel_and_read_metadata():
    """Builds a real wheel via `maturin build` into a fresh temp dir and
    discovers it by glob -- never a hardcoded filename, since the
    platform/interpreter tag varies by machine."""
    with tempfile.TemporaryDirectory() as out_dir:
        subprocess.run(
            ["maturin", "build", "--release", "--features", "python", "--out", out_dir],
            cwd=REPO_ROOT,
            check=True,
            capture_output=True,
            text=True,
        )
        wheels = list(Path(out_dir).glob("*.whl"))
        assert len(wheels) == 1, f"expected exactly one built wheel, got {wheels}"
        with zipfile.ZipFile(wheels[0]) as z:
            metadata_names = [n for n in z.namelist() if n.endswith(".dist-info/METADATA")]
            assert len(metadata_names) == 1, f"expected exactly one METADATA file, got {metadata_names}"
            content = z.read(metadata_names[0]).decode("utf-8")
    return email.message_from_string(content)


@requires_maturin
class TestSyntheseusExtraMetadata(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.metadata = _build_wheel_and_read_metadata()

    def _syntheseus_requires_dist_lines(self):
        requires = self.metadata.get_all("Requires-Dist") or []
        return [r for r in requires if r.split(";")[0].strip().lower().startswith("syntheseus")]

    def test_provides_extra_syntheseus(self):
        extras = self.metadata.get_all("Provides-Extra") or []
        self.assertIn("syntheseus", extras, f"Provides-Extra values: {extras}")

    def test_requires_dist_declares_the_verified_interval(self):
        lines = self._syntheseus_requires_dist_lines()
        self.assertEqual(
            len(lines), 1, f"expected exactly one syntheseus Requires-Dist line, got {lines}"
        )
        dep_part, _, marker_part = lines[0].partition(";")
        self.assertIn(">=0.7.2", dep_part, dep_part)
        self.assertIn("<=0.8.0", dep_part, dep_part)
        # Explicitly NOT an open-ended upper bound -- verified and
        # supported are not the same claim, an unverified future version
        # must not be silently accepted.
        self.assertNotIn("<0.9", dep_part, dep_part)
        self.assertIn("extra ==", marker_part, marker_part)
        self.assertIn("syntheseus", marker_part, marker_part)

    def test_no_stale_exact_pin_survives(self):
        for line in self.metadata.get_all("Requires-Dist") or []:
            self.assertNotIn(
                "syntheseus==0.7.2", line.replace(" ", ""),
                f"stale exact pin found in a Requires-Dist line: {line!r}",
            )

    def test_base_install_has_no_ungated_syntheseus_dependency(self):
        for line in self._syntheseus_requires_dist_lines():
            self.assertIn(
                "extra ==", line, f"syntheseus Requires-Dist not gated behind an extra: {line!r}"
            )


if __name__ == "__main__":
    unittest.main()
