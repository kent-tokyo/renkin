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


def apply_resource_limits(memory_bytes: int = 6 * 1024**3, cpu_seconds: int = 150) -> None:
    if os.name != "posix":
        return
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
