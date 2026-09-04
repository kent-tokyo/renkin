#!/usr/bin/env python3
"""Deterministic 500-target cohort selection for the v0.24 coverage-mode
formal-TEST confirmation protocol (see
`data/coverage_mode_formal_test/protocol.md`).

Selects a prefix of a SHA-256-keyed sorted ordering over the existing
4,903-group formal TEST corpus (`data/reranker_groups_uspto50k_test.jsonl`
-- the same denominator Task 35's formal candidate-ranking gate used, see
`data/phase3e_reranker_training/findings.md`), not the raw 4,907-row
`data/uspto50k_test.smi` directly (that file has rows the reranker-group
extraction step already dropped as unusable, e.g. unparseable/duplicate
products -- reusing the corpus that already excludes those keeps this
cohort a strict subset of the same 4,903 groups the rest of this program
calls "formal TEST").

Same technique as `scripts/compare_sampling.py` (SHA-256-of-canonical-
SMILES sort, RDKit as the tool-neutral canonicalizer), but a distinct
`PROTOCOL_VERSION` string -- this cohort has nothing to do with Issue #66's
open-source planner comparison and must not share its hash namespace, even
though both draw from the same underlying TEST corpus.

This script only selects *which* 500 targets are in scope -- it does not
run any search and does not touch route-search outcomes for any target.
Per the user's explicit pre-registration: "target結果を見ずにmanifestを
commit" (commit the manifest before seeing any result).

Usage:
    python3 scripts/select_coverage_mode_formal_test_cohort.py \
        --groups data/reranker_groups_uspto50k_test.jsonl \
        --cohort-size 500 \
        --output data/coverage_mode_formal_test/cohort_manifest.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys

try:
    from rdkit import Chem, RDLogger

    RDLogger.DisableLog("rdApp.*")
    HAVE_RDKIT = True
except ImportError:  # pragma: no cover -- exercised by scripts/tests without the dep installed
    HAVE_RDKIT = False

PROTOCOL_VERSION = "renkin-v024-coverage-formal-test-cohort-v1"


def canonical_smiles(raw_smiles: str) -> str | None:
    if not HAVE_RDKIT:
        raise RuntimeError("rdkit is required: pip install rdkit")
    mol = Chem.MolFromSmiles(raw_smiles)
    if mol is None:
        return None
    return Chem.MolToSmiles(mol, canonical=True)


def sample_key(canonical: str) -> str:
    h = hashlib.sha256()
    h.update(f"{PROTOCOL_VERSION}|".encode("utf-8"))
    h.update(canonical.encode("utf-8"))
    return h.hexdigest()


def sha256_text(text: str) -> str:
    return hashlib.sha256(text.encode("utf-8")).hexdigest()


def sha256_file(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(65536), b""):
            h.update(chunk)
    return h.hexdigest()


def load_groups(groups_path: str) -> list[dict]:
    rows = []
    with open(groups_path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line:
                continue
            rows.append(json.loads(line))
    return rows


def build_cohort(groups_path: str, cohort_size: int) -> dict:
    rows = load_groups(groups_path)
    total_groups = len(rows)

    unparseable = []
    keyed = []
    for row in rows:
        canon = canonical_smiles(row["target_id"])
        if canon is None:
            unparseable.append(row["group_id"])
            continue
        key = sample_key(canon)
        keyed.append((key, canon, row["group_id"], row["target_id"]))

    # Sort by (sample_key, canonical_smiles) for a fully deterministic
    # tie-break (matches scripts/compare_sampling.py's convention).
    keyed.sort(key=lambda t: (t[0], t[1]))

    if len(keyed) < cohort_size:
        raise ValueError(
            f"only {len(keyed)} parseable groups available, need {cohort_size}"
        )

    selected = keyed[:cohort_size]
    excluded_count = len(keyed) - cohort_size

    cohort_rows = []
    for rank, (key, canon, group_id, raw_target_id) in enumerate(selected):
        cohort_rows.append(
            {
                "cohort_rank": rank,
                "group_id": group_id,
                "target_id": raw_target_id,
                "canonical_smiles": canon,
                "sample_key": key,
            }
        )

    cohort_list_json = json.dumps(cohort_rows, sort_keys=True, separators=(",", ":"))

    manifest = {
        "_purpose": (
            "Pre-registered 500-target cohort for the v0.24 coverage-mode "
            "formal-TEST confirmation protocol. Committed before any "
            "search is run against these targets -- selection is a "
            "deterministic function of the corpus content alone, not "
            "influenced by any observed route-search outcome."
        ),
        "protocol_version": PROTOCOL_VERSION,
        "selection_rule": (
            "sample_key = SHA256(protocol_version + '|' + canonical_smiles), "
            "RDKit canonicalization, sorted ascending by (sample_key, "
            "canonical_smiles), first N taken as a prefix."
        ),
        "source_corpus": groups_path,
        "source_corpus_sha256": sha256_file(groups_path),
        "source_total_groups": total_groups,
        "source_unparseable_by_rdkit": unparseable,
        "cohort_size": cohort_size,
        "excluded_count": excluded_count,
        "cohort_targets_sha256": sha256_text(cohort_list_json),
        "targets": cohort_rows,
    }
    return manifest


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--groups", default="data/reranker_groups_uspto50k_test.jsonl")
    ap.add_argument("--cohort-size", type=int, default=500)
    ap.add_argument(
        "--output", default="data/coverage_mode_formal_test/cohort_manifest.json"
    )
    args = ap.parse_args()

    manifest = build_cohort(args.groups, args.cohort_size)
    with open(args.output, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")

    print(
        f"cohort_size={manifest['cohort_size']} "
        f"excluded={manifest['excluded_count']} "
        f"unparseable={len(manifest['source_unparseable_by_rdkit'])} "
        f"cohort_targets_sha256={manifest['cohort_targets_sha256'][:16]}... "
        f"-> {args.output}"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
