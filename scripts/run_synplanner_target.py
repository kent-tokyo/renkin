#!/usr/bin/env python3
"""Run one SynPlanner target and emit one common benchmark record.

This is intentionally a one-target process boundary.  It makes timeout and
artifact provenance unambiguous and lets an outer orchestrator resume by
``target_id`` without re-running completed rows.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import time
from pathlib import Path

from four_tool_record import FourToolRecord
from four_tool_resources import apply_resource_limits, enforcement_label, resource_environment
from four_tool_synplanner import audit_rank1_route, load_export, record_for_target


def build_command(args: argparse.Namespace, target_file: Path, results_dir: Path) -> list[str]:
    return [
        args.synplan, "planning", "--config", args.config,
        "--targets", str(target_file), "--reaction_rules", args.reaction_rules,
        "--building_blocks", args.building_blocks, "--policy_network", args.policy_network,
        "--value_network", args.value_network, "--results_dir", str(results_dir),
        "--export_routes",
    ]


def failed_record(args: argparse.Namespace, status: str, reason: str) -> FourToolRecord:
    return FourToolRecord(
        target_id=args.target_id, target_smiles=args.target_smiles,
        sample_rank=args.sample_rank, tool="synplanner", arm_id=args.arm_id,
        run_status=status, failure_reason=reason,
    )


def run(args: argparse.Namespace) -> FourToolRecord:
    output = Path(args.output)
    work = output.with_suffix("")
    work.mkdir(parents=True, exist_ok=True)
    target_file = work / "target.smi"
    target_file.write_text(args.target_smiles + "\n", encoding="utf-8")
    results_dir = work / "synplan-results"
    results_dir.mkdir(exist_ok=True)
    started = time.perf_counter()
    try:
        completed = subprocess.run(
            build_command(args, target_file, results_dir), check=False,
            capture_output=True, text=True, timeout=args.timeout_s + args.grace_s,
        )
    except subprocess.TimeoutExpired as exc:
        return failed_record(args, "timeout", f"synplan exceeded timeout/grace: {exc}")
    elapsed_ms = (time.perf_counter() - started) * 1000
    if completed.returncode != 0:
        detail = completed.stderr.strip() or completed.stdout.strip() or "synplan failed"
        return failed_record(args, "crashed", detail[-2000:])

    results_path = results_dir / "results.json.gz"
    if not results_path.exists():
        return failed_record(args, "parse_error", f"missing export artifact: {results_path}")
    try:
        results = load_export(results_path)
        routes = results.get(args.target_smiles)
        if routes is None and len(results) == 1:
            routes = next(iter(results.values()))
        if routes is None:
            raise ValueError("target SMILES is absent from SynPlanner export")
        audit = None
        warning = None
        if routes:
            try:
                audit = audit_rank1_route(
                    routes[0], renkin_binary=args.renkin, stock=args.stock,
                    work_dir=work / "rank1-audit",
                )
            except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as exc:
                warning = {"code": "common_audit_unavailable", "detail": str(exc)}
        record = record_for_target(
            target_id=args.target_id, target_smiles=args.target_smiles,
            sample_rank=args.sample_rank, arm_id=args.arm_id, routes=routes,
            raw_output_sha256=hashlib.sha256(results_path.read_bytes()).hexdigest(),
            audit=audit, planning_elapsed_ms=elapsed_ms,
            resource_enforcement=enforcement_label(),
        )
        if warning:
            record.warnings.append(warning)
        return record
    except (OSError, ValueError, json.JSONDecodeError) as exc:
        return failed_record(args, "parse_error", str(exc))


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--synplan", required=True)
    parser.add_argument("--config", required=True)
    parser.add_argument("--targets", help="reserved for outer manifest compatibility")
    parser.add_argument("--target-id", required=True)
    parser.add_argument("--target-smiles", required=True)
    parser.add_argument("--sample-rank", type=int, required=True)
    parser.add_argument("--reaction-rules", required=True, dest="reaction_rules")
    parser.add_argument("--building-blocks", required=True, dest="building_blocks")
    parser.add_argument("--policy-network", required=True, dest="policy_network")
    parser.add_argument("--value-network", required=True, dest="value_network")
    parser.add_argument("--stock", required=True)
    parser.add_argument("--renkin", required=True)
    parser.add_argument("--arm-id", default="synplanner-1.6.0-shared-stock")
    parser.add_argument("--timeout-s", type=float, default=150)
    parser.add_argument("--grace-s", type=float, default=10)
    parser.add_argument("--output", required=True)
    parser.add_argument("--cpus", type=int, default=8)
    parser.add_argument("--memory-bytes", type=int, default=6 * 1024**3)
    args = parser.parse_args()
    import os
    os.environ.update(resource_environment(args.cpus))
    apply_resource_limits(args.memory_bytes, int(args.timeout_s + args.grace_s))
    record = run(args)
    Path(args.output).write_text(record.to_json_line() + "\n", encoding="utf-8")
    print(record.to_json_line())
    return 0 if record.run_status == "completed" else 1


if __name__ == "__main__":
    raise SystemExit(main())
