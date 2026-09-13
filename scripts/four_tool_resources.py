"""Explicit resource envelope shared by native benchmark runners."""

from __future__ import annotations

import os
import resource
import sys


def resource_environment(cpus: int = 8) -> dict[str, str]:
    value = str(cpus)
    return {name: value for name in (
        "OMP_NUM_THREADS", "MKL_NUM_THREADS", "OPENBLAS_NUM_THREADS",
        "NUMEXPR_NUM_THREADS", "TORCH_NUM_THREADS",
    )}


def cpu_rlimit_seconds(wall_clock_seconds: int, cpus: int) -> int:
    """Return a CPU-time limit that cannot preempt the wall-clock deadline.

    ``RLIMIT_CPU`` charges aggregate CPU time across every worker thread.
    A process allowed to use ``cpus`` threads therefore consumes up to
    ``wall_clock_seconds * cpus`` CPU seconds before its wall-clock deadline.
    Applying the bare wall-clock number here kills parallel native planners
    early (for example, 150 CPU seconds after roughly 20 seconds on 8 cores).
    """
    if wall_clock_seconds <= 0:
        raise ValueError("wall_clock_seconds must be positive")
    if cpus <= 0:
        raise ValueError("cpus must be positive")
    return wall_clock_seconds * cpus


def apply_resource_limits(
    memory_bytes: int = 6 * 1024**3,
    wall_clock_seconds: int = 150,
    cpus: int = 8,
) -> None:
    if os.name != "posix":
        return
    cpu_seconds = cpu_rlimit_seconds(wall_clock_seconds, cpus)
    resource.setrlimit(resource.RLIMIT_CPU, (cpu_seconds, cpu_seconds))
    # macOS exposes RLIMIT_AS but rejects finite values for this process
    # class. Keep CPU limiting active and let the caller's label disclose that
    # the memory ceiling was not enforceable on this host.
    try:
        resource.setrlimit(resource.RLIMIT_AS, (memory_bytes, memory_bytes))
    except (ValueError, OSError):
        pass


def enforcement_label() -> str:
    if os.name != "posix":
        return "thread_env_only_non_posix"
    if sys.platform == "darwin":
        return "posix_rlimit_cpu_plus_thread_env; RLIMIT_AS_unavailable"
    return "posix_rlimit_as_rlimit_cpu_plus_thread_env"
