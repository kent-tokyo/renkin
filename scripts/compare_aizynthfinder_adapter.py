"""AiZynthFinder adapter for the Issue #66 open-source planner comparison.

Wraps the EXISTING, unmodified `aizynthcli` (from the official aizynthfinder
PyPI package) as a per-target Docker container invocation -- never AiZynthFinder's
own source. Python 3.13 (this host's venv) is outside aizynthfinder's supported
range (>=3.10,<3.13), so it runs in a linux/arm64 container (no amd64
emulation needed -- every pinned native dependency ships arm64 wheels for
Python 3.10-3.12); see docker/aizynthfinder.Dockerfile.

One target, one container invocation, one aizynthcli subprocess -- not the
natural aizynthcli batch mode. This is required so the external wall-clock
deadline can be authoritative per target (a batch run can't have one target
killed without killing all of them). AiZynthFinder therefore pays its
policy-model load cost on every target, same as the RENKIN adapter's
per-target CLI invocation -- record via `startup_overhead_ms` calibration
if that cost needs to be subtracted later; not measured in this round.

Peak RSS uses `docker stats` polling (labelled `docker_stats_sampled`) --
coarser than RENKIN's per-target `/usr/bin/time -l` measurement
(`usr_bin_time_v`) and *not* directly comparable to it. This asymmetry is a
disclosed limitation, not a bug: see
docs/guides/open-source-retrosynthesis-comparison.md, "Why RENKIN is not
containerized".
"""

from __future__ import annotations

import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from dataclasses import dataclass

from compare_route_graph import (
    count_leaves,
    graph_depth,
    iter_leaves,
    normalize_aizynthfinder_route,
    normalized_route_sha256,
)
from compare_schema import PlannerComparisonRow
from compare_validation import (
    build_stock_set,
    check_reaction_steps_parseable,
    check_target_element_accounting,
    validate_stock_leaves,
)


@dataclass
class AizynthfinderConfig:
    image: str
    public_data_dir: str  # host dir with config.yml + models + stock, mounted read-only
    config_filename: str = "config.yml"
    stock_name: str = "zinc"
    policy_name: str = "uspto"
    external_timeout_s: float = 150.0
    grace_s: float = 10.0
    cpus: str = "8"
    memory: str = "6g"
    time_limit_s: float = 120.0  # aizynthfinder's own native search budget
    iteration_limit: int = 100


_MEM_USAGE_RE = re.compile(r"([\d.]+)\s*(B|KiB|MiB|GiB)")


def _parse_mem_usage(text: str) -> int | None:
    """Parses docker stats' "12.3MiB / 6GiB" style MemUsage into bytes (first value)."""
    m = _MEM_USAGE_RE.search(text)
    if not m:
        return None
    value, unit = float(m.group(1)), m.group(2)
    multiplier = {"B": 1, "KiB": 1024, "MiB": 1024**2, "GiB": 1024**3}[unit]
    return int(value * multiplier)


def _poll_peak_rss(container_name: str, stop_event: threading.Event, result: dict) -> None:
    peak = 0
    while not stop_event.is_set():
        try:
            out = subprocess.run(
                ["docker", "stats", "--no-stream", "--format", "{{.MemUsage}}", container_name],
                capture_output=True,
                timeout=5,
                text=True,
            )
            if out.returncode == 0 and out.stdout.strip():
                usage = _parse_mem_usage(out.stdout.strip())
                if usage is not None:
                    peak = max(peak, usage)
        except Exception:
            pass
        stop_event.wait(0.3)
    result["peak_rss_bytes"] = peak if peak > 0 else None


