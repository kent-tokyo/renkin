#!/usr/bin/env python3
"""
Extract retrosynthetic SMIRKS templates from USPTO-50k training set,
then simplify them to chematic-compatible basic SMARTS.

Usage:
    python3 scripts/extract_templates.py [--top N] [--output data/templates_extracted.smi]

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


def extract_templates(top_n: int, output_path: str) -> None:
    print("Loading USPTO-50k training set...", flush=True)
    ds = load_dataset("bisectgroup/USPTO_50K", split="train")
    print(f"  {len(ds)} reactions loaded", flush=True)

    counts: Counter = Counter()
    errors = 0

    for i, row in enumerate(ds):
        if i % 5000 == 0:
            print(f"  Processing {i}/{len(ds)}...", flush=True)
        try:
            reaction = {
                "reactants": row["reactants"],
                "products": row["product"],
                "_id": row["id"],
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
        f.write("# RENKIN extracted SMIRKS templates from USPTO-50k training set\n")
        f.write(f"# Source: bisectgroup/USPTO_50K (train split, {len(ds)} reactions)\n")
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
    args = parser.parse_args()

    extract_templates(args.top, args.output)


if __name__ == "__main__":
    main()
