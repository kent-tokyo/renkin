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
import hashlib
import os
import platform
import re
import stat
import subprocess
import sys
import time



SECURITY_CONTRACT_VERSION = 1
MAX_MANIFEST_BYTES = 64 * 1024 * 1024


def sha256_file(path: str) -> str:
    """Hash a manifest input only after applying its file boundary policy."""
    metadata = os.lstat(path)
    if stat.S_ISLNK(metadata.st_mode):
        raise ValueError(f"manifest input must not be a symlink: {path!r}")
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"manifest input must be a regular file: {path!r}")
    if metadata.st_size > MAX_MANIFEST_BYTES:
        raise ValueError(
            f"manifest input exceeds {MAX_MANIFEST_BYTES} bytes: {path!r}"
        )
    digest = hashlib.sha256()
    total = 0
    with open(path, "rb") as handle:
        while chunk := handle.read(65536):
            total += len(chunk)
            if total > MAX_MANIFEST_BYTES:
                raise ValueError(
                    f"manifest input exceeds {MAX_MANIFEST_BYTES} bytes: {path!r}"
                )
            digest.update(chunk)
    return digest.hexdigest()
SECURITY_CASES = [
    {
        "security_case_id": "RENKIN-S0-INPUT-001",
        "severity": "medium",
        "surfaces": ["cli", "python", "wasm", "mcp", "library"],
        "scenario": "malformed or adversarial chemical/route input",
        "release_blocker": "panic, silent acceptance, or secret disclosure",
    },
    {
        "security_case_id": "RENKIN-S0-RESOURCE-001",
        "severity": "high",
        "surfaces": ["cli", "python", "wasm", "mcp", "library"],
        "scenario": "resource exhaustion through depth, branching, or oversized input",
        "release_blocker": "unbounded work or an unclassified timeout",
    },
    {
        "security_case_id": "RENKIN-S0-DATA-001",
        "severity": "high",
        "surfaces": ["cli", "python", "mcp", "ci"],
        "scenario": "path traversal, swapped local bundle, or changed input artifact",
        "release_blocker": "input hash mismatch or unverified bundle provenance",
    },
    {
        "security_case_id": "RENKIN-S0-PROTOCOL-001",
        "severity": "high",
        "surfaces": ["mcp"],
        "scenario": "malformed, unknown, duplicate, or out-of-order JSON-RPC request",
        "release_blocker": "crash, silent drop, or non-fail-closed response",
    },
]


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


def validate_security_contract(manifest: dict) -> None:
    """Fail closed when a comparison manifest loses its S0 contract.

    This is intentionally a structural check, not a judgment that the run
    itself was safe. The latter depends on the recorded hashes and outcomes;
    this guard only prevents incomplete security metadata from being treated
    as release evidence.
    """
    contract = manifest.get("security_contract")
    if not isinstance(contract, dict):
        raise ValueError("manifest is missing security_contract")
    if contract.get("version") != SECURITY_CONTRACT_VERSION:
        raise ValueError("manifest has an unsupported security_contract version")
    if not isinstance(contract.get("trusted_boundary"), str) or not contract["trusted_boundary"]:
        raise ValueError("security_contract.trusted_boundary must be a non-empty string")
    if not isinstance(contract.get("resource_budget"), dict):
        raise ValueError("security_contract.resource_budget must be an object")
    cases = contract.get("threat_cases")
    if not isinstance(cases, list) or not cases:
        raise ValueError("security_contract.threat_cases must be a non-empty list")
    case_ids: set[str] = set()
    for case in cases:
        if not isinstance(case, dict):
            raise ValueError("security_contract.threat_cases contains a non-object")
        case_id = case.get("security_case_id")
        if not isinstance(case_id, str) or not case_id or case_id in case_ids:
            raise ValueError("security_contract.threat_cases contains an invalid or duplicate id")
        case_ids.add(case_id)
        for field in ("severity", "scenario", "release_blocker"):
            if not isinstance(case.get(field), str) or not case[field]:
                raise ValueError(f"security contract case {case_id!r} is missing {field}")
        if not isinstance(case.get("surfaces"), list) or not case["surfaces"]:
            raise ValueError(f"security contract case {case_id!r} has no surfaces")
    blockers = contract.get("release_blockers")
    if not isinstance(blockers, list) or not blockers or not all(isinstance(item, str) and item for item in blockers):
        raise ValueError("security_contract.release_blockers must be a non-empty string list")


def load_and_validate_manifest(path: str) -> dict:
    """Load a persisted comparison manifest before any resume work starts."""
    try:
        metadata = os.lstat(path)
        if stat.S_ISLNK(metadata.st_mode):
            raise ValueError("comparison manifest must not be a symlink")
        if not stat.S_ISREG(metadata.st_mode):
            raise ValueError("comparison manifest must be a regular file")
        with open(path, "rb") as handle:
            raw = handle.read(MAX_MANIFEST_BYTES + 1)
        if len(raw) > MAX_MANIFEST_BYTES:
            raise ValueError(f"comparison manifest exceeds {MAX_MANIFEST_BYTES} bytes")
        manifest = json.loads(raw.decode("utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as exc:
        raise ValueError(f"cannot load comparison manifest {path!r}: {exc}") from exc
    if not isinstance(manifest, dict):
        raise ValueError("comparison manifest must be a JSON object")
    validate_security_contract(manifest)
    return manifest


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
    resource_budget: dict | None = None,
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
        "security_contract": {
            "version": SECURITY_CONTRACT_VERSION,
            "trusted_boundary": "caller-controlled targets and route data are untrusted; local bundles are identified by hash",
            "resource_budget": resource_budget or {},
            "threat_cases": SECURITY_CASES,
            "release_blockers": [
                "input_files_unchanged_during_run=false",
                "resource budget missing for a bounded comparison run",
                "unclassified timeout or external process error",
            ],
        },
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
    validate_security_contract(manifest)
    return manifest


if __name__ == "__main__":  # pragma: no cover -- manual inspection entry point
    print(json.dumps(capture_environment(), indent=2, default=str))
    sys.exit(0)
