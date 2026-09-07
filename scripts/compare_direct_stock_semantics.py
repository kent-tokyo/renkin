#!/usr/bin/env python3
"""Report primary and direct-purchase-inclusive paired stock metrics.

The primary comparison keeps the historical non-empty-route contract.  The
secondary metric counts a RENKIN depth-0 route as solved when the target is
confirmed to be in the configured stock, matching the ordinary planner
meaning of "buy this directly".
"""

from __future__ import annotations

import argparse
import json
from collections import Counter
from pathlib import Path


def rows(path: str) -> dict[str, dict]:
    with open(path, encoding="utf-8") as handle:
        return {x["target_id"]: x for x in map(json.loads, handle)}


def main() -> int:
    p = argparse.ArgumentParser()
    p.add_argument("--aizynthfinder", required=True)
    p.add_argument("--renkin", required=True)
    p.add_argument("--direct-diagnostics", required=True)
    p.add_argument("--output", required=True)
    args = p.parse_args()
    ai, renkin = rows(args.aizynthfinder), rows(args.renkin)
    diagnostics = json.load(open(args.direct_diagnostics, encoding="utf-8"))
    direct_ids = {
        x["target_id"]
        for x in diagnostics["records"]
        if x["status"] == "depth_zero_stock_route"
    }
    paired = Counter()
    for target_id in sorted(set(ai) & set(renkin)):
        a = bool(ai[target_id].get("all_leaves_in_configured_stock"))
        primary = bool(renkin[target_id].get("validator_confirmed_route_found"))
        secondary = primary or target_id in direct_ids
        paired[(a, primary)] += 1
        paired[(a, secondary)] += 0
    primary_renkin = sum(bool(x.get("validator_confirmed_route_found")) for x in renkin.values())
    secondary_renkin = primary_renkin + len(direct_ids)
    ai_solved = sum(bool(x.get("all_leaves_in_configured_stock")) for x in ai.values())
    report = {
        "schema_version": "direct-stock-semantics-comparison/1",
        "n_targets": len(set(ai) & set(renkin)),
        "aizynthfinder_route_to_configured_stock": ai_solved,
        "renkin_validator_confirmed_nonempty_route": primary_renkin,
        "renkin_secondary_direct_purchase_inclusive": secondary_renkin,
        "renkin_direct_purchase_targets_added": sorted(direct_ids),
        "primary_rates": {
            "aizynthfinder": ai_solved / len(ai),
            "renkin": primary_renkin / len(renkin),
        },
        "secondary_rates": {
            "aizynthfinder": ai_solved / len(ai),
            "renkin": secondary_renkin / len(renkin),
        },
        "interpretation": "Secondary metric is not a replacement for the historical primary metric; it aligns the zero-step direct-purchase meaning across planner contracts.",
    }
    out = Path(args.output)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
