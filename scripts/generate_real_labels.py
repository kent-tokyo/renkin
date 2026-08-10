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
through the `renkin-canonicalize --clear-atom-maps` binary -- RENKIN's own
canonicalizer (`chem_env::to_canonical`), the exact function
`propose_one_step` uses to produce `precursor_smiles` in the candidate
pool. `train_reranker.py` labels a candidate positive via an EXACT string
match against `correct_precursor_sets` (`label_and_split_rows`), so using
any other toolkit's canonical form (e.g. RDKit) would silently produce zero
matches.

Atom maps in the raw dataset (`[CH3:1]O`) are cleared structurally inside
the binary (`chem_env::clear_atom_maps`, operating on the parsed molecule
graph), never by regex/string manipulation on the SMILES text here: `:` is
also SMILES bond syntax (explicit aromatic/double bonds), so a text-level
`:\d+` strip can delete a ring-closure digit that happens to follow an
explicit bond symbol rather than an atom map, silently corrupting the ring
into an open chain. See
`chem_env::clear_atom_maps_tests::explicit_colon_bond_with_ring_closure_digit_is_not_corrupted`
for a concrete before/after example.

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
import json
import sys
from collections import defaultdict

from reranker_label_common import canonicalize_batch, sha256_of


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--raw-test-split", default="data/uspto50k_raw_test_split.jsonl")
    parser.add_argument("--sample-list", default="data/comparison/sample_full_sorted.jsonl")
    parser.add_argument("--canonicalize-bin", default="target/release/renkin-canonicalize")
    parser.add_argument("--output", default="data/reranker_labels_uspto50k_test.jsonl")
    parser.add_argument(
        "--groups-output",
        default="data/reranker_groups_uspto50k_test.jsonl",
        help="{group_id, target_id} pairs only, no ground truth -- the input a pool-generation "
             "driver consumes, kept separate from --output so proposal/label separation holds "
             "even at the file level (a driver never needs to see correct_precursor_sets).",
    )
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
    # binary invocation as the pool exporter will use at runtime. Fragments
    # are still split on top-level '.' here (never inside '[...]' bracket
    # atom notation, so a plain split is safe) -- precursor_smiles in the
    # pool is one string per fragment, not one multi-component string.
    # Atom maps are left in place; the binary clears them structurally.
    raw_products_canon = canonicalize_batch(
        [r["product"] for r in raw_rows], args.canonicalize_bin
    )

    reactant_frag_index: list[tuple[int, int]] = []  # (raw_row_idx, frag_idx)
    reactant_frags_mapped: list[str] = []
    for i, r in enumerate(raw_rows):
        for frag in r["reactants"].split("."):
            reactant_frag_index.append((i, len(reactant_frags_mapped)))
            reactant_frags_mapped.append(frag)
    reactant_frags_canon = canonicalize_batch(reactant_frags_mapped, args.canonicalize_bin)

    n_product_parse_fail = sum(1 for c in raw_products_canon if c is None)
    n_reactant_parse_fail = sum(1 for c in reactant_frags_canon if c is None)

    # Group raw rows -> distinct canonical precursor-sets, by canonical product.
    by_product: dict[str, set[tuple[str, ...]]] = defaultdict(set)
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
        # `sample_id` is sample_full_sorted.jsonl's own identifier
        # (`uspto50k_test#L{n}`), used as group_id -- a human-readable,
        # traceable-to-source label. `target_id` must be the RENKIN-canonical
        # SMILES text: `propose_one_step` derives `CandidatePool.target_id`
        # from the target molecule itself (`to_canonical(&target_mol)`), not
        # from any caller-supplied identifier, so a labels file whose
        # target_id isn't that same canonical text would fail
        # train_reranker.py's group-index cross-check on every row. Verified
        # empirically (see round2_split_hygiene.md's driver-design note)
        # before writing this pipeline stage.
        sample_id = t["target_id"]
        if target_canon is None:
            unmatched.append({"target_id": sample_id, "reason": "target_smiles_parse_fail"})
            continue
        precursor_sets = by_product.get(target_canon)
        if not precursor_sets:
            unmatched.append({"target_id": sample_id, "reason": "no_test_split_ground_truth_match"})
            continue
        if len(precursor_sets) > 1:
            n_multi_route += 1
        group_id = sample_id  # 1 group per USPTO-50k target; see module docstring.
        assert group_id not in seen_group_ids, f"duplicate group_id {group_id!r}"
        assert target_canon not in seen_target_ids, f"duplicate target_id {target_canon!r}"
        seen_group_ids.add(group_id)
        seen_target_ids.add(target_canon)
        output_rows.append(
            {
                "schema_version": 1,
                "group_id": group_id,
                "target_id": target_canon,
                "correct_precursor_sets": [list(s) for s in sorted(precursor_sets)],
            }
        )

    assert len(seen_group_ids) == len(seen_target_ids) == len(output_rows), (
        "group_id/target_id/row-count mismatch -- label rows must be 1:1 with targets"
    )

    with open(args.output, "w", encoding="utf-8") as f:
        for row in output_rows:
            f.write(json.dumps(row, sort_keys=True) + "\n")

    with open(args.groups_output, "w", encoding="utf-8") as f:
        for row in output_rows:
            f.write(
                json.dumps({"group_id": row["group_id"], "target_id": row["target_id"]}, sort_keys=True)
                + "\n"
            )

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
