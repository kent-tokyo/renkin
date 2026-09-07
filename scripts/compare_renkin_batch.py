#!/usr/bin/env python3
"""Run RENKIN's shared-process benchmark and emit comparison rows.

This is an O3 throughput arm, not a replacement for the per-target formal
adapter: the process-level run cannot provide isolated per-target RSS and
does not claim formal latency comparability.  It does, however, reuse one
loaded stock/template environment and sends the resulting route through the
same structural validators as the formal adapter.
"""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path

from compare_route_graph import count_leaves, normalize_renkin_route, normalized_route_sha256
from compare_schema import PlannerComparisonRow
from compare_validation import (
    build_stock_set,
    check_reaction_steps_parseable,
    check_target_element_accounting,
    route_edge_snapshot,
    target_element_excess_counts,
    validate_stock_leaves,
)


def load_sample(path: str, size: int) -> list[dict]:
    rows = [json.loads(line) for line in open(path, encoding="utf-8") if line.strip()]
    return rows[:size]


def run(args: argparse.Namespace) -> list[PlannerComparisonRow]:
    samples = load_sample(args.sample_list, args.sample_size)
    if not samples:
        raise ValueError("sample list is empty")
    with tempfile.NamedTemporaryFile(mode="w", suffix=".smi", delete=False) as handle:
        input_path = handle.name
        for sample in samples:
            smiles = sample.get("target_smiles") or sample["canonical_smiles"]
            handle.write(f"{smiles} {sample['target_id']}\n")
    try:
        command = [
            args.renkin_bench,
            "--input",
            input_path,
            "--depth",
            str(args.depth),
            "--beam-width",
            str(args.beam_width),
            "--max-routes",
            str(args.max_routes),
            "--building-blocks",
            args.stock,
            "--templates",
            args.templates,
            "--include-routes",
        ]
        if args.timing_diagnostics:
            command.append("--timing-diagnostics")
        if args.cross_template_dedup:
            command.append("--cross-template-dedup")
        if args.beam_diversity_policy != "off":
            command += ["--beam-diversity-policy", args.beam_diversity_policy]
            command += ["--beam-diversity-slots", str(args.beam_diversity_slots)]
        if args.bond_index:
            command.append("--bond-index")
        if args.search_mode == "recovery":
            command += ["--search-mode", "recovery"]
            if args.recovery_depth is not None:
                command += ["--recovery-depth", str(args.recovery_depth)]
            if args.recovery_beam_width is not None:
                command += ["--recovery-beam-width", str(args.recovery_beam_width)]
            if args.recovery_timeout_secs is not None:
                command += ["--recovery-timeout-secs", str(args.recovery_timeout_secs)]
        completed = subprocess.run(
            command,
            check=True,
            capture_output=True,
            text=True,
            timeout=args.process_timeout_s,
        )
        report = json.loads(completed.stdout)
    finally:
        os.unlink(input_path)

    stock_set = build_stock_set(
        [line.split()[0] for line in open(args.stock, encoding="utf-8") if line.strip()]
    )
    samples_by_id = {sample["target_id"]: sample for sample in samples}
    rows: list[PlannerComparisonRow] = []
    for result in report["results"]:
        target_id = result["name"]
        sample = samples_by_id[target_id]
        route = result.get("best_route")
        base = dict(
            target_id=target_id,
            target_smiles=sample.get("target_smiles") or sample["canonical_smiles"],
            sample_rank=sample.get("sample_rank", 0),
            tool="renkin",
            tool_version=args.tool_version,
            configuration_id=f"renkin-batch-{args.search_mode}-d{args.depth}-b{args.beam_width}-n{args.max_routes}",
            comparison_mode="shared_stock",
            run_status="completed",
            route_found=bool(result["solved"]),
            tool_reported_route_count=result["routes_found"],
            total_elapsed_ms=result["time_ms"],
            peak_rss_bytes=None,
            rss_measurement_method=None,
        )
        if route is None:
            base["tool_specific"] = {
                "renkin": {
                    "batch_process": True,
                    "nodes_expanded": result["nodes_expanded"],
                    "max_depth_reached": result["max_depth_reached"],
                    "beam_limit_hit": result["beam_limit_hit"],
                    "matched_templates": result["matched_templates"],
                    "recovery_audit": result.get("recovery_audit"),
                    "retro_expansion_wall_time_us": result.get("retro_expansion_wall_time_us"),
                    "candidates_generated_before_dedup": result.get("candidates_generated_before_dedup"),
                    "candidates_after_cross_template_dedup": result.get("candidates_after_cross_template_dedup"),
                }
            }
            rows.append(PlannerComparisonRow(**base))
            continue
        target_smiles = sample.get("target_smiles") or sample["canonical_smiles"]
        outcome = normalize_renkin_route(route, target_smiles)
        base["route_tree_parseable"] = outcome.parseable
        if not outcome.parseable or outcome.graph is None:
            base["validator_confirmed_route_found"] = False
            base["common_validation_warnings"] = outcome.defects
            rows.append(PlannerComparisonRow(**base))
            continue
        graph = outcome.graph
        steps_ok, step_warnings = check_reaction_steps_parseable(graph)
        stock_result = validate_stock_leaves(graph, stock_set)
        accounting, accounting_warnings = check_target_element_accounting(graph)
        base.update(
            best_route_depth=route.get("depth"),
            best_route_step_count=len(route.get("steps", [])),
            best_route_leaf_count=count_leaves(graph.root),
            all_leaves_in_configured_stock=stock_result.all_leaves_in_configured_stock,
            reaction_steps_parseable=steps_ok,
            target_element_accounting_status=accounting,
            normalized_route_sha256=normalized_route_sha256(graph),
            common_validation_warnings=list(step_warnings) + list(accounting_warnings),
            tool_specific={
                "renkin": {
                    "batch_process": True,
                    "nodes_expanded": result["nodes_expanded"],
                    "max_depth_reached": result["max_depth_reached"],
                    "beam_limit_hit": result["beam_limit_hit"],
                    "matched_templates": result["matched_templates"],
                    "recovery_audit": result.get("recovery_audit"),
                    "target_element_excess_counts": target_element_excess_counts(graph),
                    "route_edge_snapshot": route_edge_snapshot(graph),
                    "retro_expansion_wall_time_us": result.get("retro_expansion_wall_time_us"),
                    "candidates_generated_before_dedup": result.get("candidates_generated_before_dedup"),
                    "candidates_after_cross_template_dedup": result.get("candidates_after_cross_template_dedup"),
                }
            },
        )
        if accounting == "not_evaluable":
            base["not_evaluable"] = True
        else:
            base["validator_confirmed_route_found"] = steps_ok is True and accounting == "accounted"
        rows.append(PlannerComparisonRow(**base))
    return rows


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--sample-list", required=True)
    parser.add_argument("--sample-size", type=int, default=100)
    parser.add_argument("--renkin-bench", default="target/debug/renkin-bench")
    parser.add_argument("--stock", required=True)
    parser.add_argument("--templates", required=True)
    parser.add_argument("--depth", type=int, default=5)
    parser.add_argument("--beam-width", type=int, default=100)
    parser.add_argument("--max-routes", type=int, default=1)
    parser.add_argument("--bond-index", action="store_true")
    parser.add_argument("--timing-diagnostics", action="store_true")
    parser.add_argument(
        "--cross-template-dedup",
        action="store_true",
        help="Enable experimental same-parent cross-template precursor deduplication.",
    )
    parser.add_argument(
        "--beam-diversity-policy",
        choices=["off", "diagnostics-only", "active", "adaptive"],
        default="off",
    )
    parser.add_argument("--beam-diversity-slots", type=int, default=0)
    parser.add_argument("--search-mode", choices=["standard", "recovery"], default="standard")
    parser.add_argument("--recovery-depth", type=int)
    parser.add_argument("--recovery-beam-width", type=int)
    parser.add_argument("--recovery-timeout-secs", type=int)
    parser.add_argument(
        "--process-timeout-s",
        type=int,
        default=900,
        help="External wall-clock bound for the shared benchmark process.",
    )
    parser.add_argument("--tool-version", default="1.0.3")
    parser.add_argument("--output-rows", required=True)
    args = parser.parse_args()
    rows = run(args)
    output = Path(args.output_rows)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text("".join(row.to_json_line() + "\n" for row in rows), encoding="utf-8")
    print(json.dumps({"rows": len(rows), "route_found": sum(r.route_found is True for r in rows)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
