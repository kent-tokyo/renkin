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

A simplified template's LHS pattern can match a real product at more than one
site: the site that was actually the reaction center for that historical
occurrence, AND, incidentally, unrelated sites elsewhere in the same molecule
that happen to share the same local pattern but were never touched by the
reaction. Only the former is real evidence about this template's chemistry;
counting the latter (as an earlier version of this script did) inflates
observation counts past the number of source occurrences and can push a
template that is genuinely NonRing everywhere it's actually applied towards
`Either`, silently permitting the exact ring-opening misapplication #72
describes.

Real reaction centers are identified independently of rdchiral's own template
extraction, directly from the dataset's atom-mapped reactant/product SMILES
(`row["reactants"]`/`row["product"]`, which share one atom-map numbering):
a bond between two mapped atoms that exists in the product but did not exist
between the same two mapped atoms among the reactants was formed by the
reaction (forward) -- equivalently, deleted by this row's retro template. A
template LHS match on the product is only counted as an observation for a
given changed (map_a, map_b) bond if that match's real atom pair is in this
formed-bond set; every other match is an incidental match, excluded from
observations (but counted in `incidental_matches_excluded` for transparency).

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
import platform
import re
import sys
from collections import defaultdict

sys.path.insert(0, __file__.rsplit("/", 1)[0])

try:
    from datasets import load_dataset
    from huggingface_hub import HfApi
    from rdchiral.template_extractor import extract_from_reaction
    from rdkit import Chem, rdBase

    from extract_templates import simplify_smirks, is_valid_for_chematic  # noqa: E402

    HAVE_DEPS = True
except ImportError:  # pragma: no cover -- exercised by scripts/tests without the deps installed
    HAVE_DEPS = False

SCHEMA_VERSION = 2
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


def real_mapped_bonds(mol) -> set:
    """(map_a, map_b) pairs (a < b) for every bond between two REAL,
    dataset-assigned atom-map numbers in an already-parsed molecule (as
    opposed to `mapped_bonds`, which parses a template's local, rdchiral-
    renumbered SMARTS pattern)."""
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


def attribute_bucket(candidate_bonds, formed_bonds):
    """Reduces every raw substructure-match candidate for one (occurrence,
    changed_bond) slot down to a single observation bucket, distinguishing
    genuine reaction-center matches from incidental ones.

    `candidate_bonds`: iterable of (real_map_pair, is_in_ring) for every raw
    match the template's LHS found in this product, translated to real
    (dataset) atom-map pairs. `formed_bonds`: the set of real atom-map pairs
    this specific historical reaction actually formed (== retro-deleted).

    A candidate whose real pair isn't in `formed_bonds` is incidental --
    the template's pattern happened to also match somewhere else in the
    same product that the reaction never touched -- and is excluded before
    the bucket decision. Returns ("ring"|"non_ring"|"ambiguous"|"unknown",
    genuine_dict) where `genuine_dict` maps each genuine real pair actually
    used to its ring-ness, for callers that want to inspect what survived.
    """
    genuine = {pair: is_ring for pair, is_ring in candidate_bonds if pair in formed_bonds}
    statuses = set(genuine.values())
    if len(statuses) > 1:
        return "ambiguous", genuine
    if statuses == {True}:
        return "ring", genuine
    if statuses == {False}:
        return "non_ring", genuine
    return "unknown", genuine


def classify_intent(ring_obs: int, non_ring_obs: int, ambiguous_obs: int, unknown_obs: int) -> str:
    """`ambiguous_obs` (a single occurrence whose distinct real reaction-center
    bonds disagreed on ring-ness) is folded into `either` -- it is exactly the
    same "this template's chemistry isn't consistently one or the other"
    signal `either` already exists to express, just observed within one
    occurrence instead of across occurrences. It is kept as its own counter
    (never merged into ring_obs/non_ring_obs) purely for audit transparency."""
    if ambiguous_obs > 0 or (ring_obs > 0 and non_ring_obs > 0):
        return "either"
    if ring_obs > 0:
        return "ring"
    if non_ring_obs > 0:
        return "non_ring"
    return "unknown"


