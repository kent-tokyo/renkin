#!/usr/bin/env python3
"""
Extract retrosynthetic SMIRKS templates from a reaction dataset,
then simplify them to chematic-compatible basic SMARTS.

Usage:
    # HuggingFace dataset (default: USPTO-50k)
    python3 scripts/extract_templates.py [--top N] [--output data/templates_extracted.smi]

    # Local reactions file (one reaction SMILES per line: reactants>>products)
    python3 scripts/extract_templates.py --reactions reactions.smiles --top 50000 \
        --output data/templates_extracted_50000.smi

Output format (one template per line, tab-separated):
    <simplified_SMIRKS>  <count>
"""

import argparse
import re
from collections import Counter

from datasets import load_dataset
from rdchiral.template_extractor import extract_from_reaction
from rdkit import Chem
from rdkit.Chem import AllChem


def simplify_atom(atom_smarts: str) -> str:
    """
    Strip chematic-unsupported constraints from a bracket atom.

    Keeps: element symbol, H-count (e.g. NH2), aromaticity (lowercase), atom map (:N).
    Removes: D (degree), +0/+1/-1 (charge as constraint), semicolons, H0.

    Examples:
      [O;D1;H0:3]  → [O:3]
      [NH2;D1;+0:1] → [NH2:1]
      [c;H0;D3;+0:1] → [c:1]
      [C;H0;D3;+0:1] → [C:1]
      [OH;D1;+0:1]  → [OH:1]
      [N+;H0;D3:1]  → [N+:1]   (keep real charge +/-)
      [O-]          → [O-]     (keep real charge)
    """
    # Extract atom map if present
    map_match = re.search(r':(\d+)', atom_smarts)
    atom_map = f":{map_match.group(1)}" if map_match else ""

    # Get the content inside brackets
    inner = atom_smarts[1:-1]  # strip [ ]

    # Remove atom map from inner for processing
    inner_no_map = re.sub(r':\d+$', '', inner)

    # Split by semicolons (AND logic) — keep only first part (element/H/charge)
    parts = inner_no_map.split(';')
    base = parts[0]

    # Remove D-constraints within the kept part: D1, D2, D3, D4
    base = re.sub(r'D\d+', '', base)
    # Remove H0 (zero explicit H — often implicit anyway)
    base = re.sub(r'H0', '', base)
    # Remove +0 (neutral charge constraint — we keep explicit +/- though)
    base = re.sub(r'\+0', '', base)
    # Clean up any trailing/leading punctuation
    base = base.strip(';').strip()

    # Reassemble bracket atom
    result = f"[{base}{atom_map}]"
    # Remove empty brackets that might result
    if result in ('[]', '[:]'):
        return ''
    return result


def simplify_smirks(smirks: str) -> str:
    """
    Convert an rdchiral SMIRKS to chematic-compatible basic SMARTS.
    Processes bracket atoms to strip unsupported constraints.
    """
    def replace_bracket(m):
        return simplify_atom(m.group(0))

    return re.sub(r'\[[^\]]+\]', replace_bracket, smirks)


def is_valid_for_chematic(smirks: str) -> bool:
    """
    Quick heuristic check: does the simplified SMIRKS look parseable?
    We try RDKit as a proxy (chematic subset overlaps with basic RDKit SMARTS).
    """
    try:
        parts = smirks.split('>>')
        if len(parts) != 2:
            return False
        reactant = parts[0]
        # Try parsing reactant side with RDKit
        mol = Chem.MolFromSmarts(reactant)
        return mol is not None
    except Exception:
        return False


def load_reactions_from_file(path: str) -> list:
    """Load reactions from a plain text file (one reaction SMILES per line)."""
    rows = []
    with open(path) as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith('#'):
                continue
            # Accept tab-separated (first column is reaction SMILES) or bare SMILES
            rxn = line.split('\t')[0]
            if '>>' not in rxn:
                continue
            reactants, _, products = rxn.partition('>>')
            rows.append({
                "reactants": reactants,
                "products": products,
                "_id": str(len(rows)),
            })
    return rows


def extract_templates(top_n: int, output_path: str,
                      reactions_path: str | None = None,
                      dataset_id: str = "bisectgroup/USPTO_50K",
                      split: str = "train") -> None:
    if reactions_path:
        print(f"Loading reactions from {reactions_path}...", flush=True)
        rows = load_reactions_from_file(reactions_path)
        source_desc = reactions_path
    else:
        print(f"Loading {dataset_id} ({split} split)...", flush=True)
        ds = load_dataset(dataset_id, split=split)
        rows = ds
        source_desc = f"{dataset_id} ({split} split, {len(ds)} reactions)"
    print(f"  {len(rows)} reactions loaded", flush=True)

    counts: Counter = Counter()
    errors = 0

    for i, row in enumerate(rows):
        if i % 5000 == 0:
            print(f"  Processing {i}/{len(rows)}...", flush=True)
        try:
            reaction = {
                "reactants": row["reactants"],
                "products": row["products"] if "products" in row else row.get("product", ""),
                "_id": row["_id"] if "_id" in row else row.get("id", str(i)),
            }
            result = extract_from_reaction(reaction)
            template = result.get("reaction_smarts")
            if template:
                counts[template] += 1
        except Exception:
            errors += 1

    print(f"\nExtracted {len(counts)} unique templates ({errors} errors)", flush=True)

    # Simplify and deduplicate
    simplified: Counter = Counter()
    for smirks, count in counts.items():
        simple = simplify_smirks(smirks)
        if is_valid_for_chematic(simple):
            simplified[simple] += count

    print(f"After simplification: {len(simplified)} unique chematic-compatible templates",
          flush=True)

    top = simplified.most_common(top_n)
    print(f"Writing top {len(top)} templates to {output_path}", flush=True)

    with open(output_path, "w") as f:
        f.write("# RENKIN extracted SMIRKS templates\n")
        f.write(f"# Source: {source_desc}\n")
        f.write("# Tool: rdchiral + simplification for chematic compatibility\n")
        f.write("# Format: SMIRKS<TAB>count\n")
        for smirks, count in top:
            f.write(f"{smirks}\t{count}\n")

    print(f"Done. Top template count: {top[0][1] if top else 0}", flush=True)
    print("\nTop 10 simplified templates:", flush=True)
    for smirks, count in top[:10]:
        print(f"  {count:5d}x  {smirks[:100]}", flush=True)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--top", type=int, default=300,
                        help="Number of most frequent templates to keep (default: 300)")
    parser.add_argument("--output", default="data/templates_extracted.smi",
                        help="Output file path")
    parser.add_argument("--reactions", default=None,
                        help="Local reaction SMILES file (one per line: reactants>>products). "
                             "Takes precedence over --dataset when specified.")
    parser.add_argument("--dataset", default="bisectgroup/USPTO_50K",
                        help="HuggingFace dataset ID (default: bisectgroup/USPTO_50K)")
    parser.add_argument("--split", default="train",
                        help="HuggingFace dataset split (default: train)")
    args = parser.parse_args()

    extract_templates(args.top, args.output,
                      reactions_path=args.reactions,
                      dataset_id=args.dataset,
                      split=args.split)


if __name__ == "__main__":
    main()
