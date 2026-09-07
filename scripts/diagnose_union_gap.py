#!/usr/bin/env python3
"""Collect full RENKIN search diagnostics for unclassified union-gap rows."""

from __future__ import annotations

import argparse
import json
import subprocess
from pathlib import Path


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--atlas", required=True)
    p.add_argument("--binary", required=True)
    p.add_argument("--stock", required=True)
    p.add_argument("--templates", required=True)
    p.add_argument("--output", required=True)
    args = p.parse_args()
    atlas = json.load(open(args.atlas, encoding="utf-8"))
    results = []
    for record in atlas["records"]:
        if record["reason"] != "diagnostics_missing":
            continue
        cmd = [
            args.binary,
            "--target",
            record["target_smiles"],
            "--depth",
            "5",
            "--beam-width",
            "100",
            "--max-routes",
            "1",
            "--building-blocks",
            args.stock,
            "--templates",
            args.templates,
            "--search-diagnostics",
            "--format",
            "json",
        ]
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        try:
            payload = json.loads(proc.stdout[proc.stdout.find("{") :])
            diagnostics = payload.get("search_diagnostics", {})
            route = payload.get("routes", [{}])[0]
            status = "depth_zero_stock_route" if route.get("depth") == 0 else "search_no_route"
        except (json.JSONDecodeError, IndexError):
            payload = None
            diagnostics = {}
            status = "runtime_or_output_failure"
        results.append(
            {
                "target_id": record["target_id"],
                "target_smiles": record["target_smiles"],
                "status": status,
                "routes_found": payload.get("routes_found") if payload else None,
                "route_depth": (payload.get("routes", [{}])[0].get("depth") if payload else None),
                "stock_terminal_candidates": diagnostics.get("stock_terminal_candidates"),
                "non_stock_candidates": diagnostics.get("non_stock_candidates"),
                "candidates_generated_before_dedup": diagnostics.get("candidates_generated_before_dedup"),
                "branching_by_depth": diagnostics.get("branching_by_depth"),
                "stderr_tail": proc.stderr[-1000:],
            }
        )
    report = {
        "schema_version": "renkin-union-gap-diagnostics/1",
        "count": len(results),
        "status_counts": {},
        "records": results,
    }
    for result in results:
        report["status_counts"][result["status"]] = report["status_counts"].get(result["status"], 0) + 1
    out = Path(args.output)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"count": report["count"], "status_counts": report["status_counts"]}, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
