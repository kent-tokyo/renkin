#!/usr/bin/env python3
"""Non-TEST implementation-compatibility check for the v0.24 coverage-mode
formal-TEST protocol's Arm C implementation decision (see
`data/coverage_mode_formal_test/protocol.md` section 3): confirms the
actual shipped `--search-mode coverage` CLI reproduces what the earlier
VAL reranker-compatibility gate's `compare_run.py` two-phase orchestration
already measured, over a handful of already-committed VAL targets --
never any formal-TEST target.

This is implementation-equivalence verification, not a new efficacy
measurement: it introduces no new pass/fail threshold, and its target
set is deliberately small (9 targets, hand-picked from the committed VAL
gate rows for fast completion) rather than a statistical sample -- the
question is "does the code path change the answer," not "how big is the
effect."

Checks, per the user's pre-registration:
  - same Stage-1 solved/unsolved partition (targets picked from Arm A's
    committed `route_found` are re-checked against the native CLI's
    `selected_stage`)
  - same semantic selected-route projection (`normalized_route_sha256`,
    same normalization function the original committed rows were scored
    with -- see scripts/compare_route_graph.py)
  - same Stage-2 invocation behavior (`stage2_invoked` matches which arm
    a target was drawn from)
  - same coverage outcome (route found or not) for non-timeout targets
    (all 6 Stage-2 targets here are drawn from Arm C rows with
    run_status == "completed", i.e. did not hit the 600s timeout)

Usage:
    target/release/renkin must already be built (release mode -- reranker
    + beam-100 search is slow in debug).
    python3 scripts/verify_coverage_mode_cli_matches_val_gate.py
"""

from __future__ import annotations

import json
import os
import subprocess
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
sys.path.insert(0, os.path.join(REPO_ROOT, "scripts"))

from compare_route_graph import normalize_renkin_route, normalized_route_sha256  # noqa: E402

RENKIN_BIN = os.path.join(REPO_ROOT, "target/release/renkin")
TEMPLATES_500 = os.path.join(
    REPO_ROOT, "data/phase_a5_template_scaling/templates/templates_500.smi"
)
TEMPLATES_2000 = os.path.join(
    REPO_ROOT, "data/phase_a5_template_scaling/templates/templates_2000.smi"
)
STOCK = os.path.join(REPO_ROOT, "data/comparison/shared_stock/shared_stock.smi")
RERANKER_MODEL = os.path.join(REPO_ROOT, "data/phase3e_reranker_training/model.txt")
RERANKER_FREQ_TABLE = os.path.join(
    REPO_ROOT, "data/phase3e_reranker_training/frequency_table.json"
)

# Hand-picked from the committed VAL reranker-compatibility gate rows
# (data/phase_b1_frontier/phase_b2/reranker_gate/), fastest-first within
# each category, to keep this check cheap. "expected_sha256" is the
# original run's own normalized_route_sha256 (None where no route was
# found) -- the ground truth this script checks the new CLI path against.
TARGETS = [
    # Stage 1 (Arm A, 500 templates + reranker) solved -- native CLI must
    # solve at Stage 1 too, never invoke Stage 2, same route.
    {
        "target_id": "uspto50k_val#L2832",
        "smiles": "c1c(C(C)=O)c(Cl)ccc1Br",
        "category": "stage1_solved",
        "expected_route_found": True,
        "expected_sha256": "1b7496f2f6ad3da0a486e2fbbe96aada4b43d2c03de714bdffd31a5c489bd190",
    },
    {
        "target_id": "uspto50k_val#L1977",
        "smiles": "C(C)(C)COc1ccccc1C(O)=O",
        "category": "stage1_solved",
        "expected_route_found": True,
        "expected_sha256": "d43ad3d7db375f0605b025ff2d875e45d59a876b20d27185f8c10724147f3e15",
    },
    {
        "target_id": "uspto50k_val#L4062",
        "smiles": "O=C(NS(C)(=O)=O)c1cc(N)ccc1",
        "category": "stage1_solved",
        "expected_route_found": True,
        "expected_sha256": "65d1c22dfa1726a79c694680e5209ff6a0a24b6115e988921e24df4d4e323e3e",
    },
    # Stage 2 (Arm C, escalated to 2,000 templates), solved, run_status ==
    # "completed" (no timeout) in the original gate.
    {
        "target_id": "uspto50k_val#L298",
        "smiles": "C[Si](c2cccc(c2)C=Cc1ccc(cc1)C(=O)O)(C)C",
        "category": "stage2_solved",
        "expected_route_found": True,
        "expected_sha256": "5b666e2605e733895dd2b0147dd9aa333cbd28ec8d3e4e99323d8b4e977c80f2",
    },
    {
        "target_id": "uspto50k_val#L2979",
        "smiles": "N4(CCNCC4)c1nn2c(-c3cccnc3)ccnc2n1",
        "category": "stage2_solved",
        "expected_route_found": True,
        "expected_sha256": "7edc91ed26b9e6b0d75b878c06adf79953d4050a89fcb04483e3c3943b2a60cc",
    },
    {
        "target_id": "uspto50k_val#L1920",
        "smiles": "c2cccc(c2)CC(C(=O)N[C@@H](CSCCc1ccccc1)C(OCC)=O)CSC(C)=O",
        "category": "stage2_solved",
        "expected_route_found": True,
        "expected_sha256": "f3350d6ad8e574c9a0d14fa19ca48fee9d5c04d200d577f2d060c70c2bf4777f",
    },
    # Stage 2, unsolved, run_status == "completed" (no timeout) -- Stage 2
    # must still be invoked (Stage 1 didn't solve it), but finds no route.
    {
        "target_id": "uspto50k_val#L3439",
        "smiles": "Clc1cnc(NN)c(c1)Cl",
        "category": "stage2_unsolved",
        "expected_route_found": False,
        "expected_sha256": None,
    },
    {
        "target_id": "uspto50k_val#L4239",
        "smiles": "c12c([nH]c(CC(=O)N(C)C)c2)nccc1Br",
        "category": "stage2_unsolved",
        "expected_route_found": False,
        "expected_sha256": None,
    },
    {
        "target_id": "uspto50k_val#L3542",
        "smiles": "C(F)(F)(F)CN1CCOc2c1ccc(c2)[N+](=O)[O-]",
        "category": "stage2_unsolved",
        "expected_route_found": False,
        "expected_sha256": None,
    },
]


