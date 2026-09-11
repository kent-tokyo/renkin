#!/usr/bin/env python3
"""Adapt a Syntheseus ``syntheseus-route-v1`` artifact to a benchmark row."""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
from pathlib import Path
from typing import Any

try:
    from .four_tool_record import FourToolRecord
except ImportError:  # direct script execution
    from four_tool_record import FourToolRecord


def load_route(path: str | Path) -> dict[str, Any]:
    payload = json.loads(Path(path).read_text(encoding="utf-8"))
    if payload.get("source_tool") != "syntheseus" or payload.get("schema_version") != 1:
        raise ValueError("expected syntheseus-route-v1 artifact")
    if not isinstance(payload.get("steps"), list) or not isinstance(payload.get("target"), str):
        raise ValueError("Syntheseus artifact must contain target and steps")
    return payload


def audit_route(route: dict[str, Any], *, renkin_binary: str, stock: str, work_dir: Path) -> dict[str, Any]:
    work_dir.mkdir(parents=True, exist_ok=True)
    route_path = work_dir / "route.json"
    route_path.write_text(json.dumps(route), encoding="utf-8")
    result = subprocess.run(
        [renkin_binary, "audit-route", str(route_path), "--format", "syntheseus",
         "--stock", stock, "--output", "json"],
        check=False, capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(result.stderr.strip() or "audit-route failed")
    report = json.loads(result.stdout)
    summary = report["summary"]
    if summary["routes_total"] != 1:
        raise ValueError("Syntheseus audit did not produce exactly one route")
    status = "pass" if summary["pass"] else "fail" if summary["fail"] else "partial"
    return {"status": status, "report": report}


def record_for_route(
    route: dict[str, Any], *, target_id: str, sample_rank: int, arm_id: str,
    raw_output_sha256: str | None = None, audit: dict[str, Any] | None = None,
) -> FourToolRecord:
    status = audit["status"] if audit else None
    return FourToolRecord(
        target_id=target_id, target_smiles=route["target"], sample_rank=sample_rank,
        tool="syntheseus", arm_id=arm_id, run_status="completed", route_found=True,
        tool_reported_route_count=1, common_route_parseable=True if audit else None,
        strict_route_to_shared_stock=(status == "pass") if audit else None,
        common_audit_status=status, raw_output_sha256=raw_output_sha256,
        route_artifact_sha256=hashlib.sha256(
            json.dumps(route, sort_keys=True, separators=(",", ":")).encode()
        ).hexdigest(),
        tool_specific={"syntheseus": {"export_schema": "syntheseus-route-v1", "audit_rank": 1}},
    )


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--route", required=True)
    parser.add_argument("--stock", required=True)
    parser.add_argument("--renkin", default="target/release/renkin")
    parser.add_argument("--arm-id", default="syntheseus-0.8.0-localretro-retrostar-shared-stock")
    parser.add_argument("--target-id", default="target-0000")
    parser.add_argument("--sample-rank", type=int, default=0)
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    route = load_route(args.route)
    raw_hash = hashlib.sha256(Path(args.route).read_bytes()).hexdigest()
    audit = audit_route(route, renkin_binary=args.renkin, stock=args.stock,
                        work_dir=Path(args.output).with_suffix(""))
    record = record_for_route(route, target_id=args.target_id, sample_rank=args.sample_rank,
                              arm_id=args.arm_id, raw_output_sha256=raw_hash, audit=audit)
    Path(args.output).write_text(record.to_json_line() + "\n", encoding="utf-8")
    print(json.dumps({"schema_version": "renkin-four-tool-row/1", "n_rows": 1}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
