import os
import sys


sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

from four_tool_resources import cpu_rlimit_seconds, resource_environment  # noqa: E402


def test_cpu_rlimit_covers_parallel_wall_clock_budget():
    assert cpu_rlimit_seconds(150, 8) == 1200


def test_cpu_rlimit_rejects_invalid_resource_values():
    for wall_clock_seconds, cpus in ((0, 8), (150, 0), (-1, 1), (1, -1)):
        try:
            cpu_rlimit_seconds(wall_clock_seconds, cpus)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid resource values must be rejected")


def test_resource_environment_uses_declared_cpu_count():
    values = resource_environment(4)
    assert values
    assert set(values.values()) == {"4"}
