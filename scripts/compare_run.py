"""Orchestrator: runs the frozen sample through both adapters, writes
PlannerComparisonRow JSONL + aggregate JSON/Markdown reports.

Usage:
    .venv-compare-66/bin/python scripts/compare_run.py \
        --sample-size 100 --tool renkin --comparison-mode native \
        --output-rows data/comparison/results_100/renkin_native.jsonl

    .venv-compare-66/bin/python scripts/compare_run.py \
        --sample-size 100 --tool aizynthfinder --comparison-mode native \
        --output-rows data/comparison/results_100/aizynthfinder_native.jsonl
"""

from __future__ import annotations

import argparse
import json
import sys
import time

import compare_aggregate as aggregate
import compare_renkin_adapter as renkin_adapter
import compare_sampling as sampling


def load_stock(path: str) -> list[str]:
    stock = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#"):
                stock.append(line.split()[0])
    return stock


def run_renkin(args, sample: list[dict]) -> list:
    building_blocks_path = (
        args.shared_stock_smi if args.comparison_mode == "shared_stock" else args.building_blocks
    )
    config = renkin_adapter.RenkinConfig(
        binary_path=args.renkin_binary,
        building_blocks_path=building_blocks_path,
        templates_path=args.templates,
        depth=args.depth,
        beam_width=args.beam_width,
        max_routes=1,
        external_timeout_s=args.timeout_s,
        grace_s=args.grace_s,
    )
    tool_version = renkin_adapter.resolve_tool_version(args.repo_root)
    configured_stock = load_stock(building_blocks_path)
    configuration_id = f"renkin-{args.comparison_mode}-d{args.depth}-b{args.beam_width}"

    rows = []
    for row_manifest in sample:
        row = renkin_adapter.run_one_target(
            row_manifest["canonical_smiles"],
            row_manifest["target_id"],
            row_manifest["sample_rank"],
            config,
            args.comparison_mode,
            configuration_id,
            tool_version,
            configured_stock,
        )
        rows.append(row)
        print(
            f"[renkin] rank={row.sample_rank} status={row.run_status} "
            f"route_found={row.route_found}",
            file=sys.stderr,
        )
    return rows


def run_aizynthfinder(args, sample: list[dict]) -> list:
    import compare_aizynthfinder_adapter as aizynth_adapter

    config_filename = (
        "config_shared_stock.yml" if args.comparison_mode == "shared_stock" else "config.yml"
    )
    config = aizynth_adapter.AizynthfinderConfig(
        image=args.aizynthfinder_image,
        public_data_dir=args.public_data_dir,
        config_filename=config_filename,
        external_timeout_s=args.timeout_s,
        grace_s=args.grace_s,
    )
    # Native mode's real stock is AiZynthFinder's ~17.4M-compound public
    # ZINC stock, not the shared stock -- passing an empty list signals the
    # adapter to trust the tool's own per-leaf claim instead of an
    # independent re-verification it can't practically run at that scale
    # (see compare_aizynthfinder_adapter.py's native-mode fallback).
    configured_stock = (
        load_stock(args.shared_stock_smi) if args.comparison_mode == "shared_stock" else []
    )
    configuration_id = f"aizynthfinder-{args.comparison_mode}"

    rows = []
    for row_manifest in sample:
        row = aizynth_adapter.run_one_target(
            row_manifest["canonical_smiles"],
            row_manifest["target_id"],
            row_manifest["sample_rank"],
            config,
            args.comparison_mode,
            configuration_id,
            "4.4.1",
            configured_stock,
        )
        rows.append(row)
        print(
            f"[aizynthfinder] rank={row.sample_rank} status={row.run_status} "
            f"route_found={row.route_found}",
            file=sys.stderr,
        )
    return rows


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--sample-list", default="data/comparison/sample_full_sorted.jsonl")
    parser.add_argument("--sample-size", type=int, default=100)
    parser.add_argument("--tool", choices=["renkin", "aizynthfinder"], required=True)
    parser.add_argument("--comparison-mode", choices=["native", "shared_stock"], default="native")
    parser.add_argument("--output-rows", required=True)
    parser.add_argument("--output-aggregate")
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--renkin-binary", default="target/release/renkin")
    parser.add_argument("--building-blocks", default="data/building_blocks.smi")
    parser.add_argument(
        "--shared-stock-smi", default="data/comparison/shared_stock/shared_stock.smi"
    )
    parser.add_argument("--templates", default="data/templates_extracted_500.smi")
    parser.add_argument("--depth", type=int, default=5)
    parser.add_argument("--beam-width", type=int, default=100)
    parser.add_argument("--timeout-s", type=float, default=150.0)
    parser.add_argument("--grace-s", type=float, default=10.0)
    parser.add_argument("--aizynthfinder-image", default="renkin-compare-66/aizynthfinder:4.4.1")
    parser.add_argument(
        "--public-data-dir", default="data/comparison/aizynthfinder_public_data"
    )
    args = parser.parse_args(argv)

    sample = sampling.load_sample(args.sample_list, args.sample_size)

    start = time.monotonic()
    if args.tool == "renkin":
        rows = run_renkin(args, sample)
    else:
        rows = run_aizynthfinder(args, sample)
    elapsed = time.monotonic() - start

    with open(args.output_rows, "w", encoding="utf-8") as f:
        for row in rows:
            f.write(row.to_json_line() + "\n")

    agg = aggregate.compute_aggregate(rows)
    agg["wall_clock_total_sweep_s"] = elapsed
    agg["tool"] = args.tool
    agg["comparison_mode"] = args.comparison_mode

    if args.output_aggregate:
        with open(args.output_aggregate, "w", encoding="utf-8") as f:
            json.dump(agg, f, indent=2, sort_keys=True)
            f.write("\n")

    print(json.dumps(agg, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
