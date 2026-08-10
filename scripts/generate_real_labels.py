"""Issue #101 Phase 3A: real ground-truth labels for the retro candidate-pool
reranker (`scripts/train_reranker.py`), derived from the raw USPTO-50k TEST
split only (never train/val -- see "Split hygiene" below).

Input: `data/uspto50k_raw_test_split.jsonl`, a one-time plain-JSONL dump of
the raw `bisectgroup/USPTO_50K` (Hugging Face) `test` split, produced by:

    /path/to/a/python/with/datasets+pyarrow -c "
    import pyarrow as pa, pyarrow.ipc as ipc, json
    path = '<HF_CACHE>/datasets/bisectgroup___uspto_50_k/default/0.0.0/<REVISION>/uspto_50_k-test.arrow'
    with pa.memory_map(path, 'r') as source:
        table = ipc.open_stream(source).read_all()
    with open('data/uspto50k_raw_test_split.jsonl', 'w') as f:
        for r in table.to_pylist():
            f.write(json.dumps({'id': r['id'], 'class': r['class'],
                                 'reactants': r['reactants'], 'product': r['product']},
                                sort_keys=True) + '\n')
    "

kept as a separate one-time step because the project's own .venv has no
pyarrow/datasets install; this script itself needs only the stdlib plus the
`renkin-canonicalize` binary, so it stays runnable from the normal repo venv.
Dataset revision `08a575f0546b2be57242997fd45f684d6814d5a9` (HF hub commit
for `bisectgroup/USPTO_50K`, config `default`); see
`data/phase3a_reranker_ground_truth_audit/findings.md` for the full
provenance table (SHA-256 of the source .arrow files and this dump).

Split hygiene: labels are built ONLY from raw rows in the test split. A
product's raw reactant set from train or val is never folded in, even when
the same product also appears there under USPTO-50k's reaction-level split
-- doing so would let train-derived ground truth leak into whichever
targets land in train_reranker.py's own held-out test bucket (it re-splits
these targets 70/15/15 by `target_id` hash, independently of the USPTO
train/val/test partition). Within the test split, a product legitimately
recorded via more than one reaction (multiple literature routes) keeps all
of its distinct reactant sets -- that's the schema's intended use of a list
under `correct_precursor_sets`, and is within-split, so it carries no
leakage risk.

Canonicalization: every SMILES (target and precursor fragments) is run
through the `renkin-canonicalize` binary -- RENKIN's own canonicalizer
(`chem_env::to_canonical`), the exact function `propose_one_step` uses to
produce `precursor_smiles` in the candidate pool. `train_reranker.py`
labels a candidate positive via an EXACT string match against
`correct_precursor_sets` (`label_and_split_rows`), so using any other
toolkit's canonical form (e.g. RDKit) would silently produce zero matches.

Usage:
    cargo build --release --bin renkin-canonicalize
    python3 scripts/generate_real_labels.py \
        --raw-test-split data/uspto50k_raw_test_split.jsonl \
        --sample-list data/comparison/sample_full_sorted.jsonl \
        --canonicalize-bin target/release/renkin-canonicalize \
        --output data/reranker_labels_uspto50k_test.jsonl \
        --summary-output data/reranker_labels_uspto50k_test.summary.json
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import subprocess
import sys
from collections import defaultdict

_ATOM_MAP_RE = re.compile(r":\d+")


def strip_atom_map(smiles: str) -> str:
    return _ATOM_MAP_RE.sub("", smiles)


def sha256_of(path: str) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def canonicalize_batch(smiles_list: list[str], canonicalize_bin: str) -> list[str | None]:
    """Batch-canonicalize via the renkin-canonicalize binary. Returns None
    for entries the binary reports as "ERR" (unparseable)."""
    if not smiles_list:
        return []
    inp = "\n".join(smiles_list) + "\n"
    try:
        result = subprocess.run(
            [canonicalize_bin], input=inp, capture_output=True, text=True, timeout=600
        )
    except FileNotFoundError:
        raise RuntimeError(
            f"renkin-canonicalize binary not found at {canonicalize_bin!r}. "
            "Build with: cargo build --release --bin renkin-canonicalize"
        )
    if result.returncode != 0:
        raise RuntimeError(
            f"renkin-canonicalize failed (exit {result.returncode}):\n{result.stderr}"
        )
    lines = result.stdout.split("\n")
    if lines and lines[-1] == "":
        lines = lines[:-1]
    if len(lines) != len(smiles_list):
        raise RuntimeError(
            f"renkin-canonicalize output line count ({len(lines)}) != input count "
            f"({len(smiles_list)}) -- cannot safely align results"
        )
    return [None if line == "ERR" else line for line in lines]


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--raw-test-split", default="data/uspto50k_raw_test_split.jsonl")
    parser.add_argument("--sample-list", default="data/comparison/sample_full_sorted.jsonl")
    parser.add_argument("--canonicalize-bin", default="target/release/renkin-canonicalize")
    parser.add_argument("--output", default="data/reranker_labels_uspto50k_test.jsonl")
    parser.add_argument("--summary-output", default="data/reranker_labels_uspto50k_test.summary.json")
    args = parser.parse_args(argv)

    with open(args.raw_test_split, "r", encoding="utf-8") as f:
        raw_rows = [json.loads(line) for line in f if line.strip()]
    with open(args.sample_list, "r", encoding="utf-8") as f:
        targets = [json.loads(line) for line in f if line.strip()]

    n_raw = len(raw_rows)
    n_targets = len(targets)

    # Canonicalize every raw product + every raw reactant fragment in one
    # batch call each, so target/precursor identity uses the exact same
    # binary invocation as the pool exporter will use at runtime.
    raw_products_stripped = [strip_atom_map(r["product"]) for r in raw_rows]
    raw_products_canon = canonicalize_batch(raw_products_stripped, args.canonicalize_bin)

    reactant_frag_index: list[tuple[int, int]] = []  # (raw_row_idx, frag_idx)
    reactant_frags_stripped: list[str] = []
    for i, r in enumerate(raw_rows):
        for frag in r["reactants"].split("."):
            reactant_frag_index.append((i, len(reactant_frags_stripped)))
            reactant_frags_stripped.append(strip_atom_map(frag))
    reactant_frags_canon = canonicalize_batch(reactant_frags_stripped, args.canonicalize_bin)

    n_product_parse_fail = sum(1 for c in raw_products_canon if c is None)
    n_reactant_parse_fail = sum(1 for c in reactant_frags_canon if c is None)

    # Group raw rows -> distinct canonical precursor-sets, by canonical product.
    by_product: dict[str, set[tuple[str, ...]]] = defaultdict(set)
    frag_iter = iter(zip(reactant_frag_index, reactant_frags_canon))
    row_frags: dict[int, list[str | None]] = defaultdict(list)
    for (row_idx, _frag_pos), canon in zip(reactant_frag_index, reactant_frags_canon):
        row_frags[row_idx].append(canon)

    n_rows_reactant_parse_fail = 0
    for i, product_canon in enumerate(raw_products_canon):
        if product_canon is None:
            continue
        frags = row_frags.get(i, [])
        if any(c is None for c in frags):
            n_rows_reactant_parse_fail += 1
            continue
        by_product[product_canon].add(tuple(sorted(frags)))

    target_canons = canonicalize_batch(
        [t["canonical_smiles"] for t in targets], args.canonicalize_bin
    )

    output_rows = []
    unmatched = []
    n_multi_route = 0
    seen_group_ids = set()
    seen_target_ids = set()
    for t, target_canon in zip(targets, target_canons):
        target_id = t["target_id"]
        if target_canon is None:
            unmatched.append({"target_id": target_id, "reason": "target_smiles_parse_fail"})
            continue
        precursor_sets = by_product.get(target_canon)
        if not precursor_sets:
            unmatched.append({"target_id": target_id, "reason": "no_test_split_ground_truth_match"})
            continue
        if len(precursor_sets) > 1:
            n_multi_route += 1
        group_id = target_id  # 1 group per USPTO-50k target; see module docstring.
        assert group_id not in seen_group_ids, f"duplicate group_id {group_id!r}"
        assert target_id not in seen_target_ids, f"duplicate target_id {target_id!r}"
        seen_group_ids.add(group_id)
        seen_target_ids.add(target_id)
        output_rows.append(
            {
                "schema_version": 1,
                "group_id": group_id,
                "target_id": target_id,
                "correct_precursor_sets": [list(s) for s in sorted(precursor_sets)],
            }
        )

    assert len(seen_group_ids) == len(seen_target_ids) == len(output_rows), (
        "group_id/target_id/row-count mismatch -- label rows must be 1:1 with targets"
    )

    with open(args.output, "w", encoding="utf-8") as f:
        for row in output_rows:
            f.write(json.dumps(row, sort_keys=True) + "\n")

    summary = {
        "dataset": "bisectgroup/USPTO_50K",
        "hf_revision": "08a575f0546b2be57242997fd45f684d6814d5a9",
        "raw_test_split_path": args.raw_test_split,
        "raw_test_split_sha256": sha256_of(args.raw_test_split),
        "sample_list_path": args.sample_list,
        "sample_list_sha256": sha256_of(args.sample_list),
        "n_raw_test_records": n_raw,
        "n_raw_product_parse_fail": n_product_parse_fail,
        "n_raw_reactant_fragment_parse_fail": n_reactant_parse_fail,
        "n_raw_rows_dropped_for_reactant_parse_fail": n_rows_reactant_parse_fail,
        "n_unique_canonical_products_in_test_split": len(by_product),
        "n_targets_in_sample_list": n_targets,
        "n_targets_labeled": len(output_rows),
        "n_targets_unmatched": len(unmatched),
        "unmatched_targets": unmatched,
        "n_targets_with_multiple_distinct_routes": n_multi_route,
        "output_path": args.output,
    }
    with open(args.summary_output, "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2, sort_keys=True)
        f.write("\n")

    print(json.dumps(summary, indent=2, sort_keys=True), file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
