"""Resolver smoke test for the `renkin[syntheseus]` optional extra
(v0.31.0 Phase 1 PR2). Two clean venvs, each pip's own real resolver
running against a real locally-built wheel -- not asserted from reading
`pyproject.toml`, since the declared spec and the resolver's actual
behavior are two different questions.

Case A -- default extra resolution: a bare clean venv installs
`renkin[syntheseus]` with nothing pre-installed; pip should resolve the
newest version satisfying the declared interval (`0.8.0`).

Case B -- lower verified endpoint: `syntheseus==0.7.2` is installed
first (exact), then `renkin[syntheseus]` is installed on top; since
`0.7.2` already satisfies the declared interval, pip's default resolver
must NOT replace it with `0.8.0` -- an unnecessary upgrade here would
mean the interval isn't actually respecting an already-satisfied lower
endpoint.

Each case also runs the real `test_python_syntheseus_exporter.py` suite
against its own venv, and `pip check` for dependency consistency.
Exits nonzero (and prints a clear per-case PASS/FAIL summary) on any
failure -- meant to be run standalone (`python3 scripts/
check_syntheseus_dependency_resolution.py`) or as a CI step.
"""

import shutil
import subprocess
import sys
import tempfile
import venv
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def run(cmd, **kwargs):
    print(f"    $ {' '.join(cmd)}")
    return subprocess.run(cmd, check=True, capture_output=True, text=True, **kwargs)


def build_wheel(out_dir: Path) -> Path:
    run(["maturin", "build", "--release", "--features", "python", "--out", str(out_dir)], cwd=REPO_ROOT)
    wheels = list(out_dir.glob("*.whl"))
    assert len(wheels) == 1, f"expected exactly one built wheel, got {wheels}"
    return wheels[0]


def venv_python(venv_dir: Path) -> Path:
    return venv_dir / ("Scripts/python.exe" if sys.platform == "win32" else "bin/python3")


def installed_version(py: Path, package: str) -> str:
    result = run([str(py), "-c", f"import importlib.metadata; print(importlib.metadata.version('{package}'))"])
    return result.stdout.strip()


def run_exporter_tests(py: Path) -> None:
    run([str(py), "-m", "unittest", "scripts.tests.test_python_syntheseus_exporter", "-v"], cwd=REPO_ROOT)


def case_a_default_resolution(wheel: Path, workdir: Path) -> dict:
    print("\n=== Case A: default extra resolution (clean venv, nothing pre-installed) ===")
    venv_dir = workdir / "venv_a"
    venv.create(venv_dir, with_pip=True, clear=True)
    py = venv_python(venv_dir)
    run([str(py), "-m", "pip", "install", "--quiet", f"{wheel}[syntheseus]"])
    resolved = installed_version(py, "syntheseus")
    run([str(py), "-m", "pip", "check"])
    run_exporter_tests(py)
    print(f"    resolved syntheseus version: {resolved}")
    return {"resolved_version": resolved, "expected": "0.8.0", "pass": resolved == "0.8.0"}


def case_b_lower_endpoint_preserved(wheel: Path, workdir: Path) -> dict:
    print("\n=== Case B: 0.7.2 pre-installed, must not be upgraded ===")
    venv_dir = workdir / "venv_b"
    venv.create(venv_dir, with_pip=True, clear=True)
    py = venv_python(venv_dir)
    run([str(py), "-m", "pip", "install", "--quiet", "syntheseus==0.7.2"])
    before = installed_version(py, "syntheseus")
    run([str(py), "-m", "pip", "install", "--quiet", f"{wheel}[syntheseus]"])
    after = installed_version(py, "syntheseus")
    run([str(py), "-m", "pip", "check"])
    run_exporter_tests(py)
    print(f"    syntheseus version before: {before}, after: {after}")
    return {"before": before, "after": after, "pass": before == "0.7.2" and after == "0.7.2"}


def main() -> int:
    if shutil.which("maturin") is None:
        print("SKIP: maturin not on PATH")
        return 0

    with tempfile.TemporaryDirectory() as tmp:
        workdir = Path(tmp)
        print("Building renkin wheel...")
        wheel = build_wheel(workdir / "dist")
        print(f"  wheel: {wheel.name}")

        results = {
            "case_a": case_a_default_resolution(wheel, workdir),
            "case_b": case_b_lower_endpoint_preserved(wheel, workdir),
        }

    print("\n=== Summary ===")
    all_pass = True
    for name, r in results.items():
        status = "PASS" if r["pass"] else "FAIL"
        if not r["pass"]:
            all_pass = False
        print(f"  {name}: {status} -- {r}")

    return 0 if all_pass else 1


if __name__ == "__main__":
    sys.exit(main())
