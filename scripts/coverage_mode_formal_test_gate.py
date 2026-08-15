#!/usr/bin/env python3
"""Computes the v0.24 coverage-mode formal-TEST protocol's PASS/FAIL
verdict (`data/coverage_mode_formal_test/protocol.md` §4) from two
already-completed `scripts/compare_run.py` arms.

Does NOT run any search itself -- takes two finished row files (Arm A:
`--search-mode` omitted/standard, 500 templates + reranker; Arm C:
`--search-mode coverage`, 500->2,000 templates + reranker, same cohort)
and reports each of §4's criteria plus the overall verdict. Intended
usage, per the protocol:

    # Arm A (once RELEASE_CANDIDATE_SHA is frozen and the run is authorized)
    python3 scripts/compare_run.py --tool renkin --comparison-mode shared_stock \
        --sample-list data/coverage_mode_formal_test/cohort_manifest.json \
        --sample-size 500 --depth 5 --beam-width 100 \
        --templates data/phase_a5_template_scaling/templates/templates_500.smi \
        --reranker-model data/phase3e_reranker_training/model.txt \
        --reranker-freq-table data/phase3e_reranker_training/frequency_table.json \
        --timeout-s 150 --grace-s 10 --resume \
        --output-rows data/coverage_mode_formal_test/results/arm_a_rows.jsonl \
        --output-aggregate data/coverage_mode_formal_test/results/arm_a_aggregate.json \
        --manifest-path data/coverage_mode_formal_test/results/arm_a_manifest.json

    # Arm C
    python3 scripts/compare_run.py --tool renkin --comparison-mode shared_stock \
        --sample-list data/coverage_mode_formal_test/cohort_manifest.json \
        --sample-size 500 --depth 5 --beam-width 100 \
        --templates data/phase_a5_template_scaling/templates/templates_500.smi \
        --search-mode coverage \
        --coverage-templates data/phase_a5_template_scaling/templates/templates_2000.smi \
        --coverage-timeout-secs 600 \
        --reranker-model data/phase3e_reranker_training/model.txt \
        --reranker-freq-table data/phase3e_reranker_training/frequency_table.json \
        --timeout-s 650 --grace-s 30 --resume \
        --output-rows data/coverage_mode_formal_test/results/arm_c_rows.jsonl \
        --output-aggregate data/coverage_mode_formal_test/results/arm_c_aggregate.json \
        --manifest-path data/coverage_mode_formal_test/results/arm_c_manifest.json

    # Gate verdict
    python3 scripts/coverage_mode_formal_test_gate.py \
        --arm-a-rows data/coverage_mode_formal_test/results/arm_a_rows.jsonl \
        --arm-c-rows data/coverage_mode_formal_test/results/arm_c_rows.jsonl \
        --output data/coverage_mode_formal_test/results/gate_verdict.json

Note on `--sample-list`: `compare_run.py`'s `--sample-list` expects a flat
`{target_id, canonical_smiles, sample_rank}` JSONL, not
`cohort_manifest.json`'s own shape -- run
`scripts/coverage_mode_formal_test_cohort_to_sample_list.py` first to
produce `data/coverage_mode_formal_test/cohort_sample_list.jsonl`, then
pass that as `--sample-list` in both invocations above.

An external wrapper timeout (`run_status == "timeout"`, i.e. the OS-level
`/usr/bin/time` wrapper killed the process) is treated as an EXECUTION
ANOMALY per protocol.md §5, not a normal Stage-2-timeout-classification
event -- it means the cooperative-cancellation deadline itself failed to
return control in time, which should not happen given PR #119's own
`call_returns_promptly_after_deadline_leaves_nothing_running` guarantee.
This script reports such rows separately and does not fold them into the
"Stage 2 timeout rate" operational metric, which is computed from
`stage2_timeout` in RENKIN's own JSON output instead (a completed run
that hit the cooperative deadline, the metric protocol.md §4 actually
means).
"""

from __future__ import annotations

import argparse
import json
import sys


def load_rows(path: str) -> list[dict]:
    rows = []
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                rows.append(json.loads(line))
    return rows


