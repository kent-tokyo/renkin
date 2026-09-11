#!/usr/bin/env python3
"""Explain partial candidate-pool overlap without changing route validity.

This is a diagnostic only.  ``reaction_family_proxy`` is a conservative
functional-group heuristic, not a chemically validated reaction label.  The
script never treats an RDKit match or a stock-reachable candidate as a valid
route.
"""

from __future__ import annotations

import argparse
import hashlib
import json
from collections import Counter
from pathlib import Path
from typing import Any

from rdkit import Chem


def _sha256_file(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def _variants(smiles: str) -> dict[str, str] | None:
    mol = Chem.MolFromSmiles(smiles)
    if mol is None:
        return None
    return {
        "canonical": Chem.MolToSmiles(mol, canonical=True, isomericSmiles=True),
        "stereo_stripped": Chem.MolToSmiles(mol, canonical=True, isomericSmiles=False),
    }


def _motif(mol: Chem.Mol, smarts: str) -> bool:
    pattern = Chem.MolFromSmarts(smarts)
    return pattern is not None and mol.HasSubstructMatch(pattern)


def reaction_family_proxy(target: str, precursors: list[str]) -> str:
    """Return a transparent motif label, never a reaction-family assertion."""
    target_mol = Chem.MolFromSmiles(target)
    precursor_mols = [Chem.MolFromSmiles(value) for value in precursors]
    if target_mol is None or any(mol is None for mol in precursor_mols):
        return "unparseable"
    if _motif(target_mol, "C(=O)N") and any(_motif(mol, "[N;H1,H2,H3]") for mol in precursor_mols):
        return "amide_like"
    if _motif(target_mol, "C(=O)O") and any(_motif(mol, "[O;H1]") for mol in precursor_mols):
        return "ester_like"
    if _motif(target_mol, "S(=O)(=O)N"):
        return "sulfonamide_like"
    if _motif(target_mol, "C(=O)O") and any(_motif(mol, "[O;H1,H0]") for mol in precursor_mols):
        return "carbonyl_oxygen_like"
    return "other_or_unknown"


def _load_stock(path: Path | None) -> set[str] | None:
    if path is None:
        return None
    values = set()
    for line in path.read_text().splitlines():
        value = line.strip().split()[0] if line.strip() else ""
        if value and not value.startswith("#"):
            variants = _variants(value)
            if variants:
                values.add(variants["canonical"])
    return values


def _load_jsonl_artifact(lines: list[str]) -> dict[str, Any]:
    proposals: dict[str, list[dict[str, Any]]] = {}
    for line_number, line in enumerate(lines, 1):
        if not line.strip():
            continue
        row = json.loads(line)
        target = row.get("target_smiles") or row.get("target_id")
        precursors = row.get("precursor_smiles")
        if not isinstance(target, str) or not isinstance(precursors, list):
            raise ValueError(f"line {line_number}: JSONL row lacks target_smiles/precursor_smiles")
        candidate = {
            "candidate_id": row.get("candidate_id"),
            "precursors": precursors,
            "provenance": {
                "source_rank": row.get("best_upstream_rank"),
                "template_id": (row.get("sources") or [{}])[0].get("template_id"),
            },
        }
        keys = {target}
        variants = _variants(target)
        if variants:
            keys.add(variants["canonical"])
        for key in keys:
            proposals.setdefault(key, []).append(candidate)
    return {"proposals": proposals}


def load_artifact(path: Path) -> dict[str, Any]:
    """Load the static JSON artifact or the candidate-pool JSONL boundary."""
    if path.suffix == ".jsonl":
        return _load_jsonl_artifact(path.read_text().splitlines())
    value = json.loads(path.read_text())
    if not isinstance(value, dict) or not isinstance(value.get("proposals"), dict):
        raise ValueError("artifact must contain a proposals object")
    return value


def _reachable(target: str, proposals: dict[str, list[dict[str, Any]]], stock: set[str], memo: dict[str, bool], visiting: set[str]) -> bool:
    variants = _variants(target)
    key = variants["canonical"] if variants else target
    if key in stock:
        return True
    if key in memo:
        return memo[key]
    if key in visiting:
        return False
    visiting.add(key)
    candidates = proposals.get(target, []) + proposals.get(key, [])
    result = any(
        bool(candidate.get("precursors"))
        and all(_reachable(child, proposals, stock, memo, visiting) for child in candidate["precursors"])
        for candidate in candidates
    )
    visiting.remove(key)
    memo[key] = result
    return result


def _representation_status(expected: str, candidates: list[str]) -> str:
    expected_variants = _variants(expected)
    if expected_variants is None:
        return "expected_unparseable"
    candidate_variants = [_variants(value) for value in candidates]
    if any(value and value["canonical"] == expected_variants["canonical"] for value in candidate_variants):
        return "rdkit_canonical_match"
    if any(value and value["stereo_stripped"] == expected_variants["stereo_stripped"] for value in candidate_variants):
        return "stereo_only_difference"
    if any(value is None for value in candidate_variants):
        return "candidate_unparseable_or_not_equivalent"
    return "not_equivalent_by_rdkit"


def classify(atlas: dict[str, Any], artifact: dict[str, Any], stock: set[str] | None = None) -> dict[str, Any]:
    proposals = artifact["proposals"]
    records = []
    reachability_memo: dict[str, bool] = {}
    for edge in atlas["records"]:
        if not edge.get("canonicalization_evaluable") or edge.get("exact_precursor_multiset_present"):
            continue
        target = edge["canonical_target"]
        candidates = proposals.get(target, [])
        expected = edge["canonical_precursors"]
        if not candidates:
            continue
        overlaps = [
            candidate for candidate in candidates
            if len(set(expected) & set(candidate.get("precursors", [])))
        ]
        if not overlaps or max(len(set(expected) & set(candidate.get("precursors", []))) for candidate in overlaps) >= len(expected):
            continue
        best = max(overlaps, key=lambda candidate: len(set(expected) & set(candidate.get("precursors", []))))
        missing = [value for value in expected if value not in best.get("precursors", [])]
        missing_status = []
        for value in missing:
            variants = _variants(value)
            canonical = variants["canonical"] if variants else value
            direct_stock = canonical in stock if stock is not None else None
            missing_status.append({
                "precursor": value,
                "representation_status": _representation_status(
                    value, [item for candidate in candidates for item in candidate.get("precursors", [])]
                ),
                "stock_status": (
                    "direct_stock" if direct_stock else "downstream_or_unavailable"
                    if stock is not None else "not_measured"
                ),
            })
        records.append({
            "target_id": edge["target_id"],
            "edge_index": edge["edge_index"],
            "canonical_target": target,
            "best_candidate_id": best.get("candidate_id"),
            "best_overlap_count": len(set(expected) & set(best.get("precursors", []))),
            "expected_precursor_count": len(expected),
            "reaction_family_proxy": reaction_family_proxy(target, expected),
            "missing_precursors": missing_status,
            "candidate_stock_reachable": (
                _reachable(target, proposals, stock, reachability_memo, set()) if stock is not None else None
            ),
        })
    return {
        "schema_version": "renkin-partial-overlap-diagnostic/1",
        "evidence_level": "heuristic_diagnostic_only",
        "route_validity_unchanged": True,
        "record_count": len(records),
        "family_proxy_counts": dict(sorted(Counter(r["reaction_family_proxy"] for r in records).items())),
        "representation_counts": dict(sorted(Counter(
            item["representation_status"]
            for record in records for item in record["missing_precursors"]
        ).items())),
        "stock_status_counts": dict(sorted(Counter(
            item["stock_status"]
            for record in records for item in record["missing_precursors"]
        ).items())),
        "records": records,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--atlas", required=True, type=Path)
    parser.add_argument("--artifact", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--stock", type=Path)
    args = parser.parse_args()
    result = classify(
        json.loads(args.atlas.read_text()),
        load_artifact(args.artifact),
        _load_stock(args.stock),
    )
    result["inputs"] = {
        "atlas_sha256": _sha256_file(args.atlas),
        "artifact_sha256": _sha256_file(args.artifact),
        "stock_sha256": _sha256_file(args.stock) if args.stock else None,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"record_count": result["record_count"], "family_proxy_counts": result["family_proxy_counts"]}))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
