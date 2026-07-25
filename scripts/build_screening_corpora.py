#!/usr/bin/env python3
"""Build the fixed screening corpora from tasks/phase32_matched_condition_goal.md
(merge-order step 2), sourced from the already-measured Phase 31 corrected
baseline (data/bench_chunks_phase31_final_e20dc8c) so no candidate arm needs
a repeated full 4,907-target run to get a first read.

Emits, under data/corpora/:
  - screening_500.json  (stratified sample: solved/unsolved proportional,
    solved further stratified by best_depth)
  - hard_200.json        (unsolved, search_limited per
    scripts/decompose_bottlenecks.py: has template matches AND stock hits
    AND hit beam/depth budget — i.e. not explained by absent templates or
    absent stock alone), stratified by nodes_expanded quartile for a
    difficulty spread
  - quality_200.json     (solved, stratified by primary rule used if
    data/corpora/_solved_rule_usage_raw.tsv exists, else by
    (best_depth, route_validation_status, atom_balance_ok))

Each file records: target list (smiles + all source fields), the fixed
seed, sha256 of the target SMILES list (order-independent — sorted before
hashing), and source provenance.

LIMITATION: the goal doc asks Screening-500 to "preserve reaction-class
distribution". USPTO-50k's source file (data/uspto50k_test.smi) has
reaction_class == "UNK" for every single row (this is the same data gap
behind the renkin-bench compare dedup-key bug Track A fixes) — there is no
real class label to stratify by. This script stratifies by solved-status
and route depth instead, and this limitation is recorded in the output
JSON rather than silently faked.
"""
import argparse
import glob
import hashlib
import json
import os
import random
import sys
from collections import defaultdict

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SOURCE_DIR = os.path.join(REPO_ROOT, "data", "bench_chunks_phase31_final_e20dc8c")
OUT_DIR = os.path.join(REPO_ROOT, "data", "corpora")
RULE_USAGE_TSV = os.path.join(OUT_DIR, "_solved_rule_usage_raw.tsv")
SEED = 32  # fixed: "Phase 32"


def load_results(source_dir):
    files = sorted(glob.glob(os.path.join(source_dir, "**", "*.json"), recursive=True))
    if not files:
        print(f"FATAL: no chunk files under {source_dir}", file=sys.stderr)
        sys.exit(1)
    all_results = []
    for f in files:
        with open(f) as fh:
            all_results.extend(json.load(fh)["results"])
    return all_results


def sha256_of_smiles(records):
    h = hashlib.sha256()
    for s in sorted(r["smiles"] for r in records):
        h.update(s.encode())
        h.update(b"\n")
    return h.hexdigest()


def depth_bucket(r):
    d = r.get("best_depth")
    return "unsolved" if d is None else str(d)


def stratified_sample(rng, groups, total_n):
    """Proportionally sample total_n items across groups (dict[key] -> list),
    largest remainder method so the sum is exactly total_n."""
    grand_total = sum(len(v) for v in groups.values())
    raw = {k: len(v) * total_n / grand_total for k, v in groups.items()}
    base = {k: int(v) for k, v in raw.items()}
    remainder = total_n - sum(base.values())
    order = sorted(groups.keys(), key=lambda k: raw[k] - base[k], reverse=True)
    for k in order[:remainder]:
        base[k] += 1

    picked = []
    for k, n in base.items():
        pool = groups[k][:]
        rng.shuffle(pool)
        picked.extend(pool[:n])
    return picked


def build_screening_500(all_results):
    rng = random.Random(SEED)
    groups = defaultdict(list)
    for r in all_results:
        key = ("solved", depth_bucket(r)) if r.get("solved") else ("unsolved",)
        groups[key].append(r)
    picked = stratified_sample(rng, groups, 500)
    rng.shuffle(picked)
    return picked


