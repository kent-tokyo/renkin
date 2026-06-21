#!/usr/bin/env python3
"""
eMolecules Building Blocks Preprocessor

Downloads and normalises the eMolecules free-tier SMILES file for use as
RENKIN's building-block library.

Usage:
    # Download (one-time, ~300 MB):
    curl -O https://downloads.emolecules.com/free/2024-01-01/version.smi.gz
    gunzip version.smi.gz

    # Process:
    python3 scripts/prepare_emolecules.py version.smi data/building_blocks_3m.smi

    # Benchmark with the result:
    ./target/release/renkin-bench \\
        --input data/uspto50k_test.smi \\
        --building-blocks data/building_blocks_3m.smi \\
        --depth 3 --beam-width 50

Requires:
    pip install rdkit
"""

import sys
import argparse
from pathlib import Path


def parse_args():
    p = argparse.ArgumentParser(description="Prepare eMolecules SMILES for RENKIN")
    p.add_argument("input", help="Raw eMolecules .smi file")
    p.add_argument("output", help="Output SMILES file (one per line)")
    p.add_argument("--max-mw", type=float, default=500.0, help="Max molecular weight (default: 500)")
    p.add_argument("--max-atoms", type=int, default=35, help="Max heavy atom count (default: 35)")
    p.add_argument("--max-rings", type=int, default=5, help="Max ring count (default: 5)")
    p.add_argument("--limit", type=int, default=0, help="Max output entries (0 = no limit)")
    p.add_argument("--verbose", action="store_true", help="Show progress")
    return p.parse_args()


def main():
    args = parse_args()

    try:
        from rdkit import Chem
        from rdkit.Chem import Descriptors, rdMolDescriptors
    except ImportError:
        print("ERROR: RDKit not found. Install with: pip install rdkit", file=sys.stderr)
        sys.exit(1)

    import gzip as _gzip

    input_path = Path(args.input)
    output_path = Path(args.output)
    output_path.parent.mkdir(parents=True, exist_ok=True)

    # Support gzip-compressed input directly
    open_fn = _gzip.open if input_path.suffix == ".gz" else open

    total = 0
    accepted = 0
    seen = set()

    with open_fn(input_path, "rt") as fin, open(output_path, "w") as fout:
        fout.write("# RENKIN building blocks from eMolecules\n")
        fout.write(f"# Source: {input_path.name}\n")
        fout.write(f"# Filters: MW<={args.max_mw}, atoms<={args.max_atoms}, rings<={args.max_rings}\n")

        for line in fin:
            line = line.strip()
            if not line or line.startswith("#"):
                continue

            parts = line.split()
            smiles = parts[0]
            # eMolecules format: isosmiles version_id parent_id — skip header row
            if smiles == "isosmiles":
                continue
            name = parts[1] if len(parts) > 1 else ""
            total += 1

            if total % 100_000 == 0 and args.verbose:
                print(f"  processed {total:,} | accepted {accepted:,}", file=sys.stderr)

            mol = Chem.MolFromSmiles(smiles)
            if mol is None:
                continue

            # Filter by molecular weight
            mw = Descriptors.MolWt(mol)
            if mw > args.max_mw:
                continue

            # Filter by atom count
            n_atoms = mol.GetNumHeavyAtoms()
            if n_atoms > args.max_atoms:
                continue

            # Filter by ring count
            n_rings = rdMolDescriptors.CalcNumRings(mol)
            if n_rings > args.max_rings:
                continue

            # Normalize SMILES
            canon = Chem.MolToSmiles(mol, canonical=True)
            if canon in seen:
                continue
            seen.add(canon)

            if name:
                fout.write(f"{canon}\t{name}\n")
            else:
                fout.write(f"{canon}\n")

            accepted += 1
            if args.limit and accepted >= args.limit:
                break

    print(f"Done: {total:,} input → {accepted:,} accepted → {output_path}", file=sys.stderr)


if __name__ == "__main__":
    main()
