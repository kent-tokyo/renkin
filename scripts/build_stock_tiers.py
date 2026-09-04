"""Deterministic nested stock-size tiers for the v0.36.0 scalable-stock pilot.

Freezes a single, reproducible ranking over an already-canonical stock file
(one SMILES per line) so the 10k/100k/1M tiers are *prefixes of one sorted
list* -- never three independently-derived samples that merely happen to
nest. Mirrors scripts/compare_sampling.py's sample_key/build_sample
technique (SHA-256(protocol-version-prefixed value), sort, take a prefix),
applied here to a stock/building-block corpus instead of a target corpus --
kept as its own script rather than extending compare_sampling.py, since
target-sampling and stock-tiering are different corpora/purposes (this
project's "don't conflate two axes under one label" discipline).

Unlike compare_sampling.py, the source file here
(data/building_blocks_emolecules_canonical.smi) is already canonical and
already deduplicated (confirmed via its own manifest) -- no RDKit
canonicalization pass is needed, just a direct hash of each line as-is.
That keeps a 9.48M-line/385MB source tractable via two streaming passes
(rank, then select), never holding every line's text in memory at once --
only a (hash, line_number) tuple per line survives pass 1.

Never "first N lines": the source is sorted (lexicographically, by
canonical SMILES), so a plain head -n would bias every tier toward one
narrow slice of chemical space (confirmed empirically -- the real source
file starts with a long run of boron compounds).

Usage:
    python3 scripts/build_stock_tiers.py \
        --source data/building_blocks_emolecules_canonical.smi \
        --output-dir data/stock_tiers \
        --tier 10000 --tier 100000 --tier 1000000
"""

from __future__ import annotations

import argparse
import hashlib
import json
import sys
from pathlib import Path

PROTOCOL_VERSION = "renkin-stock-tier-v1"


def rank_key(line: str) -> str:
    h = hashlib.sha256()
    h.update(f"{PROTOCOL_VERSION}|".encode("utf-8"))
    h.update(line.encode("utf-8"))
    return h.hexdigest()


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def build_tiers(source_path: Path, tiers: list[int], output_dir: Path) -> dict:
    if not tiers:
        raise ValueError("at least one --tier is required")
    sorted_tiers = sorted(set(tiers))
    max_tier = sorted_tiers[-1]

    # Pass 1: stream the source once, computing (rank_key, line_number) for
    # every non-blank line. Line content itself isn't retained past this
    # loop -- pass 2 re-reads the file -- so peak memory is O(total_lines)
    # tuples, not O(total_lines) full SMILES strings.
    keyed: list[tuple[str, int]] = []
    total_lines = 0
    blank_lines = 0
    with open(source_path, "r", encoding="utf-8") as f:
        for line_number, raw_line in enumerate(f, start=1):
            total_lines += 1
            stripped = raw_line.strip()
            if not stripped:
                blank_lines += 1
                continue
            keyed.append((rank_key(stripped), line_number))

    if len(keyed) < max_tier:
        raise ValueError(
            f"source has only {len(keyed)} non-blank lines, cannot build a "
            f"{max_tier}-line tier"
        )

    keyed.sort(key=lambda t: t[0])
    selected = keyed[:max_tier]  # rank == index into this already-sorted prefix
    rank_by_line_number = {line_no: rank for rank, (_, line_no) in enumerate(selected)}

    # Pass 2: stream the source a second time, writing each selected line to
    # every tier file whose cutoff it falls under. Nesting (10k subset of
    # 100k subset of 1M) is automatic: a line ranked below 10,000 is by
    # definition also ranked below 100,000 and 1,000,000.
    output_dir.mkdir(parents=True, exist_ok=True)
    handles = {
        n: open(output_dir / f"tier_{n}.smi", "w", encoding="utf-8") for n in sorted_tiers
    }
    counts = {n: 0 for n in sorted_tiers}
    try:
        with open(source_path, "r", encoding="utf-8") as f:
            for line_number, raw_line in enumerate(f, start=1):
                stripped = raw_line.strip()
                if not stripped:
                    continue
                rank = rank_by_line_number.get(line_number)
                if rank is None:
                    continue
                for n in sorted_tiers:
                    if rank < n:
                        handles[n].write(stripped + "\n")
                        counts[n] += 1
    finally:
        for h in handles.values():
            h.close()

    tier_manifests = []
    for n in sorted_tiers:
        if counts[n] != n:
            raise AssertionError(f"tier {n}: expected {n} rows, wrote {counts[n]}")
        path = output_dir / f"tier_{n}.smi"
        tier_manifests.append(
            {
                "cutoff": n,
                "actual_row_count": counts[n],
                "output_path": str(path),
                "output_sha256": sha256_file(path),
            }
        )

    return {
        "protocol_version": PROTOCOL_VERSION,
        "source_file": str(source_path),
        "source_file_sha256": sha256_file(source_path),
        "source_total_lines": total_lines,
        "source_blank_lines": blank_lines,
        "source_ranked_lines": len(keyed),
        "hash_function": f'SHA-256("{PROTOCOL_VERSION}|" + line), lowercase hex',
        "tiers": tier_manifests,
        "rebuild_command": (
            f"python3 scripts/build_stock_tiers.py --source {source_path} "
            f"--output-dir {output_dir} " + " ".join(f"--tier {n}" for n in sorted_tiers)
        ),
    }


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    parser.add_argument("--source", required=True)
    parser.add_argument("--output-dir", required=True)
    parser.add_argument("--tier", type=int, action="append", required=True, dest="tiers")
    parser.add_argument("--output-manifest", default=None)
    args = parser.parse_args(argv)

    source_path = Path(args.source)
    output_dir = Path(args.output_dir)
    manifest = build_tiers(source_path, args.tiers, output_dir)

    manifest_path = (
        Path(args.output_manifest) if args.output_manifest else output_dir / "sampling_manifest.json"
    )
    with open(manifest_path, "w", encoding="utf-8") as f:
        json.dump(manifest, f, indent=2, sort_keys=True)
        f.write("\n")

    print(json.dumps(manifest, indent=2, sort_keys=True))
    return 0


if __name__ == "__main__":
    sys.exit(main())