def build_hard_200(all_results):
    sys.path.insert(0, os.path.join(REPO_ROOT, "scripts"))
    from decompose_bottlenecks import bucket  # noqa: E402

    candidates = [r for r in all_results if not r.get("solved") and bucket(r) == "search_limited"]
    rng = random.Random(SEED)
    ranked = sorted(candidates, key=lambda r: r.get("nodes_expanded") or 0)
    quartiles = defaultdict(list)
    n = len(ranked)
    for i, r in enumerate(ranked):
        quartiles[min(i * 4 // n, 3)].append(r)
    return stratified_sample(rng, quartiles, min(200, len(candidates)))


def load_rule_usage():
    """Parse examples/inspect_validation.rs TSV output: for each solved
    target, the rarest rule used across its route's steps (rarest = most
    diagnostic for stratification purposes)."""
    if not os.path.exists(RULE_USAGE_TSV):
        return None
    rule_counts = defaultdict(int)
    target_rules = defaultdict(list)
    with open(RULE_USAGE_TSV) as f:
        for line in f:
            parts = line.rstrip("\n").split("\t")
            if len(parts) >= 5 and parts[1] == "STEP":
                smiles, _, _status, _bal, rule_field = parts[0], parts[1], parts[2], parts[3], parts[4]
                rule = rule_field.split("=", 1)[1] if rule_field.startswith("rule=") else rule_field
                target_rules[smiles].append(rule)
                rule_counts[rule] += 1
    if not target_rules:
        return None
    rarest_per_target = {}
    for smiles, rules in target_rules.items():
        rarest_per_target[smiles] = min(rules, key=lambda ru: rule_counts[ru])
    return rarest_per_target


def build_quality_200(all_results):
    solved = [r for r in all_results if r.get("solved")]
    rng = random.Random(SEED)
    rarest_per_target = load_rule_usage()
    if rarest_per_target:
        groups = defaultdict(list)
        for r in solved:
            key = rarest_per_target.get(r["smiles"], "UNKNOWN_RULE")
            groups[key].append(r)
        strat_field = "primary_rule"
    else:
        groups = defaultdict(list)
        for r in solved:
            key = (depth_bucket(r), r.get("route_validation_status"), r.get("atom_balance_ok"))
            groups[key].append(r)
        strat_field = "depth_status_balance_fallback"
    picked = stratified_sample(rng, groups, min(200, len(solved)))
    return picked, strat_field


def write_corpus(name, records, extra_meta):
    os.makedirs(OUT_DIR, exist_ok=True)
    path = os.path.join(OUT_DIR, f"{name}.json")
    payload = {
        "name": name,
        "seed": SEED,
        "n": len(records),
        "source": os.path.relpath(SOURCE_DIR, REPO_ROOT),
        "sha256_of_sorted_smiles": sha256_of_smiles(records),
        **extra_meta,
        "targets": records,
    }
    with open(path, "w") as f:
        json.dump(payload, f, indent=2)
    print(f"{name}: n={len(records)} sha256={payload['sha256_of_sorted_smiles'][:16]}... -> {path}")


def main():
    ap = argparse.ArgumentParser()
    ap.parse_args()

    all_results = load_results(SOURCE_DIR)

    s500 = build_screening_500(all_results)
    write_corpus(
        "screening_500",
        s500,
        {
            "strategy": "proportional solved/unsolved, solved further stratified by best_depth",
            "limitation": (
                "goal doc asks to preserve reaction-class distribution; "
                "source data has reaction_class=UNK for every target (same "
                "gap as the renkin-bench compare dedup-key bug) so this is "
                "not possible -- stratified by solved-status/depth instead."
            ),
        },
    )

    h200 = build_hard_200(all_results)
    write_corpus(
        "hard_200",
        h200,
        {
            "strategy": (
                "unsolved AND bucket=search_limited (has matched_templates>0 "
                "and stock_hits>0 and hit beam/depth budget) per "
                "scripts/decompose_bottlenecks.py, stratified by "
                "nodes_expanded quartile"
            ),
        },
    )

    q200, strat_field = build_quality_200(all_results)
    write_corpus(
        "quality_200",
        q200,
        {
            "strategy": f"solved, stratified by {strat_field}",
            "rule_usage_source": (
                os.path.relpath(RULE_USAGE_TSV, REPO_ROOT)
                if os.path.exists(RULE_USAGE_TSV)
                else None
            ),
        },
    )


if __name__ == "__main__":
    main()
