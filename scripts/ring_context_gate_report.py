#!/usr/bin/env python3
"""
Summarize scripts/ring_context_gate.py's raw output into the comparison the
draft PR needs: Disabled vs AuditOnly (must be identical), Disabled vs
Conservative (routes may change), aggregate reject-reason counters, and a
same-process Conservative-vs-Conservative-repeat determinism check.

Usage:
    python3 scripts/ring_context_gate_report.py --input <gate_results.json>
"""
import argparse
import json


def compare_arms(results, a, b):
    ids = sorted(results[a].keys())
    route_found_flips = []
    signature_flips = []
    status_flips = []
    for tid in ids:
        ra, rb = results[a][tid], results[b][tid]
        if ra.get("status") != rb.get("status"):
            status_flips.append((tid, ra.get("status"), rb.get("status")))
            continue
        if ra.get("status") != "completed":
            continue
        if ra.get("route_found") != rb.get("route_found"):
            route_found_flips.append((tid, ra.get("route_found"), rb.get("route_found")))
        elif ra.get("route_signature") != rb.get("route_signature"):
            signature_flips.append((tid, ra.get("route_signature"), rb.get("route_signature")))
    return {
        "status_flips": status_flips,
        "route_found_flips": route_found_flips,
        "route_signature_flips_same_solve_state": signature_flips,
    }


def sum_diagnostics(arm_results):
    total = {}
    for r in arm_results.values():
        d = r.get("ring_context_diagnostics")
        if not d:
            continue
        for k, v in d.items():
            total[k] = total.get(k, 0) + v
    return total


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--input", default="ring_context_gate_results.json")
    args = ap.parse_args()

    with open(args.input) as f:
        results = json.load(f)

    n = len(results["disabled"])
    print(f"=== {n} targets ===\n")

    for arm in (
        "disabled",
        "audit_only",
        "conservative",
        "conservative_repeat",
        "ring_only",
        "element_only",
    ):
        statuses = {}
        for r in results[arm].values():
            statuses[r.get("status")] = statuses.get(r.get("status"), 0) + 1
        solved = sum(1 for r in results[arm].values() if r.get("route_found"))
        print(f"[{arm}] statuses={statuses} route_found={solved}/{n}")
    print()

    print("=== Disabled vs AuditOnly (must be identical) ===")
    d = compare_arms(results, "disabled", "audit_only")
    for k, v in d.items():
        print(f"  {k}: {len(v)}")
        for row in v[:10]:
            print(f"    {row}")
    print()

    print("=== Disabled vs Conservative ===")
    d = compare_arms(results, "disabled", "conservative")
    for k, v in d.items():
        print(f"  {k}: {len(v)}")
        for row in v[:20]:
            print(f"    {row}")
    print()

    print("=== Conservative vs Conservative-repeat (determinism) ===")
    d = compare_arms(results, "conservative", "conservative_repeat")
    for k, v in d.items():
        print(f"  {k}: {len(v)}")
        for row in v[:10]:
            print(f"    {row}")
    print()

    print("=== Disabled vs RingOnly (ring-context gate's isolated effect) ===")
    d = compare_arms(results, "disabled", "ring_only")
    for k, v in d.items():
        print(f"  {k}: {len(v)}")
        for row in v[:20]:
            print(f"    {row}")
    print()

    print("=== Disabled vs ElementOnly (element-accounting gate's isolated effect) ===")
    d = compare_arms(results, "disabled", "element_only")
    for k, v in d.items():
        print(f"  {k}: {len(v)}")
        for row in v[:20]:
            print(f"    {row}")
    print()

    print("=== Aggregate ring_context_diagnostics: AuditOnly ===")
    print(json.dumps(sum_diagnostics(results["audit_only"]), indent=2, sort_keys=True))
    print()
    print("=== Aggregate ring_context_diagnostics: Conservative ===")
    print(json.dumps(sum_diagnostics(results["conservative"]), indent=2, sort_keys=True))
    print()
    print("=== Aggregate ring_context_diagnostics: RingOnly ===")
    print(json.dumps(sum_diagnostics(results["ring_only"]), indent=2, sort_keys=True))
    print()
    print("=== Aggregate ring_context_diagnostics: ElementOnly ===")
    print(json.dumps(sum_diagnostics(results["element_only"]), indent=2, sort_keys=True))
    print()

    print("=== Latency (elapsed_s) percentiles by arm ===")
    for arm in ("disabled", "audit_only", "conservative", "ring_only", "element_only"):
        times = sorted(r["elapsed_s"] for r in results[arm].values() if "elapsed_s" in r)
        if not times:
            continue
        n_t = len(times)
        p50 = times[n_t // 2]
        p95 = times[min(n_t - 1, int(n_t * 0.95))]
        total = sum(times)
        print(f"  {arm}: n={n_t} total={total:.1f}s p50={p50:.2f}s p95={p95:.2f}s max={times[-1]:.2f}s")


if __name__ == "__main__":
    main()
