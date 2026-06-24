#!/usr/bin/env python3
"""
Fetch FDA-approved small molecules from ChEMBL (Phase 4) for OOD evaluation.
Filters by MW 150-700, heavy atom count 10-60, no salts/mixtures.
Outputs a .smi file for use with renkin-bench.
"""
import json
import subprocess
import sys
import random

OUTPUT = "data/chembl_approved_ood.smi"
LIMIT = 1000       # max per page
MAX_MOLS = 500    # final sample size
SEED = 42

def chembl_get(url):
    result = subprocess.run(["curl", "-s", url], capture_output=True, text=True)
    return json.loads(result.stdout)

def fetch_all_approved():
    offset = 0
    mols = []
    print("Fetching approved drugs from ChEMBL...", flush=True)
    while True:
        url = (
            f"https://www.ebi.ac.uk/chembl/api/data/molecule.json"
            f"?max_phase=4&molecule_type=Small+molecule"
            f"&limit={LIMIT}&offset={offset}"
        )
        data = chembl_get(url)
        batch = data["molecules"]
        if not batch:
            break
        mols.extend(batch)
        print(f"  fetched {len(mols)} / {data['page_meta']['total_count']}", flush=True)
        if len(mols) >= data["page_meta"]["total_count"]:
            break
        offset += LIMIT
    return mols

def filter_mols(mols):
    filtered = []
    for m in mols:
        props = m.get("molecule_properties") or {}
        structs = m.get("molecule_structures") or {}
        smi = structs.get("canonical_smiles", "")
        if not smi:
            continue
        # exclude salts/mixtures (contains '.')
        if "." in smi:
            continue
        mw = props.get("mw_freebase")
        hac = props.get("heavy_atoms")
        try:
            mw = float(mw or 0)
            hac = int(hac or 0)
        except (TypeError, ValueError):
            continue
        if not (150 <= mw <= 700):
            continue
        if not (10 <= hac <= 60):
            continue
        name = m.get("pref_name") or m.get("chembl_id") or "unknown"
        name = name.replace(" ", "_")
        filtered.append((smi, name))
    return filtered

def main():
    mols = fetch_all_approved()
    filtered = filter_mols(mols)
    print(f"\nAfter filtering: {len(filtered)} molecules")

    random.seed(SEED)
    sample = random.sample(filtered, min(MAX_MOLS, len(filtered)))

    with open(OUTPUT, "w") as f:
        f.write("# ChEMBL Phase 4 approved drugs — OOD evaluation set\n")
        f.write(f"# {len(sample)} molecules sampled from {len(filtered)} filtered (seed={SEED})\n")
        for smi, name in sample:
            f.write(f"{smi}\t{name}\n")

    print(f"Saved {len(sample)} molecules to {OUTPUT}")

if __name__ == "__main__":
    main()
