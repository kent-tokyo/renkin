#!/usr/bin/env python3
"""Run the four-tool benchmark and merge rows under one frozen manifest.

The tool-native runners remain separate because their resource boundaries
differ, but this command gives the experiment one entry point and one merged
output. Every intermediate file is retained under ``--output-dir``.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from pathlib import Path

from four_tool_record import FourToolRecord, load_records
from validate_four_tool_registry import validate_registry


def sha256_file(path: str | Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run_command(command: list[str]) -> None:
    completed = subprocess.run(command, check=False)
    if completed.returncode != 0:
        raise RuntimeError(f"benchmark command failed with exit code {completed.returncode}: {command[0]}")


def legacy_command(args: argparse.Namespace, tool: str, output: Path) -> list[str]:
    return [
        sys.executable, str(Path(__file__).with_name("compare_run.py")),
        "--sample-list", args.target_manifest, "--sample-size", str(args.sample_size),
        "--tool", tool, "--comparison-mode", args.comparison_mode,
        "--output-rows", str(output), "--repo-root", args.repo_root,
        "--renkin-binary", args.renkin, "--shared-stock-smi", args.stock,
        "--templates", args.templates, "--public-data-dir", args.public_data_dir,
        "--aizynthfinder-image", args.aizynthfinder_image,
        "--timeout-s", str(args.timeout_s), "--grace-s", str(args.grace_s),
    ]


def external_command(args: argparse.Namespace, tool: str, output: Path, artifacts: Path) -> list[str]:
    command = [
        sys.executable, str(Path(__file__).with_name("run_external_targets.py")),
        "--tool", tool, "--target-manifest", args.target_manifest,
        "--sample-size", str(args.sample_size),
        "--output", str(output), "--artifact-dir", str(artifacts),
        "--stock", args.stock, "--renkin", args.renkin, "--arm-id", args.arm_id[tool],
        "--timeout-s", str(args.timeout_s), "--grace-s", str(args.grace_s),
    ]
    if tool == "synplanner":
        command += [
            "--synplan", args.synplan, "--config", args.synplanner_config,
            "--reaction-rules", args.reaction_rules, "--building-blocks", args.building_blocks,
            "--policy-network", args.policy_network, "--value-network", args.value_network,
        ]
    else:
        command += ["--python", args.synth_python, "--model-dir", args.synth_model_dir]
    image = (getattr(args, "synplanner_container_image", None)
             if tool == "synplanner" else getattr(args, "syntheseus_container_image", None))
    if image:
        command += ["--container-image", image, "--repo-root", args.repo_root]
    return command


def merge_rows(paths: list[Path], output: Path) -> int:
    records: list[FourToolRecord] = []
    for path in paths:
        records.extend(load_records(str(path)))
    records.sort(key=lambda row: (row.sample_rank, row.tool, row.arm_id))
    seen: set[tuple[str, str, str]] = set()
    with output.open("w", encoding="utf-8") as handle:
        for record in records:
            key = (record.target_id, record.tool, record.arm_id)
            if key in seen:
                raise ValueError(f"duplicate merged record key: {key!r}")
            seen.add(key)
            handle.write(record.to_json_line() + "\n")
    return len(records)


def load_target_manifest(path: str | Path, sample_size: int) -> list[dict[str, object]]:
    targets = []
    with open(path, encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            target = json.loads(line)
            if not isinstance(target, dict):
                raise ValueError(f"{path}:{line_number}: target must be an object")
            targets.append(target)
    sample = targets[:sample_size]
    ids = [target.get("target_id") for target in sample]
    if len(ids) != len(set(ids)):
        raise ValueError("target manifest sample contains duplicate target_id values")
    if not all(isinstance(target.get("target_id"), str) for target in sample):
        raise ValueError("target manifest sample requires string target_id values")
    return sample


def validate_target_coverage(
    records: list[FourToolRecord], targets: list[dict[str, object]], tools: list[str]
) -> None:
    """Reject missing, extra, or mismatched target rows before report output."""
    expected = {target["target_id"]: target for target in targets}
    if len(expected) != len(targets):
        raise ValueError("target manifest contains duplicate target_id values")
    for tool in tools:
        rows = [record for record in records if record.tool == tool]
        actual = {record.target_id: record for record in rows}
        missing = sorted(set(expected) - set(actual))
        extra = sorted(set(actual) - set(expected))
        if missing or extra or len(rows) != len(actual):
            raise ValueError(
                f"{tool} target coverage mismatch: missing={missing[:3]} extra={extra[:3]}"
            )
        for target_id, record in actual.items():
            target = expected[target_id]
            if record.target_smiles != target.get("canonical_smiles") or record.sample_rank != target.get("sample_rank"):
                raise ValueError(f"{tool} target metadata mismatch for {target_id!r}")


def validate_formal_inputs(args: argparse.Namespace) -> None:
    """Fail closed before starting a formal four-arm run."""
    if not args.registry:
        raise ValueError("--registry is required with --formal")
    if set(args.tools) != {"renkin", "aizynthfinder", "syntheseus", "synplanner"}:
        raise ValueError("--formal requires all four benchmark tools")
    registry_path = Path(args.registry)
    payload = json.loads(registry_path.read_text(encoding="utf-8"))
    identities = None
    if args.image_identities:
        identities = json.loads(Path(args.image_identities).read_text(encoding="utf-8"))
    problems = validate_registry(
        payload, Path(args.repo_root).resolve(), check_artifacts=True,
        formal=True, image_identities=identities,
    )
    if problems:
        raise ValueError("formal registry preflight failed: " + "; ".join(problems))
    expected_count = payload["common"]["formal_target_count"]
    if args.sample_size != expected_count:
        raise ValueError(
            f"formal sample size must equal registry common.formal_target_count ({expected_count}); "
            f"got {args.sample_size}"
        )


def write_run_manifest(args: argparse.Namespace, output: Path, count: int) -> None:
    payload = {
        "schema_version": "renkin-four-tool-run-manifest/1",
        "target_manifest": {"path": str(Path(args.target_manifest).resolve()),
                            "sha256": sha256_file(args.target_manifest),
                            "sample_size": args.sample_size},
        "shared_stock": {"path": str(Path(args.stock).resolve()),
                          "sha256": sha256_file(args.stock)},
        "registry": ({"path": str(Path(args.registry).resolve()),
                      "sha256": sha256_file(args.registry)} if args.registry else None),
        "comparison_mode": args.comparison_mode,
        "tools": args.tools,
        "merged_output": str(Path(args.merged_output).resolve()),
        "n_records": count,
    }
    output.write_text(json.dumps(payload, indent=2, ensure_ascii=False) + "\n", encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target-manifest", required=True)
    parser.add_argument("--sample-size", type=int, required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--merged-output", required=True)
    parser.add_argument("--registry")
    parser.add_argument("--formal", action="store_true",
                        help="run only after the verified four-arm formal preflight")
    parser.add_argument("--image-identities",
                        help="JSON emitted by verify_four_tool_images.py; required with --formal")
    parser.add_argument("--tools", nargs="+", choices=["renkin", "aizynthfinder", "syntheseus", "synplanner"],
                        default=["renkin", "aizynthfinder", "syntheseus", "synplanner"])
    parser.add_argument("--comparison-mode", choices=["native", "shared_stock"], default="shared_stock")
    parser.add_argument("--repo-root", default=".")
    parser.add_argument("--renkin", required=True)
    parser.add_argument("--stock", required=True)
    parser.add_argument("--templates", default="data/templates_extracted_500.smi")
    parser.add_argument("--public-data-dir", default="data/comparison/aizynthfinder_public_data")
    parser.add_argument("--aizynthfinder-image", default="renkin-compare-66/aizynthfinder:4.4.1")
    parser.add_argument("--syntheseus-container-image")
    parser.add_argument("--synplanner-container-image")
    parser.add_argument("--timeout-s", type=float, default=150)
    parser.add_argument("--grace-s", type=float, default=10)
    parser.add_argument("--arm-id", nargs=2, action="append", metavar=("TOOL", "ARM"), default=[])
    parser.add_argument("--synth-python", default=sys.executable)
    parser.add_argument("--synth-model-dir", default="")
    parser.add_argument("--synplan", default="")
    parser.add_argument("--synplanner-config", default="")
    parser.add_argument("--reaction-rules", default="")
    parser.add_argument("--building-blocks", default="")
    parser.add_argument("--policy-network", default="")
    parser.add_argument("--value-network", default="")
    args = parser.parse_args()
    args.arm_id = dict(args.arm_id)
    if args.formal:
        try:
            validate_formal_inputs(args)
        except (OSError, ValueError, json.JSONDecodeError) as exc:
            parser.error(str(exc))
    defaults = {
        "syntheseus": "syntheseus-0.8.0-localretro-retrostar-shared-stock",
        "synplanner": "synplanner-1.6.0-shared-stock",
    }
    for tool in ("syntheseus", "synplanner"):
        args.arm_id.setdefault(tool, defaults[tool])
    if "syntheseus" in args.tools and not args.synth_model_dir:
        parser.error("--synth-model-dir is required for syntheseus")
    if "synplanner" in args.tools and not all([
        args.synplan, args.synplanner_config, args.reaction_rules,
        args.building_blocks, args.policy_network, args.value_network,
    ]):
        parser.error("SynPlanner executable, config, rules, building blocks, policy, and value paths are required")

    output_dir = Path(args.output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)
    row_paths: list[Path] = []
    for tool in args.tools:
        output = output_dir / f"{tool}.jsonl"
        if tool in {"renkin", "aizynthfinder"}:
            run_command(legacy_command(args, tool, output))
            converted = output_dir / f"{tool}.common.jsonl"
            run_command([sys.executable, str(Path(__file__).with_name("convert_existing_rows.py")),
                         str(output), "--output", str(converted)])
            row_paths.append(converted)
        else:
            run_command(external_command(args, tool, output, output_dir / f"{tool}-artifacts"))
            row_paths.append(output)
    count = merge_rows(row_paths, Path(args.merged_output))
    merged_records = load_records(args.merged_output)
    validate_target_coverage(merged_records, load_target_manifest(args.target_manifest, args.sample_size), args.tools)
    write_run_manifest(args, output_dir / "run_manifest.json", count)
    print(json.dumps({"schema_version": "renkin-four-tool-run/1", "n_records": count,
                      "tools": args.tools}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
