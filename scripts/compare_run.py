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
import os
import sys
import time

import compare_aggregate as aggregate
import compare_manifest as manifest_mod
import compare_renkin_adapter as renkin_adapter
import compare_sampling as sampling
import compare_schema as schema


def load_stock(path: str) -> list[str]:
    stock = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line and not line.startswith("#"):
                stock.append(line.split()[0])
    return stock


def renkin_config_and_id(args):
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
        ring_context_policy=args.ring_context_policy,
        ring_context_sidecar=args.ring_context_sidecar,
    )
    policy_suffix = (
        f"-{args.ring_context_policy}"
        if args.ring_context_policy and args.ring_context_policy != "disabled"
        else "-disabled"
    )
    configuration_id = (
        f"renkin-{args.comparison_mode}-d{args.depth}-b{args.beam_width}{policy_suffix}"
    )
    return config, building_blocks_path, configuration_id


def run_renkin(args, sample: list[dict], skip_ids: set[str]):
    """Yields rows one at a time, skipping targets already present (resume)."""
    config, building_blocks_path, configuration_id = renkin_config_and_id(args)
    tool_version = renkin_adapter.resolve_tool_version(args.repo_root)
    configured_stock = load_stock(building_blocks_path)

    for row_manifest in sample:
        if row_manifest["target_id"] in skip_ids:
            continue
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
        print(
            f"[renkin] rank={row.sample_rank} status={row.run_status} "
            f"route_found={row.route_found}",
            file=sys.stderr,
        )
        yield row


def aizynth_config_and_id(args):
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
    configuration_id = f"aizynthfinder-{args.comparison_mode}"
    return config, configuration_id


def run_aizynthfinder(args, sample: list[dict], skip_ids: set[str]):
    """Yields rows one at a time, skipping targets already present (resume)."""
    import compare_aizynthfinder_adapter as aizynth_adapter

    config, configuration_id = aizynth_config_and_id(args)
    # Native mode's real stock is AiZynthFinder's ~17.4M-compound public
    # ZINC stock, not the shared stock -- passing an empty list signals the
    # adapter to trust the tool's own per-leaf claim instead of an
    # independent re-verification it can't practically run at that scale
    # (see compare_aizynthfinder_adapter.py's native-mode fallback).
    configured_stock = (
        load_stock(args.shared_stock_smi) if args.comparison_mode == "shared_stock" else []
    )

    for row_manifest in sample:
        if row_manifest["target_id"] in skip_ids:
            continue
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
        print(
            f"[aizynthfinder] rank={row.sample_rank} status={row.run_status} "
            f"route_found={row.route_found}",
            file=sys.stderr,
        )
        yield row


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
    parser.add_argument(
        "--ring-context-policy",
        choices=["disabled", "audit-only", "conservative", "ring-only", "element-only"],
        default="disabled",
        help="RENKIN-only ring-context safety guard policy (Issue #72/#242); ignored for --tool aizynthfinder",
    )
    parser.add_argument(
        "--ring-context-sidecar",
        default=None,
        help="Ring-context metadata JSON sidecar; required when --ring-context-policy != disabled",
    )
    parser.add_argument(
        "--resume",
        action="store_true",
        help="Append to --output-rows if it exists, skipping target_ids already present, "
        "flushing each new row immediately. Without this flag, --output-rows is "
        "overwritten from scratch (original behavior).",
    )
    parser.add_argument(
        "--manifest-path",
        default=None,
        help="Write/update a compare_manifest.py run manifest (binary/commit/input hashes, "
        "host environment) alongside this arm's output.",
    )
    args = parser.parse_args(argv)
    if args.ring_context_policy != "disabled" and not args.ring_context_sidecar:
        parser.error("--ring-context-policy != disabled requires --ring-context-sidecar")

    sample = sampling.load_sample(args.sample_list, args.sample_size)
    sample_ids = [row["target_id"] for row in sample]
    if len(set(sample_ids)) != len(sample_ids):
        parser.error("sample list contains duplicate target_id values -- refusing to run")

    existing_rows: list = []
    existing_ids: set[str] = set()
    if args.resume and os.path.exists(args.output_rows):
        existing_rows = schema.load_rows(args.output_rows)
        for row in existing_rows:
            if row.target_id in existing_ids:
                parser.error(
                    f"--output-rows already contains a duplicate target_id {row.target_id!r} "
                    "-- refusing to resume onto a corrupted file"
                )
            existing_ids.add(row.target_id)

    if args.manifest_path:
        building_blocks_path = (
            args.shared_stock_smi if args.comparison_mode == "shared_stock" else args.building_blocks
        )
        input_files = {
            "sample_list": args.sample_list,
            "stock": building_blocks_path,
            "templates": args.templates,
        }
        if args.ring_context_sidecar:
            input_files["ring_context_sidecar"] = args.ring_context_sidecar
        if not os.path.exists(args.manifest_path):
            run_manifest = manifest_mod.capture_start_manifest(
                tool=args.tool,
                comparison_mode=args.comparison_mode,
                ring_context_policy=args.ring_context_policy if args.tool == "renkin" else None,
                command_line=sys.argv,
                repo_root=args.repo_root,
                binary_path=args.renkin_binary if args.tool == "renkin" else None,
                docker_image=args.aizynthfinder_image if args.tool == "aizynthfinder" else None,
                input_files=input_files,
            )
            with open(args.manifest_path, "w", encoding="utf-8") as f:
                json.dump(run_manifest, f, indent=2, sort_keys=True, default=str)
                f.write("\n")

    start = time.monotonic()
    file_mode = "a" if (args.resume and os.path.exists(args.output_rows)) else "w"
    new_row_count = 0
    with open(args.output_rows, file_mode, encoding="utf-8") as f:
        row_gen = (
            run_renkin(args, sample, existing_ids)
            if args.tool == "renkin"
            else run_aizynthfinder(args, sample, existing_ids)
        )
        for row in row_gen:
            if row.target_id in existing_ids:
                parser.error(f"adapter produced duplicate target_id {row.target_id!r}")
            existing_ids.add(row.target_id)
            f.write(row.to_json_line() + "\n")
            f.flush()
            os.fsync(f.fileno())
            new_row_count += 1
    elapsed = time.monotonic() - start

    all_rows = schema.load_rows(args.output_rows)
    agg = aggregate.compute_aggregate(all_rows)
    agg["wall_clock_total_sweep_s"] = elapsed
    agg["new_rows_this_invocation"] = new_row_count
    agg["total_rows_in_file"] = len(all_rows)
    agg["tool"] = args.tool
    agg["comparison_mode"] = args.comparison_mode

    if args.output_aggregate:
        with open(args.output_aggregate, "w", encoding="utf-8") as f:
            json.dump(agg, f, indent=2, sort_keys=True)
            f.write("\n")

    if args.manifest_path and len(all_rows) >= args.sample_size:
        with open(args.manifest_path, "r", encoding="utf-8") as f:
            run_manifest = json.load(f)
        building_blocks_path = (
            args.shared_stock_smi if args.comparison_mode == "shared_stock" else args.building_blocks
        )
        input_files = {
            "sample_list": args.sample_list,
            "stock": building_blocks_path,
            "templates": args.templates,
        }
        if args.ring_context_sidecar:
            input_files["ring_context_sidecar"] = args.ring_context_sidecar
        run_manifest = manifest_mod.finalize_manifest(run_manifest, input_files)
        with open(args.manifest_path, "w", encoding="utf-8") as f:
            json.dump(run_manifest, f, indent=2, sort_keys=True, default=str)
            f.write("\n")

    print(json.dumps(agg, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
