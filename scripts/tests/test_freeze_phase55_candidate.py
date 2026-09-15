import importlib.util
import json
import tempfile
from pathlib import Path

import pytest


MODULE_PATH = Path(__file__).parents[1] / "freeze_phase55_candidate.py"
SPEC = importlib.util.spec_from_file_location("phase55_candidate_freeze", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def write_json(path: Path, value: dict) -> Path:
    path.write_text(json.dumps(value), encoding="utf-8")
    return path


def manifest(configuration_id: str) -> dict:
    return {
        "end_time_unix": 1,
        "input_files_unchanged_during_run": True,
        "configuration_id": configuration_id,
        "tool_version": "1.0.7",
        "input_file_sha256": {"sample_list": "sample", "stock": "stock", "templates": "templates"},
    }


def verification(**updates: object) -> dict:
    value = {
        "row_count": 2,
        "zero_regression": True,
        "zero_strict_regression": True,
        "within_budget": True,
        "within_process_budget": True,
        "timeout_count": 0,
        "crash_count": 0,
        "missing_attempts_count": 0,
        "recovered_count": 1,
        "baseline_strict_success_count": 1,
        "final_strict_success_count": 2,
        "budget_ms": 31000,
        "process_budget_ms": 31000,
    }
    value.update(updates)
    return value


def rows(path: Path, identifiers: tuple[str, ...] = ("a", "b")) -> Path:
    path.write_text("".join(json.dumps({"target_id": item}) + "\n" for item in identifiers), encoding="utf-8")
    return path


def test_freezes_verified_development_candidate():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        result = MODULE.freeze_candidate(
            baseline_manifest_path=write_json(root / "baseline.json", manifest("baseline")),
            candidate_manifest_path=write_json(root / "candidate.json", manifest("recovery")),
            baseline_rows_path=rows(root / "baseline.rows"),
            candidate_rows_path=rows(root / "candidate.rows"),
            verification_path=write_json(root / "verification.json", verification()),
            output_path=root / "frozen.json",
            selection_id="phase55-val-v2",
        )
        assert result["selection_status"] == "frozen"
        assert result["development_only"] is True
        assert result["candidate"]["configuration_id"] == "recovery"


def test_rejects_failed_budget_or_missing_recovery():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        common = dict(
            baseline_manifest_path=write_json(root / "baseline.json", manifest("baseline")),
            candidate_manifest_path=write_json(root / "candidate.json", manifest("recovery")),
            baseline_rows_path=rows(root / "baseline.rows"),
            candidate_rows_path=rows(root / "candidate.rows"),
            output_path=root / "frozen.json",
            selection_id="phase55-val-v2",
        )
        common["verification_path"] = write_json(
            root / "verification.json", verification(within_process_budget=False)
        )
        with pytest.raises(ValueError, match="within_process_budget"):
            MODULE.freeze_candidate(**common)


def test_rejects_changed_row_target_set():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        with pytest.raises(ValueError, match="target sets differ"):
            MODULE.freeze_candidate(
                baseline_manifest_path=write_json(root / "baseline.json", manifest("baseline")),
                candidate_manifest_path=write_json(root / "candidate.json", manifest("recovery")),
                baseline_rows_path=rows(root / "baseline.rows"),
                candidate_rows_path=rows(root / "candidate.rows", ("a", "different")),
                verification_path=write_json(root / "verification.json", verification()),
                output_path=root / "frozen.json",
                selection_id="phase55-val-v2",
            )
