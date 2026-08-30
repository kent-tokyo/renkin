#!/usr/bin/env python3
"""
Phase A.5: aggregate the five per-arm metrics.json files for a given stage
into a summary table and apply the PRE-REGISTERED decision thresholds
(fixed before any results were seen -- see ROADMAP.md/findings.md; this
script does not choose or adjust them).

500->10,000 template zero-positive-rate absolute improvement:
    >=10pp: Phase B strong GO
    5-10pp: Phase B GO (then examine efficiency/dedup for implementation design)
    3-5pp:  ambiguous -- needs Phase C comparison
    <3pp:   simple Phase B template-count scaling rejected -- prioritize Phase C

Also reports the full saturation curve (successive-arm deltas), since the
shape (still improving at 10k vs. plateaued by 2k) matters as much as the
endpoint delta for reading what a residual gap implies.

When produced by v0.49+ pool metrics, each arm also carries the persisted
candidate-pool accounting summary. Older metrics remain readable and expose
``null`` for that optional field.

Usage:
    python3 scripts/phase_a5_report.py --stage-dir data/phase_a5_template_scaling/full_val
"""
import argparse
import json
from pathlib import Path

ARMS = ["500", "1000", "2000", "5000", "10000"]


def verdict(delta_pp):
    if delta_pp >= 10:
        return "Phase B strong GO"
    if delta_pp >= 5:
        return "Phase B GO (check efficiency/dedup for implementation design)"
    if delta_pp >= 3:
        return "ambiguous -- needs Phase C comparison"
    return "simple Phase B template-count scaling rejected -- prioritize Phase C"


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--stage-dir", required=True)
    ap.add_argument("--output", help="defaults to <stage-dir>/summary.json")
    args = ap.parse_args()

    stage_dir = Path(args.stage_dir)
    templates_dir = Path("data/phase_a5_template_scaling/templates")
    arm_metrics = {}
    for arm in ARMS:
        path = stage_dir / f"{arm}_metrics.json"
        if not path.exists():
            raise FileNotFoundError(f"missing {path} -- all 5 arms must be run before reporting")
        arm_metrics[arm] = json.loads(path.read_text())

    def n_templates_actual(arm):
        lines = (templates_dir / f"templates_{arm}.smi").read_text().splitlines()
        return sum(1 for l in lines if l.strip() and not l.startswith("#"))

    rows = []
    for arm in ARMS:
        m = arm_metrics[arm]
        pgs = m["pool_gen_summary"]
        rows.append({
            "arm": arm,
            "n_templates_actual": n_templates_actual(arm),
            "group_count": m["group_count"],
            "zero_positive_rate": m["zero_positive_rate"],
            "positive_present_rate": m["positive_present_rate"],
            "ground_truth_precursor_recall_target_level": m["ground_truth_precursor_recall_target_level"],
            "dedup_rate": m["dedup_rate"],
            "candidate_pool_accounting": m.get("candidate_pool_accounting"),
            "candidates_per_group_p50": pgs["candidates_per_group_p50"],
            "candidates_per_group_p95": pgs["candidates_per_group_p95"],
            "n_candidate_rows": pgs["n_candidate_rows"],
            "wall_clock_seconds": pgs["wall_clock_seconds"],
            "n_groups_zero_candidates": pgs["n_groups_zero_candidates"],
            "n_groups_target_id_mismatch": pgs["n_groups_target_id_mismatch"],
            "n_groups_parse_failed": pgs["n_groups_parse_failed"],
        })

    zp_500 = arm_metrics["500"]["zero_positive_rate"]
    zp_10k = arm_metrics["10000"]["zero_positive_rate"]
    delta_pp = (zp_500 - zp_10k) * 100

    saturation_curve = []
    prev = None
    for arm in ARMS:
        zp = arm_metrics[arm]["zero_positive_rate"]
        delta_from_prev_pp = None if prev is None else (prev - zp) * 100
        saturation_curve.append({"arm": arm, "zero_positive_rate": zp, "delta_from_prev_arm_pp": delta_from_prev_pp})
        prev = zp

    result = {
        "_purpose": (
            "Phase A.5: does increasing TRAIN-derived template count "
            "(500->10,000, nested by construction) reduce the one-step "
            "candidate-pool zero-positive rate on VAL? Formal TEST not used "
            "-- see findings.md."
        ),
        "primary_metric": {
            "zero_positive_rate_500": zp_500,
            "zero_positive_rate_10000": zp_10k,
            "absolute_improvement_pp": delta_pp,
            "verdict": verdict(delta_pp),
        },
        "saturation_curve": saturation_curve,
        "arms": rows,
    }

    out_path = Path(args.output) if args.output else stage_dir / "summary.json"
    out_path.write_text(json.dumps(result, indent=2))
    print(json.dumps(result, indent=2))
    print(f"\nWrote {out_path}")


if __name__ == "__main__":
    main()
