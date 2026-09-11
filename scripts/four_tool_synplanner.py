#!/usr/bin/env python3
"""Adapt SynPlanner ``--export_routes`` output to the common benchmark row.

The export is target-SMILES keyed and contains every route found by one
planning run.  The benchmark keeps that count, but audits only the first
(rank-1) route so the primary metric cannot be improved by selecting a
convenient route after the run.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import subprocess
import time
from pathlib import Path
from typing import Any

try:
    from .four_tool_record import FourToolRecord
except ImportError:  # direct script execution
    from four_tool_record import FourToolRecord


def load_target_manifest(path: str | Path) -> dict[str, tuple[str, int]]:
    mapping = {}
    with open(path, encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, 1):
            if not line.strip():
                continue
            row = json.loads(line)
            try:
                mapping[row["canonical_smiles"]] = (row["target_id"], row["sample_rank"])
            except KeyError as exc:
                raise ValueError(f"{path}:{line_number}: missing manifest field {exc}") from exc
    return mapping


def load_export(path: str | Path) -> dict[str, list[dict[str, Any]]]:
    """Load plain JSON or gzip-compressed SynPlanner export results."""
    source = Path(path)
    opener = gzip.open if source.suffix == ".gz" else open
    with opener(source, "rt", encoding="utf-8") as handle:
        payload = json.load(handle)
    if not isinstance(payload, dict):
        raise ValueError("SynPlanner export must be an object keyed by target SMILES")
    for target, routes in payload.items():
        if not isinstance(target, str) or not isinstance(routes, list):
            raise ValueError("each SynPlanner export value must be a route list")
        for route in routes:
            if not isinstance(route, dict) or route.get("type") != "mol":
                raise ValueError("SynPlanner routes must be RouteNode objects")
    return payload


def canonical_sha256(value: Any) -> str:
    encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(encoded).hexdigest()


def audit_rank1_route(
    route: dict[str, Any], *, renkin_binary: str, stock: str, work_dir: Path
) -> dict[str, Any]:
    """Audit exactly one exported route using RENKIN's common bridge."""
    work_dir.mkdir(parents=True, exist_ok=True)
    route_path = work_dir / "rank1-route.json"
    route_path.write_text(json.dumps({"0": route}), encoding="utf-8")
    completed = subprocess.run(
        [renkin_binary, "audit-route", str(route_path), "--format", "synplanner",
         "--stock", stock, "--output", "json"],
        check=False, capture_output=True, text=True,
    )
    if completed.returncode != 0:
        raise RuntimeError(completed.stderr.strip() or "audit-route failed")
    report = json.loads(completed.stdout)
    summary = report["summary"]
    if summary["routes_total"] != 1:
        raise ValueError("rank-1 audit did not produce exactly one route")
    status = "pass" if summary["pass"] else "fail" if summary["fail"] else "partial"
    return {"status": status, "report": report, "route_path": str(route_path)}


def record_for_target(
    *, target_id: str, target_smiles: str, sample_rank: int, arm_id: str,
    routes: list[dict[str, Any]], raw_output_sha256: str | None = None,
    audit: dict[str, Any] | None = None, planning_elapsed_ms: float | None = None,
    resource_enforcement: str | None = None,
) -> FourToolRecord:
    """Build a row while keeping native route count separate from audit status."""
    route = routes[0] if routes else None
    common_status = audit["status"] if audit else None
    return FourToolRecord(
        target_id=target_id, target_smiles=target_smiles, sample_rank=sample_rank,
        tool="synplanner", arm_id=arm_id, run_status="completed", route_found=bool(routes),
        tool_reported_route_count=len(routes),
        common_route_parseable=True if audit is not None else None,
        strict_route_to_shared_stock=(common_status == "pass") if audit else None,
        common_audit_status=common_status,
        planning_elapsed_ms=planning_elapsed_ms, raw_output_sha256=raw_output_sha256,
        route_artifact_sha256=canonical_sha256(route) if route else None,
        tool_specific={"synplanner": {
            "route_export_schema": "synplan-routes/1", "audit_rank": 1,
            **({"resource_enforcement": resource_enforcement} if resource_enforcement else {}),
        }},
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--results", required=True)
    parser.add_argument("--stock", required=True)
    parser.add_argument("--renkin", default="target/release/renkin")
    parser.add_argument("--arm-id", default="synplanner-1.6.0-shared-stock")
    parser.add_argument("--target-manifest", help="frozen target JSONL manifest")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    started = time.perf_counter()
    results = load_export(args.results)
    target_manifest = load_target_manifest(args.target_manifest) if args.target_manifest else {}
    raw_hash = hashlib.sha256(Path(args.results).read_bytes()).hexdigest()
    rows = []
    for rank, (target_smiles, routes) in enumerate(results.items()):
        target_id, sample_rank = target_manifest.get(
            target_smiles, (f"target-{rank:04d}", rank)
        )
        audit = None
        if routes:
            try:
                audit = audit_rank1_route(
                    routes[0], renkin_binary=args.renkin, stock=args.stock,
                    work_dir=Path(args.output).with_suffix("") / f"{rank:04d}",
                )
            except (OSError, RuntimeError, ValueError, json.JSONDecodeError) as exc:
                # A native route exists, but the common post-hoc measurement is
                # not available. Keep the native result and mark the common
                # measurement missing; this is not a strict failure.
                record = record_for_target(
                    target_id=target_id, target_smiles=target_smiles,
                    sample_rank=sample_rank, arm_id=args.arm_id, routes=routes,
                    raw_output_sha256=raw_hash,
                )
                record.warnings.append({"code": "common_audit_unavailable", "detail": str(exc)})
                rows.append(record.to_json_line())
                continue
        record = record_for_target(
            target_id=target_id, target_smiles=target_smiles,
            sample_rank=sample_rank, arm_id=args.arm_id, routes=routes,
            raw_output_sha256=raw_hash, audit=audit,
        )
        rows.append(record.to_json_line())
    Path(args.output).write_text("\n".join(rows) + ("\n" if rows else ""), encoding="utf-8")
    print(json.dumps({"schema_version": "renkin-four-tool-row/1", "n_rows": len(rows),
                      "elapsed_ms": (time.perf_counter() - started) * 1000}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
