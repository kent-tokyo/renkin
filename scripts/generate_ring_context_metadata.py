#!/usr/bin/env python3
"""
Generate ring-context provenance metadata for extracted templates (Issue #72).

For each of the 500 checked-in extracted templates (`data/templates_extracted_500.smi`),
determines which mapped bonds the template deletes (present on the LHS pattern,
absent on the RHS pattern -- the retro-direction disconnection point), then
classifies each such bond's `RingBondIntent` (Ring / NonRing / Either / Unknown)
from the REAL product molecule of every USPTO-50k training reaction that
contributed to that template, NOT from the abstract SMARTS pattern (which never
carries ring-membership primitives -- see Issue #72's root-cause finding that
this information is `absent_in_rdchiral_output`, not stripped by RENKIN).

Atom-map numbers are preserved unchanged through `simplify_atom`/`simplify_smirks`
(verified: 0/500 mismatches across the full checked-in corpus), so a changed
bond's (map_a, map_b) pair identified on the simplified template can be looked
up directly against each source reaction's real, atom-mapped product SMILES.

Output: a deterministic sidecar JSON keyed by `smirks-sha256:<hex>` (RENKIN's
own `template_id_for_smirks` scheme, `src/chem_env.rs:51-55`) -- NOT by
`extracted_N`, since that's an unstable re-extraction-order-dependent line
position (this repo's own audit found only 389/500 exact rank+count matches
on reproduction; a name-keyed sidecar would silently misattribute intent).

Usage:
    python3 scripts/generate_ring_context_metadata.py \
        --templates data/templates_extracted_500.smi \
        --output data/ring_context_metadata_500.json
"""
import argparse
import hashlib
import importlib.metadata
import json
import re
import sys
from collections import defaultdict

from datasets import load_dataset
from rdchiral.template_extractor import extract_from_reaction
from rdkit import Chem, rdBase

sys.path.insert(0, __file__.rsplit("/", 1)[0])
from extract_templates import simplify_smirks, is_valid_for_chematic  # noqa: E402

SCHEMA_VERSION = 1
DATASET_ID = "bisectgroup/USPTO_50K"
DATASET_SPLIT = "train"


def sha256_hex(s: str) -> str:
    return hashlib.sha256(s.encode("utf-8")).hexdigest()


def template_id_for_smirks(smirks: str) -> str:
    """Mirrors src/chem_env.rs:51-55 exactly: sha256(smirks.trim()), lowercase hex."""
    return f"smirks-sha256:{sha256_hex(smirks.strip())}"


def load_checked_in_templates(path: str) -> list:
    templates = []
    with open(path) as f:
        for line in f:
            if line.startswith("#") or not line.strip():
                continue
            smirks, count = line.rstrip("\n").split("\t")
            templates.append((smirks, int(count)))
    return templates


def mapped_bonds(smarts_side: str) -> set:
    """(map_a, map_b) pairs (a < b) for every bond whose both endpoints carry
    an atom map number, parsed via RDKit SMARTS. Mirrors the atom-map-pair
    keying `mapped_bond_signature` uses in crates/renkin-forward/src/hints.rs,
    but only needs presence (not full BondQuery) for this purpose."""
    mol = Chem.MolFromSmarts(smarts_side)
    if mol is None:
        return set()
    pairs = set()
    for bond in mol.GetBonds():
        a = bond.GetBeginAtom().GetAtomMapNum()
        b = bond.GetEndAtom().GetAtomMapNum()
        if a and b:
            pairs.add((a, b) if a < b else (b, a))
    return pairs


def changed_bonds_for_template(simplified_smirks: str) -> list:
    """Bonds present on the LHS (target-matching) pattern but absent on the
    RHS (precursor-producing) pattern -- the disconnection point a retro
    application makes. Matches `compute_bond_delta`'s "broken" classification
    in hints.rs, computed directly (no reversal needed: retro SMIRKS's LHS
    already matches the target/product side)."""
    if ">>" not in simplified_smirks:
        return []
    lhs, rhs = simplified_smirks.split(">>", 1)
    lhs_bonds = mapped_bonds(lhs)
    rhs_bonds = mapped_bonds(rhs)
    return sorted(lhs_bonds - rhs_bonds)


