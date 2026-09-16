#!/usr/bin/env python3
"""Generate a local O7 operational-validation receipt from a public procedure.

The fixture is deliberately a small, attributable transcription of a public
patent's numeric fields.  This script never downloads source material and does
not assert experimental, safety, forward-replay, PMI, or E-factor validity.
It proves only the local evidence-chain operation: source artifact -> audit ->
metric sidecar -> canonical export -> fresh-process re-audit.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import shutil
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
FIXTURE_DIR = ROOT / "tests" / "fixtures" / "o7"
ROUTE = FIXTURE_DIR / "ethyl_benzoate_cn115677497_route.json"
STOCK = FIXTURE_DIR / "ethyl_benzoate_cn115677497_stock.smi"
SOURCE = FIXTURE_DIR / "ethyl_benzoate_cn115677497_procedure.json"


def canonical_sha256(value: object) -> str:
    """Match RENKIN's JSON hashing: sorted object keys and compact JSON."""
    material = json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False)
    return "sha256:" + hashlib.sha256(material.encode("utf-8")).hexdigest()


def run(binary: Path, *args: str) -> dict:
    completed = subprocess.run(
        [str(binary), *args],
        cwd=ROOT,
        check=True,
        text=True,
        capture_output=True,
    )
    return json.loads(completed.stdout)


def sha256_file(path: Path) -> str:
    return "sha256:" + hashlib.sha256(path.read_bytes()).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--binary",
        type=Path,
        default=ROOT / "target" / "release" / "renkin",
        help="RENKIN binary built from the candidate commit",
    )
    parser.add_argument(
        "--output-dir",
        type=Path,
        required=True,
        help="new directory for generated local receipts",
    )
    args = parser.parse_args()

    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"RENKIN binary does not exist: {binary}")
    output_dir = args.output_dir.resolve()
    if output_dir.exists():
        parser.error(f"refusing to overwrite existing output directory: {output_dir}")
    output_dir.mkdir(parents=True)

    initial = run(
        binary,
        "audit-route",
        str(ROUTE),
        "--stock",
        str(STOCK),
        "--interchange",
        "--output",
        "json",
    )
    (output_dir / "initial-audit.json").write_text(
        json.dumps(initial, indent=2) + "\n", encoding="utf-8"
    )
    route = initial["routes"][0]
    route_id = route["normalized_route_sha256"]
    interchange = initial["route_interchange"][0]
    interchange_path = output_dir / "interchange-v1.json"
    interchange_path.write_text(json.dumps(interchange, indent=2) + "\n", encoding="utf-8")

    source_artifact = json.loads(SOURCE.read_text(encoding="utf-8"))
    source_sha256 = canonical_sha256(source_artifact)
    sidecar = {
        "schema_version": 1,
        "source_artifact": source_artifact,
        "ledgers": [
            {
                "schema_version": 1,
                "route_id": route_id,
                "scope": "route",
                "boundary": {
                    "description": "Reported production batch; intended accounting boundary includes unreported water and workup.",
                    "includes_water": True,
                    "includes_workup": True,
                    "recycling_policy": "No recovery credit: the source does not report a recycling allocation."
                },
                "product": {"value": 956.8, "unit": "kilogram"},
                "inputs": [
                    {"category": "starting_material", "amount": {"value": 940.2, "unit": "kilogram"}, "label": "benzoyl chloride"},
                    {"category": "starting_material", "amount": {"value": 318.2, "unit": "kilogram"}, "label": "ethanol"}
                ],
                "required_categories": ["starting_material", "water", "workup"],
                "waste": None,
                "method": "O7 operational validation: source quantities only; no density, yield, utility, waste, or workup mass inferred.",
                "source_sha256": source_sha256
            }
        ]
    }
    sidecar_path = output_dir / "metrics-sidecar.json"
    sidecar_path.write_text(json.dumps(sidecar, indent=2) + "\n", encoding="utf-8")

    reaudited = run(
        binary,
        "audit-route",
        str(interchange_path),
        "--format",
        "interchange",
        "--stock",
        str(STOCK),
        "--route-metrics-sidecar",
        str(sidecar_path),
        "--output",
        "json",
    )
    (output_dir / "reaudit.json").write_text(
        json.dumps(reaudited, indent=2) + "\n", encoding="utf-8"
    )

    receipt = reaudited["route_metrics"][0][0]
    assert reaudited["routes"][0]["normalized_route_sha256"] == route_id
    assert receipt["process_mass_intensity"]["status"] == "not_evaluable"
    assert receipt["e_factor"]["status"] == "not_evaluable"
    assert set(receipt["coverage"]["missing_categories"]) == {"water", "workup"}
    assert reaudited["route_metrics_sidecar"]["source_artifact_sha256"] == source_sha256
    assert "patents.google.com" not in json.dumps(reaudited)

    manifest = {
        "schema_version": 1,
        "purpose": "O7 operational-validation receipt; not a chemical-performance claim",
        "binary": str(binary),
        "binary_sha256": sha256_file(binary),
        "fixture_sha256": {
            "route": sha256_file(ROUTE),
            "stock": sha256_file(STOCK),
            "source_artifact": sha256_file(SOURCE),
        },
        "route_id": route_id,
        "source_artifact_sha256": source_sha256,
        "route_status": reaudited["routes"][0]["status"],
        "pmi_status": receipt["process_mass_intensity"]["status"],
        "e_factor_status": receipt["e_factor"]["status"],
        "missing_mass_categories": receipt["coverage"]["missing_categories"],
        "generated_files": ["initial-audit.json", "interchange-v1.json", "metrics-sidecar.json", "reaudit.json"],
    }
    (output_dir / "manifest.json").write_text(
        json.dumps(manifest, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(manifest, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