def run_renkin_coverage(target_smiles: str) -> dict:
    argv = [
        RENKIN_BIN,
        "--target",
        target_smiles,
        "--depth",
        "5",
        "--max-routes",
        "1",
        "--beam-width",
        "100",
        "--building-blocks",
        STOCK,
        "--templates",
        TEMPLATES_500,
        "--search-mode",
        "coverage",
        "--coverage-templates",
        TEMPLATES_2000,
        "--coverage-timeout-secs",
        "600",
        "--reranker-model",
        RERANKER_MODEL,
        "--reranker-freq-table",
        RERANKER_FREQ_TABLE,
        "--format",
        "json",
    ]
    result = subprocess.run(argv, capture_output=True, text=True, timeout=650)
    if result.returncode != 0:
        raise RuntimeError(
            f"renkin exited {result.returncode} for {target_smiles!r}: {result.stderr}"
        )
    return json.loads(result.stdout)


def check_target(spec: dict) -> list[str]:
    """Returns a list of failure descriptions (empty == pass)."""
    failures = []
    output = run_renkin_coverage(spec["smiles"])

    expected_stage = "stage1" if spec["category"] == "stage1_solved" else "stage2"
    expected_stage2_invoked = spec["category"] != "stage1_solved"

    if output.get("search_mode") != "coverage":
        failures.append(f"search_mode={output.get('search_mode')!r}, expected 'coverage'")
    if output.get("selected_stage") != expected_stage:
        failures.append(
            f"selected_stage={output.get('selected_stage')!r}, expected {expected_stage!r}"
        )
    if output.get("stage2_invoked") != expected_stage2_invoked:
        failures.append(
            f"stage2_invoked={output.get('stage2_invoked')!r}, "
            f"expected {expected_stage2_invoked!r}"
        )
    if bool(output.get("routes_found", 0) > 0) != spec["expected_route_found"]:
        failures.append(
            f"routes_found={output.get('routes_found')!r}, "
            f"expected route_found={spec['expected_route_found']!r}"
        )

    routes = output.get("routes") or []
    if spec["expected_route_found"]:
        if not routes:
            failures.append("expected a route in output.routes, got none")
        else:
            outcome = normalize_renkin_route(routes[0], spec["smiles"])
            if not outcome.parseable:
                failures.append(f"new route failed to normalize: {outcome.defects}")
            else:
                actual_sha256 = normalized_route_sha256(outcome.graph)
                if actual_sha256 != spec["expected_sha256"]:
                    failures.append(
                        f"normalized_route_sha256={actual_sha256!r}, "
                        f"expected {spec['expected_sha256']!r} (semantic route mismatch)"
                    )
    else:
        if routes:
            failures.append(f"expected no route, got {len(routes)}")

    return failures


def main() -> int:
    if not os.path.exists(RENKIN_BIN):
        print(
            f"ERROR: {RENKIN_BIN} not found -- build with `cargo build --release` first",
            file=sys.stderr,
        )
        return 2

    all_failures = {}
    for spec in TARGETS:
        print(f"checking {spec['target_id']} ({spec['category']}) ...", flush=True)
        failures = check_target(spec)
        if failures:
            all_failures[spec["target_id"]] = failures
            for f in failures:
                print(f"  FAIL: {f}")
        else:
            print("  ok")

    print()
    if all_failures:
        print(f"FAIL: {len(all_failures)}/{len(TARGETS)} targets mismatched")
        return 1
    print(f"PASS: {len(TARGETS)}/{len(TARGETS)} targets match the VAL gate's committed rows")
    return 0


if __name__ == "__main__":
    sys.exit(main())