def lhs_query_and_map_index(simplified_smirks: str):
    """Parse the LHS side once and return (query_mol, {atom_map: query_atom_idx}).

    CRITICAL: rdchiral renumbers atom-map numbers locally within each
    extracted template -- they do NOT correspond to the original per-reaction
    atom-map numbers in the dataset's atom-mapped product SMILES (verified
    empirically: reaction 0's product maps atoms 1-27+, but its extracted
    template only uses local maps 1-2). So a changed bond's (map_a, map_b)
    can only be resolved to real atoms in a specific reaction's product via a
    SMARTS substructure match against that product (`lhs_query_and_map_index`
    + `GetSubstructMatches`), never by direct AtomMapNum lookup.
    """
    lhs = simplified_smirks.split(">>", 1)[0]
    query_mol = Chem.MolFromSmarts(lhs)
    if query_mol is None:
        return None, {}
    map_to_query_idx = {
        atom.GetAtomMapNum(): atom.GetIdx()
        for atom in query_mol.GetAtoms()
        if atom.GetAtomMapNum()
    }
    return query_mol, map_to_query_idx


def classify_intent(ring_obs: int, non_ring_obs: int, unknown_obs: int) -> str:
    if ring_obs > 0 and non_ring_obs > 0:
        return "either"
    if ring_obs > 0:
        return "ring"
    if non_ring_obs > 0:
        return "non_ring"
    return "unknown"


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--templates", default="data/templates_extracted_500.smi")
    ap.add_argument("--output", default="data/ring_context_metadata_500.json")
    ap.add_argument("--dataset-id", default=DATASET_ID)
    ap.add_argument("--split", default=DATASET_SPLIT)
    args = ap.parse_args()

    checked_in = load_checked_in_templates(args.templates)
    print(f"Loaded {len(checked_in)} checked-in templates from {args.templates}", flush=True)

    # Precompute changed bonds + smirks_sha256 per checked-in template, and a
    # fast lookup from simplified-SMIRKS string -> template id, for the main
    # reaction loop.
    template_meta = {}
    smirks_to_id = {}
    n_lhs_query_parse_failed = 0
    for smirks, count in checked_in:
        tid = template_id_for_smirks(smirks)
        cbonds = changed_bonds_for_template(smirks)
        query_mol, map_to_query_idx = lhs_query_and_map_index(smirks)
        if cbonds and query_mol is None:
            n_lhs_query_parse_failed += 1
        template_meta[tid] = {
            "smirks": smirks,
            "count": count,
            "changed_bonds": cbonds,
            "query_mol": query_mol,
            "map_to_query_idx": map_to_query_idx,
            # (map_a, map_b) -> [ring_observations, non_ring_observations, unknown_observations]
            "obs": {pair: [0, 0, 0] for pair in cbonds},
        }
        smirks_to_id[smirks] = tid
    if n_lhs_query_parse_failed:
        print(
            f"WARNING: {n_lhs_query_parse_failed} templates have changed bonds but their "
            "LHS pattern failed to parse as a SMARTS query -- all their bonds will be "
            "'unknown' (mapped_bonds() already succeeded on the same string via a separate "
            "MolFromSmarts call, so this indicates a parser inconsistency worth investigating "
            "if n_lhs_query_parse_failed > 0).",
            flush=True,
        )

    n_no_changed_bonds = sum(1 for m in template_meta.values() if not m["changed_bonds"])
    print(
        f"{len(template_meta) - n_no_changed_bonds}/{len(template_meta)} templates have "
        f"at least one changed (deleted) mapped bond; {n_no_changed_bonds} delete none "
        f"(e.g. pure functional-group interconversion, no fragment split).",
        flush=True,
    )

    print(f"Loading {args.dataset_id} ({args.split} split)...", flush=True)
    ds = load_dataset(args.dataset_id, split=args.split)
    n_reactions = len(ds)
    print(f"  {n_reactions} reactions loaded", flush=True)

    n_matched_reactions = 0
    n_extract_errors = 0
    n_product_parse_errors = 0

    for i, row in enumerate(ds):
        if i % 5000 == 0:
            print(f"  processing {i}/{n_reactions}...", flush=True)
        reaction = {
            "reactants": row["reactants"],
            "products": row["product"],
            "_id": row.get("id", str(i)),
        }
        try:
            result = extract_from_reaction(reaction)
            raw_template = result.get("reaction_smarts")
        except Exception:
            n_extract_errors += 1
            continue
        if not raw_template:
            continue

        simplified = simplify_smirks(raw_template)
        if not is_valid_for_chematic(simplified):
            continue
        tid = smirks_to_id.get(simplified)
        if tid is None:
            continue  # this reaction's template isn't one of the 500 checked-in

        meta = template_meta[tid]
        if not meta["changed_bonds"]:
            continue

        product_mol = Chem.MolFromSmiles(row["product"])
        if product_mol is None:
            n_product_parse_errors += 1
            continue

        n_matched_reactions += 1
        query_mol = meta["query_mol"]
        if query_mol is None:
            for pair in meta["changed_bonds"]:
                meta["obs"][pair][2] += 1
            continue

        # A template can match the same real product at more than one site
        # (symmetric/repeated substructures); every match is a legitimate
        # data point for "what does this bond look like wherever this
        # template's LHS pattern can apply" -- which is exactly what the
        # runtime guard needs (it filters per-match against arbitrary future
        # targets, not just this one historical occurrence).
        matches = product_mol.GetSubstructMatches(query_mol, uniquify=True)
        if not matches:
            # LHS was derived from this exact product by rdchiral, so this
            # should be rare; simplification (dropping D/H0/+0 constraints)
            # can occasionally loosen or -- via H-count changes -- tighten
            # the pattern enough to change match behavior. Counts as unknown
            # rather than silently skipped.
            for pair in meta["changed_bonds"]:
                meta["obs"][pair][2] += 1
            continue

        for match in matches:
            map_to_real_idx = {
                m: match[qidx] for m, qidx in meta["map_to_query_idx"].items()
            }
            for (map_a, map_b) in meta["changed_bonds"]:
                obs = meta["obs"][(map_a, map_b)]
                idx_a = map_to_real_idx.get(map_a)
                idx_b = map_to_real_idx.get(map_b)
                if idx_a is None or idx_b is None:
                    obs[2] += 1
                    continue
                bond = product_mol.GetBondBetweenAtoms(idx_a, idx_b)
                if bond is None:
                    obs[2] += 1
                elif bond.IsInRing():
                    obs[0] += 1
                else:
                    obs[1] += 1

    print(
        f"Matched {n_matched_reactions} reaction occurrences against the 500 checked-in "
        f"templates ({n_extract_errors} extraction errors, {n_product_parse_errors} "
        f"product-parse errors, both excluded from observation counts).",
        flush=True,
    )

    templates_out = {}
    for tid, meta in sorted(template_meta.items()):
        changed_bonds_out = []
        for pair in meta["changed_bonds"]:
            ring_obs, non_ring_obs, unknown_obs = meta["obs"][pair]
            changed_bonds_out.append(
                {
                    "map_a": pair[0],
                    "map_b": pair[1],
                    "operation": "delete",
                    "intent": classify_intent(ring_obs, non_ring_obs, unknown_obs),
                    "ring_observations": ring_obs,
                    "non_ring_observations": non_ring_obs,
                    "unknown_observations": unknown_obs,
                }
            )
        templates_out[tid] = {
            "smirks_sha256": tid,
            "simplified_smirks": meta["smirks"],
            "count": meta["count"],
            "changed_bonds": changed_bonds_out,
        }

    intent_counts = defaultdict(int)
    for meta in templates_out.values():
        for cb in meta["changed_bonds"]:
            intent_counts[cb["intent"]] += 1

    sidecar = {
        "schema_version": SCHEMA_VERSION,
        "template_file": args.templates,
        "template_file_sha256": sha256_hex(open(args.templates).read()),
        "source_dataset": args.dataset_id,
        "source_dataset_revision": f"{args.split} split, {n_reactions} reactions (as re-derived; "
        "see 'Reproduction note' below -- exact per-line rank/count reproduction against the "
        "checked-in template file is not guaranteed, only presence-based intent classification, "
        "which is robust to reordering)",
        "rdchiral_version": importlib.metadata.version("rdchiral"),
        "rdkit_version": rdBase.rdkitVersion,
        "generator": "scripts/generate_ring_context_metadata.py",
        "generator_sha256": sha256_hex(open(__file__).read()),
        "reaction_occurrences_matched": n_matched_reactions,
        "intent_counts": dict(intent_counts),
        "templates": templates_out,
    }

    with open(args.output, "w") as f:
        json.dump(sidecar, f, indent=2, sort_keys=True)
        f.write("\n")
    print(f"Wrote sidecar to {args.output}", flush=True)
    print(f"Intent distribution across all changed bonds: {dict(intent_counts)}", flush=True)


if __name__ == "__main__":
    main()
