import importlib.util
import json
import tempfile
from pathlib import Path


MODULE_PATH = Path(__file__).parents[1] / "phase55_preflight.py"
SPEC = importlib.util.spec_from_file_location("phase55_preflight", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def manifest(commit="abc", clean=True):
    return {
        "comparison_mode": "shared_stock",
        "tool_version": "1.0.6",
        "input_file_sha256": {"sample_list": "s", "stock": "k", "templates": "t"},
        "resource_budget": {
            "depth": 5,
            "beam_width": 100,
            "timeout_s": 120,
            "grace_s": 5,
            "max_routes": 1,
            "execution_enforcement": {
                "cpu_enforced": True,
                "memory_enforced": True,
            },
        },
        "git_commit": commit,
        "worktree": {"clean": clean},
        "input_files_unchanged_during_run": True,
    }


def test_matching_pair_is_eligible():
    result = MODULE.preflight(manifest(), manifest(), {"a", "b"}, {"a", "b"})
    assert result["eligible"] is True
    assert result["blockers"] == []


def test_current_git_worktree_schema_is_accepted():
    current = manifest()
    current.pop("worktree")
    current["git_worktree"] = {"clean": True, "changed_entry_count": 0}
    result = MODULE.preflight(current, current, {"a"}, {"a"})
    assert result["eligible"] is True


def test_tool_specific_depth_and_beam_may_differ():
    right = manifest()
    right["resource_budget"].update({"depth": 6, "beam_width": 200})
    result = MODULE.preflight(manifest(), right, {"a"}, {"a"})
    assert result["eligible"] is True


def test_formal_preflight_rejects_missing_or_ineffective_enforcement():
    right = manifest()
    right["resource_budget"]["execution_enforcement"]["memory_enforced"] = False
    result = MODULE.preflight(
        manifest(), right, {"a"}, {"a"}, require_effective_resource_enforcement=True
    )
    assert result["eligible"] is False
    assert "right_memory_enforcement_not_effective" in result["blockers"]


def test_preflight_rejects_stale_or_incomplete_pair():
    right = manifest(commit="different", clean=False)
    right["input_file_sha256"]["stock"] = "other"
    right["resource_budget"] = None
    result = MODULE.preflight(manifest(), right, {"a"}, {"b"})
    assert result["eligible"] is False
    assert {
        "target_id_set_mismatch:left_only=1:right_only=1",
        "input_hash_mismatch:stock",
        "missing_resource_budget",
        "git_revision_mismatch",
        "right_worktree_not_clean_or_unrecorded",
    } <= set(result["blockers"])


def test_frozen_cohort_preflight_checks_sample_list_identity():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        sample = root / "sample.jsonl"
        rows = [
            {"target_id": "a"},
            {"target_id": "b"},
        ]
        sample.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
        import hashlib

        sample_hash = hashlib.sha256(sample.read_bytes()).hexdigest()
        frozen = root / "frozen.json"
        frozen.write_text(
            json.dumps(
                {
                    "freeze_status": "frozen",
                    "freeze_id": "test",
                    "targets": rows,
                    "sample_list": {
                        "path": str(sample),
                        "sha256": sample_hash,
                        "rows": 2,
                    },
                }
            ),
            encoding="utf-8",
        )
        result = MODULE.frozen_cohort_preflight(frozen)
        assert result["eligible"] is True
        assert result["target_count"] == 2


def test_frozen_cohort_preflight_rejects_changed_sample_list():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        sample = root / "sample.jsonl"
        sample.write_text('{"target_id": "a"}\n', encoding="utf-8")
        frozen = root / "frozen.json"
        frozen.write_text(
            json.dumps(
                {
                    "freeze_status": "frozen",
                    "targets": [{"target_id": "a"}],
                    "sample_list": {"path": str(sample), "sha256": "wrong", "rows": 1},
                }
            ),
            encoding="utf-8",
        )
        result = MODULE.frozen_cohort_preflight(frozen)
        assert result["eligible"] is False
        assert "cohort_sample_list_hash_mismatch" in result["blockers"]


def test_smoke_preflight_requires_frozen_prefix_and_hash():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        sample = root / "sample.jsonl"
        rows = [{"target_id": value} for value in ("a", "b", "c")]
        sample.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
        import hashlib

        sample_hash = hashlib.sha256(sample.read_bytes()).hexdigest()
        frozen = root / "frozen.json"
        frozen.write_text(
            json.dumps(
                {
                    "freeze_status": "frozen",
                    "freeze_id": "test",
                    "targets": rows,
                    "sample_list": {
                        "path": str(sample),
                        "sha256": sample_hash,
                        "rows": len(rows),
                    },
                }
            ),
            encoding="utf-8",
        )
        left = manifest()
        right = manifest()
        left["input_file_sha256"]["sample_list"] = sample_hash
        right["input_file_sha256"]["sample_list"] = sample_hash
        result = MODULE.smoke_preflight(
            frozen, left, right, {"a", "b"}, {"a", "b"}, smoke_size=2
        )
        assert result["eligible"] is True
        assert result["freeze_id"] == "test"


def test_smoke_preflight_rejects_non_prefix_or_wrong_input_hash():
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        sample = root / "sample.jsonl"
        rows = [{"target_id": value} for value in ("a", "b", "c")]
        sample.write_text("".join(json.dumps(row) + "\n" for row in rows), encoding="utf-8")
        import hashlib

        sample_hash = hashlib.sha256(sample.read_bytes()).hexdigest()
        frozen = root / "frozen.json"
        frozen.write_text(
            json.dumps(
                {
                    "freeze_status": "frozen",
                    "targets": rows,
                    "sample_list": {
                        "path": str(sample), "sha256": sample_hash, "rows": len(rows)
                    },
                }
            ),
            encoding="utf-8",
        )
        result = MODULE.smoke_preflight(
            frozen, manifest(), manifest(), {"a", "c"}, {"a", "c"}, smoke_size=2
        )
        assert result["eligible"] is False
        assert "smoke_target_set_mismatch:expected=2:left=2:right=2" in result["blockers"]
        assert "left_smoke_sample_hash_mismatch" in result["blockers"]