def resolve_dataset_revision(dataset_id: str, requested: str | None) -> tuple:
    """Returns (revision, resolution_method). An explicit --dataset-revision is
    used as-is; otherwise the current HEAD commit SHA of the dataset repo is
    resolved via the Hub API and pinned, so `source_dataset_revision` in the
    output sidecar is always a real, reproducible commit -- never a
    description of the split/row-count (the prior version of this field)."""
    if requested:
        return requested, "user-provided"
    info = HfApi().dataset_info(dataset_id)
    return info.sha, "resolved-via-HfApi.dataset_info-at-generation-time"


def rows_content_sha256(ds) -> str:
    """Deterministic content hash over every row's id/reactants/product, in
    dataset order, so a later change to the upstream dataset (even without a
    revision bump, or with `requested` overriding auto-resolution) is
    detectable independently of the revision string."""
    h = hashlib.sha256()
    for i, row in enumerate(ds):
        line = f"{row.get('id', str(i))}\t{row['reactants']}\t{row['product']}\n"
        h.update(line.encode("utf-8"))
    return h.hexdigest()


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--templates", default="data/templates_extracted_500.smi")
    ap.add_argument("--output", default="data/ring_context_metadata_500.json")
    ap.add_argument("--dataset-id", default=DATASET_ID)
    ap.add_argument("--split", default=DATASET_SPLIT)
    ap.add_argument(
        "--dataset-revision",
        default=None,
        help="Exact HF dataset commit SHA to pin. If omitted, resolved and pinned "
        "automatically via the Hub API at generation time (never left as a "
        "mutable 'main'/'latest').",
    )
    args = ap.parse_args()

    checked_in = load_checked_in_templates(args.templates)
    print(f"Loaded {len(checked_in)} checked-in templates from {args.templates}", flush=True)

    revision, revision_resolution = resolve_dataset_revision(args.dataset_id, args.dataset_revision)
    print(f"Pinned dataset revision: {revision} ({revision_resolution})", flush=True)

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
            "source_occurrences_matched": 0,
            # (map_a, map_b) -> [ring, non_ring, ambiguous, unknown] observations
            "obs": {pair: [0, 0, 0, 0] for pair in cbonds},
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

    print(f"Loading {args.dataset_id}@{revision} ({args.split} split)...", flush=True)
    ds = load_dataset(args.dataset_id, split=args.split, revision=revision)
    n_reactions = len(ds)
    print(f"  {n_reactions} reactions loaded", flush=True)

    print("Hashing row content for provenance...", flush=True)
    content_sha256 = rows_content_sha256(ds)

    n_extract_errors = 0
    n_product_parse_errors = 0
    n_reactant_parse_errors = 0
    n_incidental_matches_excluded = 0
    n_genuine_matches = 0

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

        meta["source_occurrences_matched"] += 1

        query_mol = meta["query_mol"]
        if query_mol is None:
            for pair in meta["changed_bonds"]:
                meta["obs"][pair][3] += 1  # unknown
            continue

        matches = product_mol.GetSubstructMatches(query_mol, uniquify=True)
        if not matches:
            # LHS was derived from this exact product by rdchiral, so this
            # should be rare; simplification (dropping D/H0/+0 constraints)
            # can occasionally loosen or -- via H-count changes -- tighten
            # the pattern enough to change match behavior. Counts as unknown
            # rather than silently skipped.
            for pair in meta["changed_bonds"]:
                meta["obs"][pair][3] += 1  # unknown
            continue

        reactant_mol = Chem.MolFromSmiles(row["reactants"])
        if reactant_mol is None:
            n_reactant_parse_errors += 1
            for pair in meta["changed_bonds"]:
                meta["obs"][pair][3] += 1  # unknown: can't compute formed bonds
            continue

        formed_bonds = real_mapped_bonds(product_mol) - real_mapped_bonds(reactant_mol)

        for (map_a, map_b) in meta["changed_bonds"]:
            # Every raw match's (real atom-map pair, is_in_ring) -- genuine
            # reaction-center matches and incidental ones are not yet
            # distinguished here; `attribute_bucket` does that, deduping by
            # real atom-map pair so never more than one observation lands
            # per (occurrence, changed_bond).
            candidate_bonds = []
            for match in matches:
                idx_a = match[meta["map_to_query_idx"][map_a]]
                idx_b = match[meta["map_to_query_idx"][map_b]]
                bond = product_mol.GetBondBetweenAtoms(idx_a, idx_b)
                if bond is None:
                    continue
                real_a = product_mol.GetAtomWithIdx(idx_a).GetAtomMapNum()
                real_b = product_mol.GetAtomWithIdx(idx_b).GetAtomMapNum()
                if not real_a or not real_b:
                    continue
                real_pair = (real_a, real_b) if real_a < real_b else (real_b, real_a)
                candidate_bonds.append((real_pair, bond.IsInRing()))

            genuine_raw_count = sum(1 for pair, _ in candidate_bonds if pair in formed_bonds)
            n_genuine_matches += genuine_raw_count
            n_incidental_matches_excluded += len(candidate_bonds) - genuine_raw_count

            bucket, _genuine = attribute_bucket(candidate_bonds, formed_bonds)
            obs = meta["obs"][(map_a, map_b)]
            if bucket == "ring":
                obs[0] += 1
            elif bucket == "non_ring":
                obs[1] += 1
            elif bucket == "ambiguous":
                obs[2] += 1
            else:
                obs[3] += 1  # unknown: no genuine match found

    print(
        f"Processed {n_reactions} reactions ({n_extract_errors} extraction errors, "
        f"{n_product_parse_errors} product-parse errors, {n_reactant_parse_errors} "
        f"reactant-parse errors, all excluded from observation counts). "
        f"{n_genuine_matches} genuine reaction-center matches, "
        f"{n_incidental_matches_excluded} incidental matches excluded.",
        flush=True,
    )

    # Invariant: every changed bond's observations partition exactly the
    # template's matched occurrences -- never more (the bug this rewrite
    # fixes: counting every incidental match as its own observation) and
    # never less (every matched occurrence must land in exactly one bucket).
    for tid, meta in template_meta.items():
        for pair, (ring_obs, non_ring_obs, ambiguous_obs, unknown_obs) in meta["obs"].items():
            total = ring_obs + non_ring_obs + ambiguous_obs + unknown_obs
            assert total == meta["source_occurrences_matched"], (
                f"observation-count invariant violated for {tid} bond {pair}: "
                f"{total} != {meta['source_occurrences_matched']} source occurrences"
            )

    templates_out = {}
    for tid, meta in sorted(template_meta.items()):
        changed_bonds_out = []
        for pair in meta["changed_bonds"]:
            ring_obs, non_ring_obs, ambiguous_obs, unknown_obs = meta["obs"][pair]
            changed_bonds_out.append(
                {
                    "map_a": pair[0],
                    "map_b": pair[1],
                    "operation": "delete",
                    "intent": classify_intent(ring_obs, non_ring_obs, ambiguous_obs, unknown_obs),
                    "ring_observations": ring_obs,
                    "non_ring_observations": non_ring_obs,
                    "ambiguous_observations": ambiguous_obs,
                    "unknown_observations": unknown_obs,
                }
            )
        templates_out[tid] = {
            "smirks_sha256": tid,
            "simplified_smirks": meta["smirks"],
            "count": meta["count"],
            "source_occurrences_matched": meta["source_occurrences_matched"],
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
        "source_dataset_revision": revision,
        "source_dataset_revision_resolution": revision_resolution,
        "source_rows_content_sha256": content_sha256,
        "rdchiral_version": importlib.metadata.version("rdchiral"),
        "rdkit_version": rdBase.rdkitVersion,
        "datasets_version": importlib.metadata.version("datasets"),
        "python_version": platform.python_version(),
        "generator": "scripts/generate_ring_context_metadata.py",
        "generator_sha256": sha256_hex(open(__file__).read()),
        "extract_templates_py_sha256": sha256_hex(
            open(__file__.rsplit("/", 1)[0] + "/extract_templates.py").read()
        ),
        "reaction_occurrences_processed": n_reactions,
        "genuine_matches": n_genuine_matches,
        "incidental_matches_excluded": n_incidental_matches_excluded,
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
