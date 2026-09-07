#!/usr/bin/env python3
"""Convert direct-retro model JSONL into RENKIN's static generator fixture.

The converter is intentionally offline and dependency-free.  It does not
claim that a model proposal is chemically valid; the Rust loader and search
pipeline still perform the transport, SMILES, stock, and route-integrity
checks.  Input rows must contain ``target_smiles`` and ``precursor_smiles``;
``candidate_id`` and ``model_confidence`` are optional.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import defaultdict
from pathlib import Path


def sha256_file(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def load_rows(path: Path, generator_id: str, generator_version: str, max_per_target: int) -> dict:
    proposals: dict[str, list[dict]] = defaultdict(list)
    seen_ids: dict[str, set[str]] = defaultdict(set)
    source_hash = sha256_file(path)
    with path.open(encoding="utf-8") as stream:
        for line_number, line in enumerate(stream, 1):
            if not line.strip():
                continue
            try:
                row = json.loads(line)
            except json.JSONDecodeError as exc:
                raise ValueError(f"line {line_number}: invalid JSON: {exc}") from exc
            if not isinstance(row, dict):
                raise ValueError(f"line {line_number}: row must be an object")
            target = row.get("target_smiles")
            precursors = row.get("precursor_smiles")
            if not isinstance(target, str) or not target:
                raise ValueError(f"line {line_number}: target_smiles must be a non-empty string")
            if (
                not isinstance(precursors, list)
                or not precursors
                or any(not isinstance(value, str) or not value for value in precursors)
            ):
                raise ValueError(
                    f"line {line_number}: precursor_smiles must be a non-empty string list"
                )
            candidate_id = row.get("candidate_id", f"line-{line_number}")
            if not isinstance(candidate_id, str) or not candidate_id:
                raise ValueError(f"line {line_number}: candidate_id must be a non-empty string")
            if candidate_id in seen_ids[target]:
                raise ValueError(f"line {line_number}: duplicate candidate_id for target {target!r}")
            if len(proposals[target]) >= max_per_target:
                continue
            confidence = row.get("model_confidence")
            if confidence is not None and (
                not isinstance(confidence, (int, float)) or not 0.0 <= confidence <= 1.0
            ):
                raise ValueError(f"line {line_number}: model_confidence must be in [0,1]")
            seen_ids[target].add(candidate_id)
            proposals[target].append(
                {
                    "candidate_id": candidate_id,
                    "target": target,
                    "precursors": precursors,
                    "atom_mapping": row.get("atom_mapping"),
                    "provenance": {
                        "generator_id": generator_id,
                        "generator_version": generator_version,
                        "artifact_sha256": source_hash,
                        "source_rank": len(proposals[target]),
                        "model_confidence": confidence,
                    },
                }
            )
    return {"schema_version": 1, "proposals": dict(proposals)}


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, required=True, help="model proposal JSONL")
    parser.add_argument("--artifact-output", type=Path, required=True)
    parser.add_argument("--manifest-output", type=Path, required=True)
    parser.add_argument("--generator-id", required=True)
    parser.add_argument("--generator-version", required=True)
    parser.add_argument("--max-per-target", type=int, default=1000)
    args = parser.parse_args(argv)
    if args.max_per_target <= 0:
        parser.error("--max-per-target must be positive")
    artifact = load_rows(args.input, args.generator_id, args.generator_version, args.max_per_target)
    artifact_bytes = json.dumps(
        artifact, ensure_ascii=False, sort_keys=True, separators=(",", ":")
    ).encode("utf-8")
    args.artifact_output.write_bytes(artifact_bytes)
    manifest = {
        "schema_version": 1,
        "model_id": args.generator_id,
        "model_kind": "retro_generator",
        "model_version": args.generator_version,
        "model_sha256": "sha256:" + hashlib.sha256(artifact_bytes).hexdigest(),
        "input_schema": "target-smiles-precursor-smiles-jsonl-v1",
        "output_semantics": "rank",
        "license": "user-supplied-model-output",
        "ood_abstain": True,
    }
    args.manifest_output.write_text(
        json.dumps(manifest, ensure_ascii=False, sort_keys=True, indent=2) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
