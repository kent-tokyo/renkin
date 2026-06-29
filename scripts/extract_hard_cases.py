#!/usr/bin/env python3
"""Extract unsolved (hard-case) targets from benchmark chunk results.

Reads renkin-bench chunk JSON files and writes the SMILES of every unsolved
target to a single .smi file. This corpus lets future graph-rule / search
changes be measured quickly on the failures that actually matter, instead of
re-running the full USPTO-50k set every time.

Usage:
    python3 scripts/extract_hard_cases.py \
        --chunks data/bench_chunks_phaseB_b100 \
        --out data/hard_cases/uspto_unsolved.smi
"""
import argparse
import glob
import json
import os


def load_first_json(path):
    """Read the first JSON object from a file (tolerates trailing garbage)."""
    with open(path) as f:
        return json.JSONDecoder().raw_decode(f.read())[0]


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--chunks", required=True, help="dir of renkin-bench chunk JSON files")
    ap.add_argument("--out", required=True, help="output .smi path")
    args = ap.parse_args()

    files = sorted(glob.glob(os.path.join(args.chunks, "*.json")))
    if not files:
        raise SystemExit(f"no JSON chunks found in {args.chunks}")

    unsolved = []
    for f in files:
        if os.path.getsize(f) == 0:
            continue
        try:
            d = load_first_json(f)
        except Exception as e:  # noqa: BLE001 — skip corrupt chunks, report at end
            print(f"  skip {os.path.basename(f)}: {e}")
            continue
        for r in d.get("results", []):
            if not r.get("solved", False):
                smi = r.get("smiles", "").strip()
                name = r.get("name", "").strip()
                if smi:
                    unsolved.append(f"{smi}\t{name}" if name else smi)

    os.makedirs(os.path.dirname(args.out), exist_ok=True)
    with open(args.out, "w") as f:
        f.write("# Hard-case corpus: unsolved targets extracted from " + args.chunks + "\n")
        f.write("\n".join(unsolved) + "\n")

    print(f"wrote {len(unsolved)} unsolved targets to {args.out}")


if __name__ == "__main__":
    main()
