#!/usr/bin/env python3
"""
100-target Disabled/AuditOnly/Conservative/RingOnly/ElementOnly gate for the
ring-context safety guard (Issue #72 / task #242). Reuses the exact
100-target sample and per-target configuration already checked into
data/comparison/results_100/renkin_native.jsonl (depth=5, beam-width=100,
max-routes=1, data/building_blocks.smi, data/templates_extracted_500.smi --
the same corpus the ring-context sidecar was generated from), calling the
`renkin` binary directly (not through the AiZynthFinder-comparison harness,
which this gate has no need for).

For each target, runs the SAME binary at six arms:
  - disabled       (must reproduce the checked-in native-mode measurement)
  - audit-only     (must be byte-identical to disabled by construction)
  - conservative   (both ring-context and element-accounting enforced)
  - conservative_repeat (same policy again, for a determinism check)
  - ring_only      (ring-context enforced, element-accounting audit-only --
                     isolates the ring-context gate's individual effect)
  - element_only   (element-accounting enforced, ring-context audit-only --
                     isolates the element-accounting gate's individual effect)

Usage:
    python3 scripts/ring_context_gate.py --renkin-binary target/release/renkin
"""
import argparse
import hashlib
import json
import subprocess
import sys
import time


def load_sample(path):
    targets = []
    with open(path) as f:
        for line in f:
            row = json.loads(line)
            targets.append(
                {
                    "target_id": row["target_id"],
                    "target_smiles": row["target_smiles"],
                    "sample_rank": row["sample_rank"],
                }
            )
    targets.sort(key=lambda t: t["sample_rank"])
    return targets


def route_signature(parsed):
    """Deterministic signature of the best (rank-1) route's precursor/step
    shape, for cross-policy comparison. Not the project's full
    normalized_route_sha256 (that needs the common comparison-harness route
    DAG machinery) -- sufficient here to detect "same solve state and same
    precursor sets" vs "route changed"."""
    if parsed.get("routes_found", 0) == 0:
        return None
    route = parsed["routes"][0]
    steps = [
        {"target": s.get("target"), "precursors": sorted(s.get("precursors", []))}
        for s in route.get("steps", [])
    ]
    steps.sort(key=lambda s: (s["target"], s["precursors"]))
    blob = json.dumps(steps, sort_keys=True).encode("utf-8")
    return hashlib.sha256(blob).hexdigest()[:16]


def run_one(binary, target_smiles, policy, sidecar, building_blocks, templates, timeout_s):
    argv = [
        binary,
        "--target",
        target_smiles,
        "--depth",
        "5",
        "--beam-width",
        "100",
        "--max-routes",
        "1",
        "--building-blocks",
        building_blocks,
        "--templates",
        templates,
        "--format",
        "json",
        "--verbose",
    ]
    if policy != "disabled":
        argv += ["--ring-context-policy", policy, "--ring-context-sidecar", sidecar]

    t0 = time.monotonic()
    try:
        proc = subprocess.run(argv, capture_output=True, timeout=timeout_s, text=True)
    except subprocess.TimeoutExpired:
        return {"status": "timeout", "elapsed_s": time.monotonic() - t0}
    elapsed_s = time.monotonic() - t0

    if proc.returncode != 0:
        return {"status": "crashed", "elapsed_s": elapsed_s, "stderr": proc.stderr[-2000:]}

    try:
        parsed = json.loads(proc.stdout)
    except json.JSONDecodeError:
        return {"status": "invalid_output", "elapsed_s": elapsed_s, "stdout": proc.stdout[-2000:]}

    ring_diag = None
    for line in proc.stderr.splitlines():
        if line.startswith("[renkin] ring_context_diagnostics: "):
            ring_diag = json.loads(line[len("[renkin] ring_context_diagnostics: ") :])
            break

    return {
        "status": "completed",
        "elapsed_s": elapsed_s,
        "route_found": parsed.get("routes_found", 0) > 0,
        "route_signature": route_signature(parsed),
        "ring_context_diagnostics": ring_diag,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--renkin-binary", required=True)
    ap.add_argument("--sample", default="data/comparison/results_100/renkin_native.jsonl")
    ap.add_argument("--sidecar", default="data/ring_context_metadata_500.json")
    ap.add_argument("--building-blocks", default="data/building_blocks.smi")
    ap.add_argument("--templates", default="data/templates_extracted_500.smi")
    ap.add_argument("--timeout-s", type=float, default=150.0)
    ap.add_argument("--output", default="ring_context_gate_results.json")
    ap.add_argument(
        "--arms",
        default="disabled,audit_only,conservative,conservative_repeat,ring_only,element_only",
        help="Comma-separated subset of arms to run (default: all 6). Useful for a "
        "cheaper confirmatory sweep -- e.g. --arms disabled,conservative,"
        "conservative_repeat for a determinism-only check on a different stock.",
    )
    args = ap.parse_args()

    targets = load_sample(args.sample)
    print(f"Loaded {len(targets)} targets from {args.sample}", flush=True)

    policy_map = {
        "disabled": "disabled",
        "audit_only": "audit-only",
        "conservative": "conservative",
        "conservative_repeat": "conservative",
        "ring_only": "ring-only",
        "element_only": "element-only",
    }
    arms = [a.strip() for a in args.arms.split(",") if a.strip()]
    unknown_arms = [a for a in arms if a not in policy_map]
    if unknown_arms:
        raise SystemExit(f"unknown --arms value(s): {unknown_arms} (valid: {list(policy_map)})")
    results = {arm: {} for arm in arms}

    for i, t in enumerate(targets):
        if i % 10 == 0:
            print(f"  {i}/{len(targets)}...", flush=True)
        for key in arms:
            cli_policy = policy_map[key]
            r = run_one(
                args.renkin_binary,
                t["target_smiles"],
                cli_policy,
                args.sidecar,
                args.building_blocks,
                args.templates,
                args.timeout_s,
            )
            results[key][t["target_id"]] = r

    with open(args.output, "w") as f:
        json.dump(results, f, indent=2, sort_keys=True)
    print(f"Wrote raw results to {args.output}", flush=True)


if __name__ == "__main__":
    main()
