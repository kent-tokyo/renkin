#!/usr/bin/env python3
"""Re-verify SynPlanner's exported routes against RENKIN's shared stock."""

from __future__ import annotations

import argparse
import gzip
import json
import subprocess
from pathlib import Path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--results", required=True)
    parser.add_argument("--stock", required=True)
    parser.add_argument("--renkin", default="target/release/renkin")
    parser.add_argument("--output", required=True)
    args = parser.parse_args()

    with gzip.open(args.results, "rt", encoding="utf-8") as handle:
        results = json.load(handle)

    rows = []
    route_dir = Path(args.output).with_suffix("").with_name(
        Path(args.output).stem + "_routes"
    )
    route_dir.mkdir(parents=True, exist_ok=True)
    for index, (target_smiles, routes) in enumerate(results.items()):
        route_path = route_dir / f"{index:04d}.json"
        route_path.write_text(
            json.dumps({str(route_id): route for route_id, route in enumerate(routes)}),
            encoding="utf-8",
        )
        completed = subprocess.run(
            [
                args.renkin,
                "audit-route",
                str(route_path),
                "--format",
                "synplanner",
                "--stock",
                args.stock,
                "--output",
                "json",
            ],
            check=False,
            capture_output=True,
            text=True,
        )
        if completed.returncode != 0:
            raise RuntimeError(
                f"audit-route failed for target {target_smiles!r}: "
                f"{completed.stderr.strip()}"
            )
        report = json.loads(completed.stdout)
        summary = report["summary"]
        rows.append(
            {
                "target_smiles": target_smiles,
                "route_count": len(routes),
                "shared_stock_pass_routes": summary["pass"],
                "shared_stock_fail_routes": summary["fail"],
                "shared_stock_partial_routes": summary["partial"],
                "route_to_shared_stock": summary["pass"] > 0,
            }
        )

    payload = {
        "schema_version": "synplanner-shared-stock-verification/1",
        "results_source": str(Path(args.results)),
        "stock_source": str(Path(args.stock)),
        "n_targets": len(rows),
        "n_route_to_shared_stock": sum(row["route_to_shared_stock"] for row in rows),
        "route_to_shared_stock_rate": sum(
            row["route_to_shared_stock"] for row in rows
        )
        / len(rows),
        "rows": rows,
    }
    Path(args.output).write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: payload[k] for k in payload if k != "rows"}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
