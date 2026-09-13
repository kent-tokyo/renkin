"""Audit provenance gates for a Phase 55.6 candidate cohort.

The audit is deliberately fail-closed.  It checks exact target IDs and
canonical product structures against supplied split files, but it cannot infer
whether an unpublished training corpus or template extractor saw a molecule.
Unknown provenance therefore remains a blocker instead of being treated as
independent.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
from pathlib import Path

try:
    from rdkit import Chem, RDLogger
    from rdkit.Chem.Scaffolds import MurckoScaffold

    RDLogger.DisableLog("rdApp.*")
    HAVE_RDKIT = True
except ImportError:  # pragma: no cover
    HAVE_RDKIT = False

MAX_FILE_BYTES = 256 * 1024 * 1024


def _regular_file(path: str | Path) -> Path:
    candidate = Path(path)
    metadata = os.lstat(candidate)
    if stat.S_ISLNK(metadata.st_mode):
        raise ValueError(f"input must not be a symlink: {candidate}")
    if not stat.S_ISREG(metadata.st_mode):
        raise ValueError(f"input must be a regular file: {candidate}")
    if metadata.st_size > MAX_FILE_BYTES:
        raise ValueError(f"input exceeds {MAX_FILE_BYTES} bytes: {candidate}")
    return candidate


def sha256_file(path: str | Path) -> str:
    candidate = _regular_file(path)
    digest = hashlib.sha256()
    with candidate.open("rb") as handle:
        for chunk in iter(lambda: handle.read(65536), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _canonical_product(row: dict) -> str | None:
    supplied = row.get("canonical_smiles")
    if isinstance(supplied, str) and supplied:
        return supplied
    product = row.get("product") or row.get("product_smiles")
    if not isinstance(product, str) or not product:
        return None
    if not HAVE_RDKIT:
        raise RuntimeError("rdkit is required to canonicalize raw split products")
    mol = Chem.MolFromSmiles(product)
    if mol is None:
        return None
    return Chem.MolToSmiles(mol, canonical=True)


def _scaffold_key(canonical: str) -> str | None:
    if not HAVE_RDKIT:
        return None
    mol = Chem.MolFromSmiles(canonical)
    if mol is None:
        return None
    for atom in mol.GetAtoms():
        atom.SetAtomMapNum(0)
    scaffold = MurckoScaffold.GetScaffoldForMol(mol)
    if scaffold.GetNumAtoms() == 0:
        return "<acyclic>"
    return Chem.MolToSmiles(scaffold, canonical=True)


def _jsonl_rows(path: str | Path) -> list[dict]:
    candidate = _regular_file(path)
    rows = []
    with candidate.open(encoding="utf-8") as handle:
        for line_number, line in enumerate(handle, start=1):
            if not line.strip():
                continue
            row = json.loads(line)
            if not isinstance(row, dict):
                raise ValueError(f"row {line_number} is not an object: {candidate}")
            rows.append(row)
    return rows


def _load_manifest(path: str | Path) -> dict:
    candidate = _regular_file(path)
    value = json.loads(candidate.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or not isinstance(value.get("targets"), list):
        raise ValueError("cohort manifest must contain a targets array")
    return value


def _split_values(value: object) -> set[str]:
    found: set[str] = set()
    if isinstance(value, dict):
        for key, child in value.items():
            normalized = str(key).lower()
            if (
                normalized in {"split", "upstream_split", "source_split"}
                or normalized.endswith("_split")
            ) and isinstance(child, str):
                found.add(child.lower())
            elif normalized.endswith("_split") and isinstance(child, dict):
                # Frozen manifests often encode provenance as train_split /
                # val_split objects rather than a scalar split field.
                found.add(normalized.removesuffix("_split"))
            found.update(_split_values(child))
    elif isinstance(value, list):
        for child in value:
            found.update(_split_values(child))
    return found


def _load_provenance(paths: list[str | Path]) -> list[dict]:
    evidence = []
    for path in paths:
        candidate = _regular_file(path)
        value = json.loads(candidate.read_text(encoding="utf-8"))
        if not isinstance(value, dict):
            raise ValueError(f"provenance artifact must be an object: {candidate}")
        split_values = _split_values(value)
        source_file = value.get("source_file")
        if isinstance(source_file, str) and "test" in Path(source_file).name.lower():
            # The frozen sampler manifest predates an explicit source_split
            # field; its source filename is the authoritative TEST marker.
            split_values.add("test")
        evidence.append(
            {
                "path": str(candidate),
                "sha256": sha256_file(candidate),
                "split_values": sorted(split_values),
                "ordered_list_sha256": value.get("ordered_list_sha256"),
            }
        )
    return evidence


def audit_cohort(
    manifest_path: str | Path,
    split_paths: list[str | Path],
    source_provenance_paths: list[str | Path] | None = None,
    training_provenance_paths: list[str | Path] | None = None,
) -> dict:
    manifest = _load_manifest(manifest_path)
    targets = manifest["targets"]
    target_ids = set()
    canonical = set()
    for index, row in enumerate(targets, start=1):
        if not isinstance(row, dict):
            raise ValueError(f"cohort target {index} is not an object")
        target_id = row.get("target_id")
        value = row.get("canonical_smiles")
        if not isinstance(target_id, str) or not target_id:
            raise ValueError(f"cohort target {index} has no target_id")
        if not isinstance(value, str) or not value:
            raise ValueError(f"cohort target {index} has no canonical_smiles")
        if target_id in target_ids or value in canonical:
            raise ValueError("cohort contains duplicate target identity")
        target_ids.add(target_id)
        canonical.add(value)

    overlaps = []
    unparseable = []
    candidate_scaffolds = {
        value for value in (_scaffold_key(item) for item in canonical) if value is not None
    }
    scaffold_overlaps = []
    split_details = []
    for path in split_paths:
        split_ids = set()
        split_canonical = set()
        split_scaffolds = set()
        for row in _jsonl_rows(path):
            value = _canonical_product(row)
            if value is None:
                unparseable.append({"path": str(path), "target_id": row.get("id")})
                continue
            split_canonical.add(value)
            scaffold = _scaffold_key(value)
            if scaffold is not None:
                split_scaffolds.add(scaffold)
            target_id = row.get("id") or row.get("target_id")
            if isinstance(target_id, str):
                split_ids.add(target_id)
        id_overlap = sorted(target_ids & split_ids)
        structure_overlap = sorted(canonical & split_canonical)
        if id_overlap or structure_overlap:
            overlaps.append(
                {
                    "path": str(path),
                    "target_id_overlap": id_overlap,
                    "canonical_overlap_count": len(structure_overlap),
                    "canonical_overlap": structure_overlap,
                }
            )
        shared_scaffolds = sorted(candidate_scaffolds & split_scaffolds)
        scaffold_overlaps.append(
            {
                "path": str(path),
                "cohort_unique_scaffolds": len(candidate_scaffolds),
                "split_unique_scaffolds": len(split_scaffolds),
                "shared_scaffold_count": len(shared_scaffolds),
                "shared_scaffold_fraction_of_cohort": (
                    len(shared_scaffolds) / len(candidate_scaffolds)
                    if candidate_scaffolds
                    else 0.0
                ),
            }
        )
        split_details.append(
            {
                "path": str(path),
                "sha256": sha256_file(path),
                "rows": len(split_ids),
                "parseable_products": len(split_canonical),
            }
        )

    blockers = []
    if overlaps:
        blockers.append("split_overlap")
    if unparseable:
        blockers.append("unparseable_split_products")
    if manifest.get("independence_status") != "pending_provenance_audit":
        blockers.append("unexpected_manifest_status")
    source_evidence = _load_provenance(source_provenance_paths or [])
    training_evidence = _load_provenance(training_provenance_paths or [])
    source_hash = manifest.get("source", {}).get("sha256")
    source_hash_matches = any(
        source_hash == evidence.get("ordered_list_sha256")
        for evidence in source_evidence
    )
    if not source_evidence or not source_hash_matches:
        blockers.append("source_provenance_missing_or_mismatch")
    source_is_test = any("test" in value for evidence in source_evidence for value in evidence["split_values"])
    if not source_is_test:
        blockers.append("source_split_not_proven")
    training_has_evidence = bool(training_evidence) and all(
        evidence["split_values"] and not any("test" in value for value in evidence["split_values"])
        for evidence in training_evidence
    )
    if not training_has_evidence:
        blockers.append("unverified_training_and_template_provenance")
    return {
        "eligible": not blockers,
        "manifest": {"path": str(manifest_path), "sha256": sha256_file(manifest_path)},
        "cohort_size": len(targets),
        "split_checks": split_details,
        "overlaps": overlaps,
        "scaffold_overlaps": scaffold_overlaps,
        "unparseable_split_products": unparseable,
        "blockers": blockers,
        "source_provenance": source_evidence,
        "training_provenance": training_evidence,
        "provenance_status": "pending_provenance_audit" if blockers else "audited",
    }


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--split", action="append", required=True)
    parser.add_argument("--source-provenance", action="append", default=[])
    parser.add_argument("--training-provenance", action="append", default=[])
    parser.add_argument("--output", required=True)
    args = parser.parse_args()
    result = audit_cohort(
        args.manifest,
        args.split,
        source_provenance_paths=args.source_provenance,
        training_provenance_paths=args.training_provenance,
    )
    output = Path(args.output)
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    print(json.dumps({"eligible": result["eligible"], "blockers": result["blockers"]}))
    return 0 if result["eligible"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
