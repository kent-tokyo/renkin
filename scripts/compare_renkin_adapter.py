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
    target_element_excess_counts,
    route_edge_snapshot,
    validate_stock_leaves,
)
from four_tool_resources import apply_resource_limits, enforcement_label, resource_environment


@dataclass
class RenkinConfig:
    binary_path: str
    building_blocks_path: str
    templates_path: str | None
    depth: int = 5
    beam_width: int = 100
    bond_index: bool = False
    max_routes: int = 1
    route_selection: str = "rank1"
    external_timeout_s: float = 150.0
    grace_s: float = 10.0
    # Ring-context safety guard (Issue #72/#242) -- None/"disabled" runs the
    # shipped default (guard off); any other policy also requires a sidecar
    # path and is used for the guard-cost comparison arm, never the primary
    # RENKIN-vs-AiZynthFinder arm (see Issue #66 500-target protocol).
    ring_context_policy: str | None = None
    ring_context_sidecar: str | None = None
    # Spectator-bond-loss fail-closed gate (v0.35.0) -- orthogonal to
    # ring_context_policy above; "off" runs the shipped default (gate off).
    spectator_bond_policy: str | None = None
    # Candidate-time directional element-accounting policy. The integrity-
    # triggered retry keeps the off-policy fast path and conditionally reruns
    # with the strict gate; all modes are RENKIN-only comparison arms.
    element_accounting_policy: str = "off"
    # Diversity-reserved beam arm. Kept explicit in the comparison config so
    # measurements cannot accidentally share the default arm's identity.
    beam_diversity_policy: str = "off"
    beam_diversity_slots: int = 0
    template_policy_manifest: str | None = None
    template_policy_artifact: str | None = None
    retro_generator_manifest: str | None = None
    retro_generator_artifact: str | None = None
    retro_generator_slots: int = 0
    # ONNX template policy in ordering-only mode. The binary must be built
    # with the nn-scoring feature; unlike the legacy scorer path this never
    # removes candidates.
    scorer: str | None = None
    # Blend weight for the ONNX ordering-only policy: 1.0 is model-only,
    # 0.0 is the legacy frequency prior. Kept in the arm config so paired
    # runs cannot accidentally reuse an ambiguous configuration id.
    scorer_ordering_blend: float = 1.0
    speed_profile: bool = False
    # O5 named search budget profile. This is forwarded to the existing CLI;
    # the adapter does not duplicate the profile's budget defaults.
    search_profile: str | None = None
    # Issue #101 Task 35: ordering-only LightGBM candidate reranker. Both
    # must be set together (renkin's own CLI already falls back to legacy
    # ordering with a stderr warning if only one is given, or if loading
    # fails -- this adapter doesn't duplicate that validation).
    reranker_model: str | None = None
    reranker_freq_table: str | None = None
    # v0.24 coverage mode (Issue #101, Phase 41.18B) -- "coverage" requires
    # coverage_templates_path; coverage_timeout_secs is optional (None means
    # no cooperative-cancellation deadline on Stage 2). Issue #239 adds
    # opt-in "recovery", where the same coverage asset is an optional final
    # stage after integrity/diversity/depth recovery.
    search_mode: str = "standard"
    coverage_templates_path: str | None = None
    recovery_coverage_tier_paths: tuple[str, ...] = ()
    coverage_timeout_secs: int | None = None
    coverage_beam_width: int | None = None
    recovery_depth: int | None = None
    recovery_beam_width: int | None = None
    recovery_timeout_secs: int | None = None
    recovery_stage_policy: str = "full"


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
            env={**os.environ, **resource_environment()},
            preexec_fn=apply_resource_limits,
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
    if config.bond_index:
        argv += ["--bond-index", "--search-diagnostics"]
    if config.scorer:
        argv += ["--scorer", config.scorer, "--scorer-ordering-only"]
        if config.scorer_ordering_blend != 1.0:
            argv += ["--scorer-ordering-blend", str(config.scorer_ordering_blend)]
    if config.speed_profile:
        argv += ["--speed-profile"]
    if config.search_profile:
        argv += ["--search-profile", config.search_profile]
    if config.ring_context_policy and config.ring_context_policy != "disabled":
        argv += ["--ring-context-policy", config.ring_context_policy]
        argv += ["--ring-context-sidecar", config.ring_context_sidecar]
    if config.spectator_bond_policy and config.spectator_bond_policy != "off":
        # --search-diagnostics is required for the CLI to emit
        # search_diagnostics.spectator_bond_gated_out -- gated_out_* below
        # would otherwise stay null even under a "gated" policy.
        argv += ["--spectator-bond-policy", config.spectator_bond_policy, "--search-diagnostics"]
    if config.element_accounting_policy != "off":
        argv += ["--element-accounting-policy", config.element_accounting_policy]
    if config.beam_diversity_policy != "off":
        argv += ["--beam-diversity-policy", config.beam_diversity_policy]
    if config.beam_diversity_policy != "off" or config.search_mode == "recovery":
        argv += ["--beam-diversity-slots", str(config.beam_diversity_slots)]
    if config.reranker_model and config.reranker_freq_table:
        argv += ["--reranker-model", config.reranker_model]
        argv += ["--reranker-freq-table", config.reranker_freq_table]
    if config.template_policy_manifest and config.template_policy_artifact:
        argv += ["--template-policy-manifest", config.template_policy_manifest]
        argv += ["--template-policy-artifact", config.template_policy_artifact]
    if config.retro_generator_manifest and config.retro_generator_artifact:
        argv += ["--retro-generator-manifest", config.retro_generator_manifest]
        argv += ["--retro-generator-artifact", config.retro_generator_artifact]
        if config.retro_generator_slots:
            argv += ["--retro-generator-slots", str(config.retro_generator_slots)]
    if config.search_mode != "standard":
        argv += ["--search-mode", config.search_mode]
        for path in config.recovery_coverage_tier_paths:
            argv += ["--recovery-coverage-tier", path]
        if config.coverage_templates_path is not None:
            argv += ["--coverage-templates", config.coverage_templates_path]
        if config.coverage_timeout_secs is not None:
            argv += ["--coverage-timeout-secs", str(config.coverage_timeout_secs)]
        if config.coverage_beam_width is not None:
            argv += ["--coverage-beam-width", str(config.coverage_beam_width)]
        if config.recovery_depth is not None:
            argv += ["--recovery-depth", str(config.recovery_depth)]
        if config.recovery_beam_width is not None:
            argv += ["--recovery-beam-width", str(config.recovery_beam_width)]
        if config.recovery_timeout_secs is not None:
            argv += ["--recovery-timeout-secs", str(config.recovery_timeout_secs)]
        if config.recovery_stage_policy != "full":
            argv += ["--recovery-stage-policy", config.recovery_stage_policy]

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
    cpu_time_tool_specific = {
        "cpu_user_s": cpu_user_s,
        "cpu_sys_s": cpu_sys_s,
        "resource_enforcement": enforcement_label(),
    }

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

    # Absent entirely on a standard-mode response (coverage_mode.rs never
    # emits these keys outside coverage mode) -- parsed.get(...) is None
    # for every field here in that case, exactly matching config.search_mode
    # == "standard" and giving every row a consistent tool_specific shape
    # regardless of which mode produced it.
    coverage_mode_fields = {
        "search_mode": parsed.get("search_mode"),
        "selected_stage": parsed.get("selected_stage"),
        "stage2_invoked": parsed.get("stage2_invoked"),
        "stage1_timeout": parsed.get("stage1_timeout"),
        "stage2_timeout": parsed.get("stage2_timeout"),
        "stage1_elapsed_ms": parsed.get("stage1_elapsed_ms"),
        "stage2_elapsed_ms": parsed.get("stage2_elapsed_ms"),
        "element_accounting_retry": parsed.get("element_accounting_retry"),
        "beam_diversity_retry": parsed.get("beam_diversity_retry"),
        "recovery": parsed.get("recovery"),
        "search_profile": parsed.get("search_profile"),
    }

    gated_out_candidate_count = None
    gated_out_reasons = None
    if config.spectator_bond_policy == "gated":
        gated_records = parsed.get("search_diagnostics", {}).get("spectator_bond_gated_out", [])
        gated_out_candidate_count = len(gated_records)
        gated_out_reasons = {}
        for rec in gated_records:
            rule_name = rec.get("rule_name", "unknown")
            gated_out_reasons[rule_name] = gated_out_reasons.get(rule_name, 0) + 1

    row_kwargs = dict(
        **base,
        run_status="completed",
        route_found=route_found,
        tool_reported_route_count=routes_found if route_found else None,
        total_elapsed_ms=total_elapsed_ms,
        peak_rss_bytes=peak_rss_bytes,
        raw_output_sha256=raw_output_sha256,
        gated_out_candidate_count=gated_out_candidate_count,
        gated_out_reasons=gated_out_reasons,
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
                "bond_index_candidates_before_element_filter": parsed.get(
                    "search_diagnostics", {}
                ).get("bond_index_candidates_before_element_filter"),
                "bond_index_candidates_after_element_filter": parsed.get(
                    "search_diagnostics", {}
                ).get("bond_index_candidates_after_element_filter"),
                "bond_index_empty_fallbacks": parsed.get("search_diagnostics", {}).get(
                    "bond_index_empty_fallbacks"
                ),
                "bond_index_no_proposal_fallbacks": parsed.get("search_diagnostics", {}).get(
                    "bond_index_no_proposal_fallbacks"
                ),
                "reranker_failures": parsed.get("reranker_failures"),
                "diagnostics_source": "single_per_target_cli_call",
                **cpu_time_tool_specific,
                **coverage_mode_fields,
            }
        }
        return PlannerComparisonRow(**row_kwargs)

    stock_set = build_stock_set(configured_stock_smiles)
    candidates = parsed["routes"]
    selected_index = 0
    if config.route_selection in {"strict_validated", "strict_on_rank1_failure"}:
        rank1_outcome = normalize_renkin_route(candidates[0], target_smiles)
        rank1_is_strict = False
        if rank1_outcome.parseable and rank1_outcome.graph is not None:
            rank1_steps_ok, _ = check_reaction_steps_parseable(rank1_outcome.graph)
            rank1_accounting, _ = check_target_element_accounting(rank1_outcome.graph)
            rank1_is_strict = rank1_steps_ok is True and rank1_accounting == "accounted"
        if config.route_selection == "strict_on_rank1_failure" and rank1_is_strict:
            candidates = candidates[:1]
        for index, candidate in enumerate(candidates):
            candidate_outcome = normalize_renkin_route(candidate, target_smiles)
            if not candidate_outcome.parseable or candidate_outcome.graph is None:
                continue
            candidate_graph = candidate_outcome.graph
            candidate_steps_ok, _ = check_reaction_steps_parseable(candidate_graph)
            candidate_accounting, _ = check_target_element_accounting(candidate_graph)
            if candidate_steps_ok is True and candidate_accounting == "accounted":
                selected_index = index
                break

    # Rank-1 remains the default and the historical comparison contract.
    # strict_validated is an explicit, separate arm that selects the first
    # candidate passing the same structural validators used below.
    best_route = candidates[selected_index]
    row_kwargs["best_route_depth"] = best_route.get("depth")
    row_kwargs["best_route_step_count"] = len(best_route.get("steps", []))
    row_kwargs["tool_specific"] = {
        "renkin": {
            "confidence": best_route.get("confidence"),
            "convergency": best_route.get("convergency"),
            "success_probability": best_route.get("success_probability"),
            "route_cost": best_route.get("route_cost"),
            "route_selection": config.route_selection,
            "selected_route_index": selected_index,
            "bond_index_candidates_before_element_filter": parsed.get(
                "search_diagnostics", {}
            ).get("bond_index_candidates_before_element_filter"),
            "bond_index_candidates_after_element_filter": parsed.get(
                "search_diagnostics", {}
            ).get("bond_index_candidates_after_element_filter"),
            "bond_index_empty_fallbacks": parsed.get("search_diagnostics", {}).get(
                "bond_index_empty_fallbacks"
            ),
            "bond_index_no_proposal_fallbacks": parsed.get("search_diagnostics", {}).get(
                "bond_index_no_proposal_fallbacks"
            ),
            "joint_success_probability": parsed.get("joint_success_probability"),
            "reranker_failures": parsed.get("reranker_failures"),
            "diagnostics_source": "single_per_target_cli_call",
            **cpu_time_tool_specific,
            **coverage_mode_fields,
        }
    }

    outcome = normalize_renkin_route(best_route, target_smiles)
    row_kwargs["route_tree_parseable"] = outcome.parseable
    if not outcome.parseable:
        row_kwargs["common_validation_warnings"] = outcome.defects
        # A route was reported but its own tree doesn't parse -- a concrete,
        # confirmed defect, not merely "couldn't evaluate".
        row_kwargs["validator_confirmed_route_found"] = False
        return PlannerComparisonRow(**row_kwargs)

    graph = outcome.graph
    row_kwargs["best_route_leaf_count"] = count_leaves(graph.root)
    row_kwargs["normalized_route_sha256"] = normalized_route_sha256(graph)

    steps_ok, step_warnings = check_reaction_steps_parseable(graph)
    row_kwargs["reaction_steps_parseable"] = steps_ok

    stock_result = validate_stock_leaves(graph, stock_set)
    row_kwargs["all_leaves_in_configured_stock"] = stock_result.all_leaves_in_configured_stock

    accounting_status, accounting_warnings = check_target_element_accounting(graph)
    row_kwargs["target_element_accounting_status"] = accounting_status
    row_kwargs["tool_specific"]["renkin"]["target_element_excess_counts"] = (
        target_element_excess_counts(graph)
    )
    row_kwargs["tool_specific"]["renkin"]["route_edge_snapshot"] = route_edge_snapshot(graph)

    row_kwargs["common_validation_warnings"] = list(step_warnings) + list(accounting_warnings)

    if accounting_status == "not_evaluable":
        row_kwargs["not_evaluable"] = True
    else:
        row_kwargs["validator_confirmed_route_found"] = (
            steps_ok is True and accounting_status == "accounted"
        )

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
