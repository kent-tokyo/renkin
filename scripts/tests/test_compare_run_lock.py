import importlib.util
from pathlib import Path

import pytest


MODULE_PATH = Path(__file__).parents[1] / "compare_run.py"
SPEC = importlib.util.spec_from_file_location("compare_run_lock_test", MODULE_PATH)
assert SPEC and SPEC.loader
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def test_output_lock_rejects_concurrent_writer(tmp_path):
    output = tmp_path / "rows.jsonl"
    first = MODULE.acquire_output_lock(output)
    try:
        with pytest.raises(ValueError, match="already locked"):
            MODULE.acquire_output_lock(output)
    finally:
        MODULE.fcntl.flock(first.fileno(), MODULE.fcntl.LOCK_UN)
        first.close()
        MODULE._OUTPUT_LOCK_HANDLES.remove(first)
