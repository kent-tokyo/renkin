"""RENKIN adapter for the Issue #66 open-source planner comparison.

Wraps the EXISTING, unmodified `renkin` CLI (src/main.rs) as a per-target
subprocess -- never the source, never modified. Deliberately uses ONLY the
main CLI (not a second `renkin-bench` batch join): the main CLI alone gives
route structure, route_found, route count, depth, and step count, which is
everything the common schema needs. `tool_specific.renkin` stays sparse
(whatever the single per-target JSON response already contains) rather than
joining a second batch process -- avoids a whole class of dual-binary
mismatch/ordering failure modes for fields that would end up excluded from
every cross-tool metric anyway.

Peak RSS is measured via `/usr/bin/time -l` (macOS) wrapping each per-target
subprocess individually -- each invocation is a fresh `time` process that
waits on exactly one child, giving an isolated per-target high-water mark
(unlike `getrusage(RUSAGE_CHILDREN)`, which is a running maximum across a
process's entire lifetime and would blur together every consecutive
target). RENKIN runs natively on the host, not in a container -- see
docs/guides/open-source-retrosynthesis-comparison.md, "Why RENKIN is not
containerized" for the comparability limitation this implies for latency/
RSS versus AiZynthFinder.
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import signal
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass

from compare_route_graph import count_leaves, normalize_renkin_route, normalized_route_sha256
from compare_schema import PlannerComparisonRow
from compare_validation import (
    build_stock_set,
    check_reaction_steps_parseable,
    check_target_element_accounting,
    validate_stock_leaves,
)


@dataclass
class RenkinConfig:
    binary_path: str
    building_blocks_path: str
    templates_path: str | None
    depth: int = 5
    beam_width: int = 100
    max_routes: int = 1
    external_timeout_s: float = 150.0
    grace_s: float = 10.0
    # Ring-context safety guard (Issue #72/#242) -- None/"disabled" runs the
    # shipped default (guard off); any other policy also requires a sidecar
    # path and is used for the guard-cost comparison arm, never the primary
    # RENKIN-vs-AiZynthFinder arm (see Issue #66 500-target protocol).
    ring_context_policy: str | None = None
    ring_context_sidecar: str | None = None
    # Issue #101 Task 35: ordering-only LightGBM candidate reranker. Both
    # must be set together (renkin's own CLI already falls back to legacy
    # ordering with a stderr warning if only one is given, or if loading
    # fails -- this adapter doesn't duplicate that validation).
    reranker_model: str | None = None
    reranker_freq_table: str | None = None


_MAXRSS_RE = re.compile(r"^\s*(\d+)\s+maximum resident set size\s*$", re.MULTILINE)
_CPU_TIME_RE = re.compile(
    r"^\s*[\d.]+\s+real\s+([\d.]+)\s+user\s+([\d.]+)\s+sys\s*$", re.MULTILINE
)


def _run_with_time_wrapper(
    argv: list[str], timeout_s: float, grace_s: float
) -> tuple[int | None, bytes, bytes, float, int | None, bool, float | None, float | None]:
    """Runs argv under `/usr/bin/time -l`, enforcing an external wall-clock
    deadline authoritative over anything the tool itself does.

    Returns (returncode, stdout, stderr, wall_clock_s, peak_rss_bytes,
    wrapper_killed, cpu_user_s, cpu_sys_s). CPU times come from the same
    report already parsed for peak_rss -- captured even when the process
    was killed for timing out, since CPU time consumed before a kill is
    still meaningful diagnostic signal (see Phase B.2d, findings.md).
    """
    with tempfile.NamedTemporaryFile(delete=False) as time_out_f:
        time_report_path = time_out_f.name
    try:
        wrapped = ["/usr/bin/time", "-l", "-o", time_report_path, "--"] + argv
        start = time.monotonic()
        proc = subprocess.Popen(
            wrapped,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        wrapper_killed = False
        try:
            stdout, stderr = proc.communicate(timeout=timeout_s)
        except subprocess.TimeoutExpired:
            os.killpg(os.getpgid(proc.pid), signal.SIGTERM)
            try:
                stdout, stderr = proc.communicate(timeout=grace_s)
            except subprocess.TimeoutExpired:
                os.killpg(os.getpgid(proc.pid), signal.SIGKILL)
                stdout, stderr = proc.communicate()
            wrapper_killed = True
        wall_clock_s = time.monotonic() - start

        peak_rss_bytes = None
        cpu_user_s = None
        cpu_sys_s = None
        if os.path.exists(time_report_path):
            with open(time_report_path, "r", encoding="utf-8", errors="replace") as f:
                report_text = f.read()
            m = _MAXRSS_RE.search(report_text)
            if m:
                peak_rss_bytes = int(m.group(1))
            m = _CPU_TIME_RE.search(report_text)
            if m:
                cpu_user_s = float(m.group(1))
                cpu_sys_s = float(m.group(2))

        return (
            proc.returncode,
            stdout,
            stderr,
            wall_clock_s,
            peak_rss_bytes,
            wrapper_killed,
            cpu_user_s,
            cpu_sys_s,
        )
    finally:
        if os.path.exists(time_report_path):
            os.unlink(time_report_path)


def run_one_target(
    target_smiles: str,
    target_id: str,
    sample_rank: int,
    config: RenkinConfig,
    comparison_mode: str,
    configuration_id: str,
    tool_version: str,
    configured_stock_smiles: list[str],
) -> PlannerComparisonRow:
    argv = [
        config.binary_path,
        "--target",
        target_smiles,
        "--depth",
        str(config.depth),
        "--beam-width",
        str(config.beam_width),
        "--max-routes",
        str(config.max_routes),
        "--building-blocks",
        config.building_blocks_path,
        "--format",
        "json",
    ]
    if config.templates_path:
        argv += ["--templates", config.templates_path]
    if config.ring_context_policy and config.ring_context_policy != "disabled":
        argv += ["--ring-context-policy", config.ring_context_policy]
        argv += ["--ring-context-sidecar", config.ring_context_sidecar]
    if config.reranker_model and config.reranker_freq_table:
        argv += ["--reranker-model", config.reranker_model]
        argv += ["--reranker-freq-table", config.reranker_freq_table]

    (
        returncode,
        stdout,
        stderr,
        wall_clock_s,
        peak_rss_bytes,
        wrapper_killed,
        cpu_user_s,
        cpu_sys_s,
    ) = _run_with_time_wrapper(argv, config.external_timeout_s, config.grace_s)
    total_elapsed_ms = wall_clock_s * 1000.0
    cpu_time_tool_specific = {"cpu_user_s": cpu_user_s, "cpu_sys_s": cpu_sys_s}

    base = dict(
        target_id=target_id,
        target_smiles=target_smiles,
        sample_rank=sample_rank,
        tool="renkin",
        tool_version=tool_version,
        configuration_id=configuration_id,
        comparison_mode=comparison_mode,
        rss_measurement_method="usr_bin_time_v" if peak_rss_bytes is not None else None,
    )

    if wrapper_killed:
        return PlannerComparisonRow(
            **base,
            run_status="timeout",
            total_elapsed_ms=total_elapsed_ms,
            peak_rss_bytes=peak_rss_bytes,
            tool_specific={"renkin": cpu_time_tool_specific},
        )

    if returncode != 0:
        return PlannerComparisonRow(
            **base,
            run_status="crashed",
            total_elapsed_ms=total_elapsed_ms,
            peak_rss_bytes=peak_rss_bytes,
            adapter_warnings=[{"code": "renkin_nonzero_exit", "detail": stderr.decode(errors="replace")[:2000]}],
        )

    try:
        parsed = json.loads(stdout)
    except json.JSONDecodeError:
        return PlannerComparisonRow(
            **base,
            run_status="invalid_input",
            total_elapsed_ms=total_elapsed_ms,
            peak_rss_bytes=peak_rss_bytes,
            adapter_warnings=[{"code": "renkin_stdout_not_json", "detail": stdout.decode(errors="replace")[:2000]}],
        )

    raw_output_sha256 = hashlib.sha256(stdout).hexdigest()
    routes_found = parsed.get("routes_found", 0)
    route_found = routes_found > 0

    row_kwargs = dict(
        **base,
        run_status="completed",
        route_found=route_found,
        tool_reported_route_count=routes_found if route_found else None,
        total_elapsed_ms=total_elapsed_ms,
        peak_rss_bytes=peak_rss_bytes,
        raw_output_sha256=raw_output_sha256,
    )

    if not route_found:
        diagnostics = parsed.get("diagnostics", {})
        row_kwargs["tool_specific"] = {
            "renkin": {
                "nodes_expanded": diagnostics.get("nodes_expanded"),
                "max_depth_reached": diagnostics.get("max_depth_reached"),
                "beam_limit_hit": diagnostics.get("beam_limit_hit"),
                "matched_templates": diagnostics.get("matched_templates"),
                "stock_hits": diagnostics.get("stock_hits"),
                "reranker_failures": parsed.get("reranker_failures"),
                "diagnostics_source": "single_per_target_cli_call",
                **cpu_time_tool_specific,
            }
        }
        return PlannerComparisonRow(**row_kwargs)

    # Rank-1 route only, per the fixed route-selection rule (see
    # docs/guides/open-source-retrosynthesis-comparison.md, "Route selection").
    best_route = parsed["routes"][0]
    row_kwargs["best_route_depth"] = best_route.get("depth")
    row_kwargs["best_route_step_count"] = len(best_route.get("steps", []))
    row_kwargs["tool_specific"] = {
        "renkin": {
            "confidence": best_route.get("confidence"),
            "convergency": best_route.get("convergency"),
            "success_probability": best_route.get("success_probability"),
            "route_cost": best_route.get("route_cost"),
            "joint_success_probability": parsed.get("joint_success_probability"),
            "reranker_failures": parsed.get("reranker_failures"),
            "diagnostics_source": "single_per_target_cli_call",
            **cpu_time_tool_specific,
        }
    }

    outcome = normalize_renkin_route(best_route, target_smiles)
    row_kwargs["route_tree_parseable"] = outcome.parseable
    if not outcome.parseable:
        row_kwargs["common_validation_warnings"] = outcome.defects
        return PlannerComparisonRow(**row_kwargs)

    graph = outcome.graph
    row_kwargs["best_route_leaf_count"] = count_leaves(graph.root)
    row_kwargs["normalized_route_sha256"] = normalized_route_sha256(graph)

    steps_ok, step_warnings = check_reaction_steps_parseable(graph)
    row_kwargs["reaction_steps_parseable"] = steps_ok

    stock_set = build_stock_set(configured_stock_smiles)
    stock_result = validate_stock_leaves(graph, stock_set)
    row_kwargs["all_leaves_in_configured_stock"] = stock_result.all_leaves_in_configured_stock

    accounting_status, accounting_warnings = check_target_element_accounting(graph)
    row_kwargs["target_element_accounting_status"] = accounting_status

    row_kwargs["common_validation_warnings"] = list(step_warnings) + list(accounting_warnings)

    return PlannerComparisonRow(**row_kwargs)


def resolve_tool_version(repo_root: str) -> str:
    """Reads the Cargo.toml package version (no --version flag on the CLI)."""
    cargo_toml = os.path.join(repo_root, "Cargo.toml")
    with open(cargo_toml, "r", encoding="utf-8") as f:
        for line in f:
            m = re.match(r'^version\s*=\s*"([^"]+)"', line.strip())
            if m:
                return m.group(1)
    raise RuntimeError(f"could not find version in {cargo_toml}")


if __name__ == "__main__":  # pragma: no cover -- smoke entry point, see compare_run.py
    print("Use compare_run.py to drive the RENKIN adapter over a sample.", file=sys.stderr)
    sys.exit(1)