def compute_gate(arm_a_rows: list[dict], arm_c_rows: list[dict]) -> dict:
    arm_a_by_id = {r["target_id"]: r for r in arm_a_rows}
    arm_c_by_id = {r["target_id"]: r for r in arm_c_rows}

    n = len(arm_a_rows)
    missing_in_c = sorted(set(arm_a_by_id) - set(arm_c_by_id))
    missing_in_a = sorted(set(arm_c_by_id) - set(arm_a_by_id))
    common_ids = sorted(set(arm_a_by_id) & set(arm_c_by_id))

    arm_a_anomalies = [r["target_id"] for r in arm_a_rows if r["run_status"] == "timeout"]
    arm_c_anomalies = [r["target_id"] for r in arm_c_rows if r["run_status"] == "timeout"]

    arm_a_solved = {tid for tid in common_ids if arm_a_by_id[tid].get("route_found")}
    arm_c_solved = {tid for tid in common_ids if arm_c_by_id[tid].get("route_found")}

    coverage_delta_pp = (
        (len(arm_c_solved) - len(arm_a_solved)) / n * 100.0 if n else 0.0
    )
    net_gain = len(arm_c_solved) - len(arm_a_solved)

    regressions = sorted(arm_a_solved - arm_c_solved)

    def is_invalid(row: dict) -> bool:
        return row["run_status"] in ("crashed", "invalid_input") or (
            row.get("route_found") and row.get("route_tree_parseable") is False
        )

    invalid = sorted(
        tid
        for tid in common_ids
        if is_invalid(arm_a_by_id[tid]) or is_invalid(arm_c_by_id[tid])
    )

    def reranker_failures_total(rows: list[dict]) -> int:
        return sum(
            (r.get("tool_specific", {}).get("renkin", {}).get("reranker_failures") or 0)
            for r in rows
        )

    reranker_failures_a = reranker_failures_total(arm_a_rows)
    reranker_failures_c = reranker_failures_total(arm_c_rows)

    stage1_semantic_mismatches = []
    stage2_invoked_when_stage1_solved = []
    for tid in sorted(arm_a_solved):
        c_row = arm_c_by_id[tid]
        c_specific = c_row.get("tool_specific", {}).get("renkin", {})
        if c_specific.get("selected_stage") != "stage1" or c_row.get(
            "normalized_route_sha256"
        ) != arm_a_by_id[tid].get("normalized_route_sha256"):
            stage1_semantic_mismatches.append(tid)

    for tid in common_ids:
        c_specific = arm_c_by_id[tid].get("tool_specific", {}).get("renkin", {})
        if c_specific.get("selected_stage") == "stage1" and c_specific.get("stage2_invoked"):
            stage2_invoked_when_stage1_solved.append(tid)

    stage2_invocations = [
        tid
        for tid in common_ids
        if arm_c_by_id[tid].get("tool_specific", {}).get("renkin", {}).get("stage2_invoked")
    ]
    stage2_timeouts = [
        tid
        for tid in stage2_invocations
        if arm_c_by_id[tid].get("tool_specific", {}).get("renkin", {}).get("stage2_timeout")
    ]
    stage2_timeout_rate = (
        len(stage2_timeouts) / len(stage2_invocations) if stage2_invocations else 0.0
    )

    criteria = {
        "coverage_delta_ge_3pp_and_net_gain_ge_15": coverage_delta_pp >= 3.0
        and net_gain >= 15,
        "regressions_zero": len(regressions) == 0,
        "invalid_zero": len(invalid) == 0,
        "reranker_failures_zero_both_arms": reranker_failures_a == 0
        and reranker_failures_c == 0,
        "arm_a_solved_exact_match_in_arm_c_stage1": len(stage1_semantic_mismatches) == 0,
        "stage2_never_invoked_when_stage1_solved": len(stage2_invoked_when_stage1_solved)
        == 0,
        "stage2_timeout_rate_le_5pct": stage2_timeout_rate <= 0.05,
    }
    overall_pass = all(criteria.values())

    return {
        "n_cohort": n,
        "n_common": len(common_ids),
        "cohort_mismatch": {
            "missing_in_arm_c": missing_in_c,
            "missing_in_arm_a": missing_in_a,
        },
        "execution_anomalies": {
            "arm_a_external_timeout": arm_a_anomalies,
            "arm_c_external_timeout": arm_c_anomalies,
        },
        "arm_a_solved": len(arm_a_solved),
        "arm_c_solved": len(arm_c_solved),
        "coverage_delta_pp": coverage_delta_pp,
        "net_gain": net_gain,
        "regressions": regressions,
        "invalid": invalid,
        "reranker_failures_arm_a": reranker_failures_a,
        "reranker_failures_arm_c": reranker_failures_c,
        "stage1_semantic_mismatches": stage1_semantic_mismatches,
        "stage2_invoked_when_stage1_solved": stage2_invoked_when_stage1_solved,
        "stage2_invocation_count": len(stage2_invocations),
        "stage2_timeout_count": len(stage2_timeouts),
        "stage2_timeout_rate": stage2_timeout_rate,
        "criteria": criteria,
        "overall_pass": overall_pass,
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--arm-a-rows", required=True)
    ap.add_argument("--arm-c-rows", required=True)
    ap.add_argument("--output", default=None)
    args = ap.parse_args()

    arm_a_rows = load_rows(args.arm_a_rows)
    arm_c_rows = load_rows(args.arm_c_rows)

    verdict = compute_gate(arm_a_rows, arm_c_rows)

    if args.output:
        with open(args.output, "w", encoding="utf-8") as f:
            json.dump(verdict, f, indent=2, sort_keys=True)
            f.write("\n")

    print(json.dumps(verdict, indent=2, sort_keys=True))

    if verdict["n_common"] != verdict["n_cohort"] or verdict["cohort_mismatch"][
        "missing_in_arm_c"
    ] or verdict["cohort_mismatch"]["missing_in_arm_a"]:
        print(
            "ERROR: arm row sets are not both complete over the cohort -- "
            "not a valid gate evaluation yet (still running, or a target is missing)",
            file=sys.stderr,
        )
        return 2

    if verdict["execution_anomalies"]["arm_a_external_timeout"] or verdict[
        "execution_anomalies"
    ]["arm_c_external_timeout"]:
        print(
            "ANOMALY: at least one row hit the external wrapper timeout "
            "(cooperative cancellation did not return promptly) -- per protocol.md "
            "§5, this is an execution anomaly, not a result. STOP, do not report "
            "a PASS/FAIL verdict from this data, report exactly how far execution "
            "got and wait for authorization before continuing.",
            file=sys.stderr,
        )
        return 3

    print(f"\nVERDICT: {'PASS' if verdict['overall_pass'] else 'FAIL'}", file=sys.stderr)
    return 0 if verdict["overall_pass"] else 1


if __name__ == "__main__":
    sys.exit(main())