def _run_container(
    argv_inside_container: list[str],
    config: AizynthfinderConfig,
    workdir: str,
) -> tuple[str, bytes, bytes, float, int | None, bool]:
    """Runs aizynthcli inside a fresh, network-isolated container.

    Returns (run_status_hint, stdout, stderr, wall_clock_s, peak_rss_bytes, wrapper_killed).
    run_status_hint is "exit_zero" | "exit_nonzero" | "timeout".
    """
    # Docker requires absolute host paths for bind mounts -- a relative
    # path is silently reinterpreted as a named-volume request instead
    # (hard error, not a mount), so resolve both explicitly.
    public_data_dir_abs = os.path.abspath(config.public_data_dir)
    workdir_abs = os.path.abspath(workdir)

    container_name = f"renkin-compare-66-aizynth-{uuid.uuid4().hex[:12]}"
    docker_cmd = [
        "docker",
        "run",
        "--name",
        container_name,
        "--platform",
        "linux/arm64",
        "--network",
        "none",
        "--cpus",
        config.cpus,
        "--memory",
        config.memory,
        "--memory-swap",
        config.memory,
        "-v",
        f"{public_data_dir_abs}:/public:ro",
        "-v",
        f"{workdir_abs}:/work",
        config.image,
    ] + argv_inside_container

    stop_event = threading.Event()
    rss_result: dict = {}
    rss_thread = threading.Thread(
        target=_poll_peak_rss, args=(container_name, stop_event, rss_result), daemon=True
    )
    rss_thread.start()

    start = time.monotonic()
    wrapper_killed = False
    try:
        proc = subprocess.Popen(docker_cmd, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        try:
            stdout, stderr = proc.communicate(timeout=config.external_timeout_s)
            returncode = proc.returncode
        except subprocess.TimeoutExpired:
            subprocess.run(["docker", "kill", container_name], capture_output=True)
            try:
                stdout, stderr = proc.communicate(timeout=config.grace_s)
            except subprocess.TimeoutExpired:
                proc.kill()
                stdout, stderr = proc.communicate()
            returncode = -1
            wrapper_killed = True
        wall_clock_s = time.monotonic() - start
    finally:
        stop_event.set()
        rss_thread.join(timeout=2)
        subprocess.run(["docker", "rm", "-f", container_name], capture_output=True)

    hint = "timeout" if wrapper_killed else ("exit_zero" if returncode == 0 else "exit_nonzero")
    return hint, stdout, stderr, wall_clock_s, rss_result.get("peak_rss_bytes"), wrapper_killed


def run_one_target(
    target_smiles: str,
    target_id: str,
    sample_rank: int,
    config: AizynthfinderConfig,
    comparison_mode: str,
    configuration_id: str,
    tool_version: str,
    configured_stock_smiles: list[str],
) -> PlannerComparisonRow:
    base = dict(
        target_id=target_id,
        target_smiles=target_smiles,
        sample_rank=sample_rank,
        tool="aizynthfinder",
        tool_version=tool_version,
        configuration_id=configuration_id,
        comparison_mode=comparison_mode,
    )

    with tempfile.TemporaryDirectory() as workdir:
        with open(os.path.join(workdir, "target.smi"), "w", encoding="utf-8") as f:
            f.write(target_smiles + "\n")

        argv = [
            "aizynthcli",
            "--smiles",
            "/work/target.smi",
            "--config",
            f"/public/{config.config_filename}",
            "--output",
            "/work/output.json",
        ]

        hint, stdout, stderr, wall_clock_s, peak_rss_bytes, wrapper_killed = _run_container(
            argv, config, workdir
        )
        total_elapsed_ms = wall_clock_s * 1000.0
        rss_measurement_method = "docker_stats_sampled" if peak_rss_bytes is not None else None

        if hint == "timeout":
            return PlannerComparisonRow(
                **base,
                run_status="timeout",
                total_elapsed_ms=total_elapsed_ms,
                peak_rss_bytes=peak_rss_bytes,
                rss_measurement_method=rss_measurement_method,
            )

        if hint == "exit_nonzero":
            return PlannerComparisonRow(
                **base,
                run_status="crashed",
                total_elapsed_ms=total_elapsed_ms,
                peak_rss_bytes=peak_rss_bytes,
                rss_measurement_method=rss_measurement_method,
                adapter_warnings=[
                    {"code": "aizynthcli_nonzero_exit", "detail": stderr.decode(errors="replace")[:2000]}
                ],
            )

        output_path = os.path.join(workdir, "output.json")
        if not os.path.exists(output_path):
            return PlannerComparisonRow(
                **base,
                run_status="invalid_input",
                total_elapsed_ms=total_elapsed_ms,
                peak_rss_bytes=peak_rss_bytes,
                rss_measurement_method=rss_measurement_method,
                adapter_warnings=[
                    {"code": "aizynth_output_missing", "detail": stderr.decode(errors="replace")[:2000]}
                ],
            )

        with open(output_path, "rb") as f:
            raw_bytes = f.read()

        try:
            parsed = json.loads(raw_bytes)
        except json.JSONDecodeError:
            return PlannerComparisonRow(
                **base,
                run_status="invalid_input",
                total_elapsed_ms=total_elapsed_ms,
                peak_rss_bytes=peak_rss_bytes,
                rss_measurement_method=rss_measurement_method,
                adapter_warnings=[{"code": "aizynth_output_not_json", "detail": raw_bytes[:2000].decode(errors="replace")}],
            )

    raw_output_sha256 = hashlib.sha256(raw_bytes).hexdigest()

    # aizynthcli's --output JSON is a pandas `to_json(orient="table")`
    # envelope: {"schema": {...column defs...}, "data": [<one record per
    # target>]} -- NOT a bare records list and NOT a bare per-target dict.
    # Confirmed empirically by inspecting real output (a JSON list or a
    # flat dict would both misparse this silently as "no data", which is
    # exactly the bug this defensive check exists to catch rather than
    # repeat). Each record's own `is_solved` boolean is the tool's actual
    # "solved" signal -- a non-empty `trees` list does NOT imply solved:
    # AiZynthFinder returns its best-effort top-N candidate routes
    # regardless of whether any of them are fully stock-terminating.
    if not (isinstance(parsed, dict) and isinstance(parsed.get("data"), list) and parsed["data"]):
        return PlannerComparisonRow(
            **base,
            run_status="invalid_input",
            total_elapsed_ms=total_elapsed_ms,
            peak_rss_bytes=peak_rss_bytes,
            rss_measurement_method=rss_measurement_method,
            raw_output_sha256=raw_output_sha256,
            adapter_warnings=[{"code": "aizynth_output_unexpected_shape", "detail": str(type(parsed))}],
        )
    record = parsed["data"][0]

    route_found = bool(record.get("is_solved"))
    trees = record.get("trees") or []

    row_kwargs = dict(
        **base,
        run_status="completed",
        route_found=route_found,
        tool_reported_route_count=record.get("number_of_routes") if route_found else None,
        total_elapsed_ms=total_elapsed_ms,
        peak_rss_bytes=peak_rss_bytes,
        rss_measurement_method=rss_measurement_method,
        raw_output_sha256=raw_output_sha256,
        tool_specific={
            "aizynthfinder": {
                "diagnostics_source": "single_per_target_cli_call",
                "time_limit_s": config.time_limit_s,
                "iteration_limit": config.iteration_limit,
                "number_of_solved_routes": record.get("number_of_solved_routes"),
                "number_of_nodes": record.get("number_of_nodes"),
                "tool_reported_search_time_s": record.get("search_time"),
            }
        },
    )

    if not route_found or not trees:
        return PlannerComparisonRow(**row_kwargs)

    # Rank-1 route only, per the fixed route-selection rule (see
    # docs/guides/open-source-retrosynthesis-comparison.md, "Route selection").
    best_tree = trees[0]
    outcome = normalize_aizynthfinder_route(best_tree, target_smiles)
    row_kwargs["route_tree_parseable"] = outcome.parseable
    if not outcome.parseable:
        row_kwargs["common_validation_warnings"] = outcome.defects
        # A route was reported but its own tree doesn't parse -- a concrete,
        # confirmed defect, not merely "couldn't evaluate".
        row_kwargs["validator_confirmed_route_found"] = False
        return PlannerComparisonRow(**row_kwargs)

    graph = outcome.graph
    # aizynthcli's own per-route depth/step-count field names are not
    # reliably identifiable ahead of a real run (Agent E's audit flagged this
    # as unconfirmed) -- harness-derived from the common graph instead of
    # left null, and disclosed as such, rather than chasing an unconfirmed
    # column name. Unlike RENKIN's tool-native `routes[0].depth`, this is a
    # harness computation for AiZynthFinder specifically.
    row_kwargs["best_route_depth"] = graph_depth(graph.root)
    row_kwargs["best_route_step_count"] = graph.step_count_collapsed_edges
    row_kwargs["best_route_leaf_count"] = count_leaves(graph.root)
    row_kwargs["normalized_route_sha256"] = normalized_route_sha256(graph)

    steps_ok, step_warnings = check_reaction_steps_parseable(graph)
    row_kwargs["reaction_steps_parseable"] = steps_ok

    if configured_stock_smiles:
        # Matched-stock mode: the configured stock (RENKIN's 402 compounds)
        # is small enough to canonicalize and independently re-verify, the
        # same way the RENKIN adapter does -- never just trust the tool's
        # own per-leaf claim when an independent check is actually feasible.
        stock_set = build_stock_set(configured_stock_smiles)
        stock_result = validate_stock_leaves(graph, stock_set)
        row_kwargs["all_leaves_in_configured_stock"] = stock_result.all_leaves_in_configured_stock
    else:
        # Native mode: the configured stock is AiZynthFinder's full public
        # ZINC stock (~17.4 million compounds, confirmed via
        # `aizynthcli`'s own startup log) -- loading and canonicalizing
        # that independently for every row is not practical at this scale.
        # Fall back to the tool's own per-leaf `in_stock` claim (already
        # captured as `is_stock_leaf` during normalization) rather than
        # fabricating an independent check this round can't actually
        # afford to run. Disclosed explicitly, not silently assumed.
        row_kwargs["all_leaves_in_configured_stock"] = all(
            leaf.is_stock_leaf for leaf in iter_leaves(graph.root)
        )
        row_kwargs["adapter_warnings"] = list(row_kwargs.get("adapter_warnings") or []) + [
            {
                "code": "native_stock_trusted_not_independently_verified",
                "detail": "all_leaves_in_configured_stock reflects AiZynthFinder's own "
                "per-leaf in_stock claim for native mode (~17.4M-compound ZINC stock, "
                "not independently re-canonicalized this round)",
            }
        ]

    accounting_status, accounting_warnings = check_target_element_accounting(graph)
    row_kwargs["target_element_accounting_status"] = accounting_status

    row_kwargs["common_validation_warnings"] = list(step_warnings) + list(accounting_warnings)

    if accounting_status == "not_evaluable":
        row_kwargs["not_evaluable"] = True
    else:
        row_kwargs["validator_confirmed_route_found"] = (
            steps_ok is True and accounting_status == "accounted"
        )

    return PlannerComparisonRow(**row_kwargs)


if __name__ == "__main__":  # pragma: no cover -- smoke entry point, see compare_run.py
    print("Use compare_run.py to drive the AiZynthFinder adapter over a sample.", file=sys.stderr)
    sys.exit(1)
