"""Per-arm run manifest for the Issue #66 500-target comparison.

Captures what the environment looked like at arm start/end -- binary/commit/
input-file identity plus host conditions (power, disk, competing CPU load) --
so a run that looks anomalous months later can be checked against what was
actually true when it ran, not just assumed. Informational by default: only
the caller decides whether a captured condition (e.g. battery power) should
block a launch.
"""

from __future__ import annotations

import json
import os
import platform
import re
import subprocess
import sys
import time

from compare_sampling import sha256_file


def _run(argv: list[str]) -> str:
    try:
        out = subprocess.run(argv, capture_output=True, text=True, timeout=10)
        return out.stdout.strip()
    except Exception as e:  # pragma: no cover -- best-effort diagnostics only
        return f"<error: {e}>"


def redact_home_dir(text: str) -> str:
    """Replace the current user's home directory -- wherever it appears,
    including the hyphen-flattened form Claude Code's scratchpad temp
    directories use (e.g. /Users/name -> -Users-name-...) -- with a neutral
    placeholder. This manifest is a committed, shareable diagnostic
    artifact (git_commit/binary_sha256/environment snapshot), not a local
    scratch file; both `command_line` (which can carry a caller-supplied
    absolute path, e.g. a --sample-list under a scratchpad dir) and
    `top_cpu_processes_raw` (macOS `ps -o comm` reports each process's full
    executable path, not just its basename) can otherwise leak a local
    username into it. Derived from the environment at call time --
    deliberately never hardcodes a specific username here.
    """
    home = os.path.expanduser("~")
    if not text or home in ("~", "/"):
        return text
    redacted = text.replace(home, "<redacted-home>")
    home_flat = re.sub(r"[/_]", "-", home.strip("/"))
    if home_flat:
        redacted = re.sub(
            re.escape(home_flat) + r"[\w-]*", "<redacted-scratchpad-session>", redacted
        )
    return redacted


def capture_environment() -> dict:
    battery = _run(["pmset", "-g", "batt"])
    top_procs = [
        redact_home_dir(line)
        for line in _run(["ps", "-Ao", "pid,%cpu,comm", "-r"]).splitlines()[:8]
    ]
    caffeinate = _run(["pgrep", "-fl", "caffeinate"]) or None
    disk = _run(["df", "-h", "."])
    return {
        "os": platform.platform(),
        "cpu_count": os.cpu_count(),
        "ram_bytes": int(_run(["sysctl", "-n", "hw.memsize"]) or 0) or None,
        "power_source": "ac" if "AC Power" in battery else ("battery" if "Battery Power" in battery else "unknown"),
        "battery_raw": battery,
        "disk_free_raw": disk,
        "top_cpu_processes_raw": top_procs,
        "caffeinate_active": caffeinate is not None,
        "caffeinate_raw": caffeinate,
        "load_average": os.getloadavg(),
    }


def docker_image_digest(image: str) -> str | None:
    out = _run(["docker", "inspect", "--format", "{{index .RepoDigests 0}}", image])
    if out.startswith("<error") or not out:
        return None
    return out


def capture_start_manifest(
    *,
    tool: str,
    comparison_mode: str,
    ring_context_policy: str | None,
    spectator_bond_policy: str | None = None,
    command_line: list[str],
    repo_root: str,
    binary_path: str | None,
    docker_image: str | None,
    input_files: dict[str, str],
) -> dict:
    """input_files maps a label (e.g. 'building_blocks', 'templates') to path."""
    git_commit = _run(["git", "-C", repo_root, "rev-parse", "HEAD"])
    manifest = {
        "tool": tool,
        "comparison_mode": comparison_mode,
        "ring_context_policy": ring_context_policy,
        # Orthogonal to ring_context_policy -- v0.35.0's spectator-bond-loss
        # gate (Off/DiagnosticsOnly/Gated) is a separate policy axis from
        # v0.36.0 Phase 1's ring-context guard (Conservative/Disabled) and
        # must never collapse into one "gated" label.
        "spectator_bond_policy": spectator_bond_policy,
        "command_line": [redact_home_dir(arg) for arg in command_line],
        "git_commit": git_commit,
        "binary_sha256": sha256_file(binary_path) if binary_path else None,
        "docker_image": docker_image,
        "docker_image_digest": docker_image_digest(docker_image) if docker_image else None,
        "input_file_sha256": {label: sha256_file(path) for label, path in input_files.items()},
        "start_time_unix": time.time(),
        "start_environment": capture_environment(),
        "end_time_unix": None,
        "end_environment": None,
        "input_file_sha256_at_end": None,
    }
    return manifest


def finalize_manifest(manifest: dict, input_files: dict[str, str]) -> dict:
    manifest["end_time_unix"] = time.time()
    manifest["end_environment"] = capture_environment()
    manifest["input_file_sha256_at_end"] = {
        label: sha256_file(path) for label, path in input_files.items()
    }
    manifest["input_files_unchanged_during_run"] = (
        manifest["input_file_sha256_at_end"] == manifest["input_file_sha256"]
    )
    return manifest


if __name__ == "__main__":  # pragma: no cover -- manual inspection entry point
    print(json.dumps(capture_environment(), indent=2, default=str))
    sys.exit(0)
