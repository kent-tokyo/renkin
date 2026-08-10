"""Phase 3D.5 Step 1: machine-readable ledger for the 123 target_id-mismatch
groups discovered in Phase 3D (109 train + 14 val). Diagnostic-only, reads
already-generated artifacts and re-runs `renkin-canonicalize` (never
`propose_one_step`/rules -- this is a canonicalization audit, not a
proposal re-run).

Usage:
    python3 scripts/phase3d5_build_mismatch_ledger.py
"""

import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
AUDIT_DIR = ROOT / "data" / "phase3d5_canonical_identity_audit"
CANONICALIZE_BIN = ROOT / "target" / "release" / "renkin-canonicalize"

RAW_SPLIT_FILES = {
    "train": ROOT / "data" / "uspto50k_raw_train_split.jsonl",
    "val": ROOT / "data" / "uspto50k_raw_val_split.jsonl",
}
LABELS_FILES = {
    "train": ROOT / "data" / "reranker_labels_uspto50k_train.jsonl",
    "val": ROOT / "data" / "reranker_labels_uspto50k_val.jsonl",
}

GROUP_ID_RE = re.compile(r"^uspto50k_(train|val)#L(\d+)$")


def canonicalize_batch(strings, clear_atom_maps=False):
    args = [str(CANONICALIZE_BIN)]
    if clear_atom_maps:
        args.append("--clear-atom-maps")
    proc = subprocess.run(
        args, input="\n".join(strings) + "\n", capture_output=True, text=True, check=True
    )
    out = proc.stdout.splitlines()
    assert len(out) == len(strings), f"line count mismatch: {len(strings)} in, {len(out)} out"
    return out


def load_labels_by_group_id(path):
    by_group = {}
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            row = json.loads(line)
            by_group[row["group_id"]] = row
    return by_group


def main():
    with open(AUDIT_DIR / "mismatch_groups_raw.json", "r", encoding="utf-8") as f:
        mismatches = json.load(f)

    raw_lines_cache = {}
    for split, path in RAW_SPLIT_FILES.items():
        with open(path, "r", encoding="utf-8") as f:
            raw_lines_cache[split] = f.read().splitlines()

    labels_cache = {split: load_labels_by_group_id(path) for split, path in LABELS_FILES.items()}

    requested_ids = [m["requested_target_id"] for m in mismatches]
    pass1 = canonicalize_batch(requested_ids, clear_atom_maps=False)
    pass2 = canonicalize_batch(pass1, clear_atom_maps=False)
    pass3 = canonicalize_batch(pass2, clear_atom_maps=False)

    raw_products = []
    raw_reactants = []
    for m in mismatches:
        match = GROUP_ID_RE.match(m["group_id"])
        assert match, f"unexpected group_id format: {m['group_id']}"
        split, line_no = match.group(1), int(match.group(2))
        assert split == m["split"]
        raw_row = json.loads(raw_lines_cache[split][line_no - 1])
        raw_products.append(raw_row["product"])
        raw_reactants.append(raw_row["reactants"])

    map_cleared = canonicalize_batch(raw_products, clear_atom_maps=True)

    ledger = []
    for i, m in enumerate(mismatches):
        split = m["split"]
        group_id = m["group_id"]
        requested = m["requested_target_id"]
        label_row = labels_cache[split].get(group_id)
        ledger.append(
            {
                "group_id": group_id,
                "split": split,
                "requested_target_id": requested,
                "propose_one_step_derived_target_id": pass1[i],
                "source_raw_mapped_product": raw_products[i],
                "source_raw_mapped_reactants": raw_reactants[i],
                "map_cleared_representation": map_cleared[i],
                "map_cleared_matches_requested": map_cleared[i] == requested,
                "canonicalization_pass1": pass1[i],
                "canonicalization_pass2": pass2[i],
                "canonicalization_pass3": pass3[i],
                "requested_eq_pass1": requested == pass1[i],
                "pass1_eq_pass2": pass1[i] == pass2[i],
                "pass2_eq_pass3": pass2[i] == pass3[i],
                "fixed_point_reached_at_pass1": pass1[i] == pass2[i],
                "classification": (
                    "requested_already_fixed_point"
                    if requested == pass1[i]
                    else "A_to_B_to_B_single_drift"
                    if pass1[i] == pass2[i]
                    else "still_drifting_after_pass2_needs_investigation"
                ),
                "label_available": label_row is not None,
                "correct_precursor_sets": label_row["correct_precursor_sets"] if label_row else None,
            }
        )

    with open(AUDIT_DIR / "mismatch_ledger.json", "w", encoding="utf-8") as f:
        json.dump(ledger, f, indent=2, sort_keys=False)

    n_drift = sum(1 for r in ledger if not r["requested_eq_pass1"])
    n_stable_after_1 = sum(1 for r in ledger if r["fixed_point_reached_at_pass1"])
    n_map_cleared_matches = sum(1 for r in ledger if r["map_cleared_matches_requested"])
    n_label_available = sum(1 for r in ledger if r["label_available"])
    print(f"total rows: {len(ledger)}")
    print(f"requested != pass1 (confirms mismatch): {n_drift} / {len(ledger)}")
    print(f"fixed point reached at pass1 (A->B->B, not A->B->C): {n_stable_after_1} / {len(ledger)}")
    print(f"map_cleared_representation == requested_target_id: {n_map_cleared_matches} / {len(ledger)}")
    print(f"label available in labels file: {n_label_available} / {len(ledger)}")


if __name__ == "__main__":
    main()
