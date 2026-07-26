#!/usr/bin/env python3
"""Deterministic, offline ORD (Open Reaction Database) -> RENKIN evidence sidecar audit/converter.

Reads *locally supplied* ORD Dataset files (.pb.gz / .pbtxt / .pb) -- this script
never downloads, clones, or makes any network request. Every accepted record is
matched against RENKIN's stable template_id via the compiled `renkin` binary's
`evidence match` subcommand (the same exact-canonical-SMILES-set matcher used at
route-display time -- see src/evidence_match.rs), so a record only ever lands in
the sidecar if RENKIN itself agrees, unambiguously, which template it belongs to.

Licensing: this script (like the rest of RENKIN) is MIT-licensed. ORD reaction
data is CC-BY-SA-4.0 (https://github.com/open-reaction-database/ord-data). The
generated sidecar/report are derivatives of ORD data (they carry ORD-sourced
SMILES, yields, conditions, and reference identifiers) and are NOT MIT -- treat
them as CC-BY-SA-4.0 and preserve attribution. See docs/guides/reaction-evidence.md.

Non-goals (see project README / CHANGELOG for the full list): no automatic
dataset download, no literature search, no DOI/patent completion or repair, no
yield/condition prediction, no fuzzy or similarity-based template matching, no
automatic ReactionWarning generation from undesired products.
"""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

try:
    from google.protobuf import text_format
    from ord_schema.proto import dataset_pb2

    HAVE_ORD_SCHEMA = True
except ImportError:  # pragma: no cover -- exercised by scripts/tests without the dep installed
    HAVE_ORD_SCHEMA = False

# Bump whenever acceptance criteria or field-mapping rules below change --
# recorded in the manifest so a regenerated sidecar can be told apart from one
# produced by an earlier version of this script's rules.
SELECTION_ALGORITHM_VERSION = "ord-evidence-audit/1"
IMPORTER_SCHEMA_VERSION = "ord-evidence-audit/1"
SIDECAR_SCHEMA_VERSION = 2

SOURCE_DATA_LICENSE = "CC-BY-SA-4.0"
IMPORTER_CODE_LICENSE = "MIT"

# Priority rules: candidates for real sidecar export once reviewed (Phase 3A scope).
PRIORITY_TEMPLATE_IDS = frozenset(
    {"rule:ester_cleavage", "rule:amide_cleavage", "rule:reductive_amination_retro"}
)
# Generic rules: audited (statistics collected) but never written to the sidecar --
# a single template_id here can cover chemically distinct reaction contexts, so a
# unique exact-precursor match alone isn't (yet) trusted as sufficient evidence.
# See docs/guides/reaction-evidence.md "Importing from ORD" for the audited rate.
AUDIT_ONLY_TEMPLATE_IDS = frozenset(
    {"rule:cn_aliphatic_cleavage", "rule:michael_retro", "rule:co_aliphatic_cleavage"}
)

# Fixed, documented precision for every numeric value written to the sidecar.
# Percentage/Temperature/Time proto fields are 32-bit floats; Python widens them
# to double on read (e.g. 24.100000381469727) -- round once, uniformly, so the
# sidecar is both byte-reproducible and readable. Not a measurement correction.
NUMERIC_ROUND_NDIGITS = 4


def round_value(value: float) -> float:
    return round(float(value), NUMERIC_ROUND_NDIGITS)


# Deterministic display-name choice for a compound: prefer a human name over an
# identifier code, but never invent one. Preserves whatever ORD already recorded.
NAME_IDENTIFIER_PRIORITY = ["NAME", "IUPAC_NAME", "SMILES", "CAS_NUMBER"]

SMILES_IDENTIFIER_TYPES = ("SMILES",)

ATMOSPHERE_NAMES = {
    "AIR": "air",
    "NITROGEN": "nitrogen",
    "ARGON": "argon",
    "OXYGEN": "oxygen",
    "HYDROGEN": "hydrogen",
    "CARBON_MONOXIDE": "carbon monoxide",
    "CARBON_DIOXIDE": "carbon dioxide",
    "METHANE": "methane",
    "AMMONIA": "ammonia",
    "OZONE": "ozone",
    "ETHYLENE": "ethylene",
    "ACETYLENE": "acetylene",
}

TEMPERATURE_TO_CELSIUS = {
    "CELSIUS": lambda v: v,
    "FAHRENHEIT": lambda v: (v - 32.0) * 5.0 / 9.0,
    "KELVIN": lambda v: v - 273.15,
}
# Precision is a delta, not an absolute reading -- only the FAHRENHEIT scale
# factor applies; Celsius and Kelvin share a scale, so their deltas are equal.
TEMPERATURE_PRECISION_TO_CELSIUS = {
    "CELSIUS": lambda v: v,
    "FAHRENHEIT": lambda v: v * 5.0 / 9.0,
    "KELVIN": lambda v: v,
}

TIME_TO_HOURS = {
    "SECOND": lambda v: v / 3600.0,
    "MINUTE": lambda v: v / 60.0,
    "HOUR": lambda v: v,
    "DAY": lambda v: v * 24.0,
}


class RejectionReason:
    MISSING_DATASET_ID = "missing_dataset_id"
    MISSING_REACTION_ID = "missing_reaction_id"
    DUPLICATE_SOURCE_RECORD = "duplicate_source_record"
    AMBIGUOUS_DESIRED_PRODUCT = "ambiguous_desired_product"
    MISSING_PRODUCT_SMILES = "missing_product_smiles"
    NO_PRECURSORS = "no_precursors"
    MISSING_PRECURSOR_SMILES = "missing_precursor_smiles"
    INVALID_SMILES = "invalid_smiles"
    NO_TEMPLATE_MATCH = "no_template_match"
    AMBIGUOUS_TEMPLATE_MATCH = "ambiguous_template_match"
    AUDIT_ONLY_TEMPLATE = "audit_only_template_excluded_from_sidecar"
    NO_YIELD_OR_CONDITION = "no_yield_or_condition"
    AMBIGUOUS_YIELD = "ambiguous_yield"


# ── ORD dataset discovery & parsing ──────────────────────────────────────────


def discover_dataset_files(ord_data_dir: Path) -> list[Path]:
    files = [
        p
        for p in ord_data_dir.rglob("*")
        if p.is_file() and p.suffix in (".gz", ".pbtxt", ".pb")
    ]
    return sorted(files, key=lambda p: str(p))


def load_dataset(path: Path) -> "dataset_pb2.Dataset":
    dataset = dataset_pb2.Dataset()
    if path.name.endswith(".pb.gz") or path.suffix == ".gz":
        with gzip.open(path, "rb") as f:
            dataset.ParseFromString(f.read())
    elif path.suffix == ".pbtxt":
        text_format.Parse(path.read_text(encoding="utf-8"), dataset)
    else:
        dataset.ParseFromString(path.read_bytes())
    return dataset


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


# ── Small, honest field extractors (no inference, no fabrication) ───────────


def first_identifier_value(identifiers, type_names) -> str | None:
    for ident in identifiers:
        if ident.type is not None and _enum_name(ident, "type") in type_names and ident.value:
            return ident.value
    return None


def _enum_name(message, field_name: str) -> str:
    return message.DESCRIPTOR.fields_by_name[field_name].enum_type.values_by_number[
        getattr(message, field_name)
    ].name


def pick_display_name(identifiers) -> str | None:
    for type_name in NAME_IDENTIFIER_PRIORITY:
        value = first_identifier_value(identifiers, (type_name,))
        if value:
            return value
    return None


def normalize_doi(raw: str) -> str | None:
    """Strips a leading `https://doi.org/` or `doi:` prefix and lowercases --
    nothing else. Returns None (never a guess/repair) if what remains doesn't
    look like a DOI (must start with "10.")."""
    value = raw.strip()
    value = re.sub(r"^(https?://(dx\.)?doi\.org/|doi:)", "", value, flags=re.IGNORECASE)
    value = value.strip().lower()
    return value if value.startswith("10.") else None


def normalize_patent(raw: str) -> str | None:
    value = raw.strip()
    return value if value else None


def normalize_url(raw: str) -> str | None:
    value = raw.strip()
    return value if "://" in value else None


# ── Per-reaction extraction ──────────────────────────────────────────────────


class ExtractionFailure(Exception):
    def __init__(self, reason: str):
        super().__init__(reason)
        self.reason = reason


def extract_desired_product_smiles(reaction) -> str:
    if len(reaction.outcomes) != 1:
        raise ExtractionFailure(RejectionReason.AMBIGUOUS_DESIRED_PRODUCT)
    outcome = reaction.outcomes[0]
    desired = [p for p in outcome.products if p.is_desired_product]
    if len(desired) != 1:
        raise ExtractionFailure(RejectionReason.AMBIGUOUS_DESIRED_PRODUCT)
    smiles = first_identifier_value(desired[0].identifiers, SMILES_IDENTIFIER_TYPES)
    if not smiles:
        raise ExtractionFailure(RejectionReason.MISSING_PRODUCT_SMILES)
    return smiles


def extract_precursor_smiles(reaction) -> list[str]:
    reactant_compounds = []
    for key in sorted(reaction.inputs.keys()):
        for component in reaction.inputs[key].components:
            if _enum_name(component, "reaction_role") == "REACTANT":
                reactant_compounds.append(component)
    if not reactant_compounds:
        raise ExtractionFailure(RejectionReason.NO_PRECURSORS)
    precursors = []
    for compound in reactant_compounds:
        smiles = first_identifier_value(compound.identifiers, SMILES_IDENTIFIER_TYPES)
        if not smiles:
            raise ExtractionFailure(RejectionReason.MISSING_PRECURSOR_SMILES)
        precursors.append(smiles)
    return precursors


def extract_conditions(reaction) -> dict | None:
    catalysts, reagents, solvents = set(), set(), set()
    for key in sorted(reaction.inputs.keys()):
        for component in reaction.inputs[key].components:
            role = _enum_name(component, "reaction_role")
            name = pick_display_name(component.identifiers)
            if not name:
                continue
            if role == "CATALYST":
                catalysts.add(name)
            elif role == "REAGENT":
                # ORD has no distinct BASE role -- REAGENT is the closest fit.
                # `bases` is intentionally always left empty; see
                # docs/guides/reaction-evidence.md "Importing from ORD".
                reagents.add(name)
            elif role == "SOLVENT":
                solvents.add(name)

    temperature_c = None
    temp = reaction.conditions.temperature
    if temp.HasField("setpoint") and temp.setpoint.HasField("value"):
        unit = _enum_name(temp.setpoint, "units")
        convert = TEMPERATURE_TO_CELSIUS.get(unit)
        if convert:
            value_c = convert(temp.setpoint.value)
            precision_c = 0.0
            if temp.setpoint.HasField("precision"):
                precision_c = TEMPERATURE_PRECISION_TO_CELSIUS[unit](temp.setpoint.precision)
            lo, hi = value_c - precision_c, value_c + precision_c
            if 0.0 <= precision_c and lo <= hi:
                temperature_c = {"min": round_value(lo), "max": round_value(hi)}

    time_hours = None
    if len(reaction.outcomes) == 1:
        rt = reaction.outcomes[0].reaction_time
        if rt.HasField("value"):
            unit = _enum_name(rt, "units")
            convert = TIME_TO_HOURS.get(unit)
            if convert:
                value_h = convert(rt.value)
                precision_h = convert(rt.precision) if rt.HasField("precision") else 0.0
                lo, hi = value_h - precision_h, value_h + precision_h
                if 0.0 <= precision_h and lo <= hi:
                    time_hours = {"min": round_value(lo), "max": round_value(hi)}

    atmosphere = None
    atm = reaction.conditions.pressure.atmosphere
    atm_type = _enum_name(atm, "type")
    if atm_type == "CUSTOM" and atm.details:
        atmosphere = atm.details
    elif atm_type in ATMOSPHERE_NAMES:
        atmosphere = ATMOSPHERE_NAMES[atm_type]

    if not (catalysts or reagents or solvents or temperature_c or time_hours or atmosphere):
        return None
    return {
        "catalysts": sorted(catalysts),
        "reagents": sorted(reagents),
        "bases": [],
        "solvents": sorted(solvents),
        "temperature_c": temperature_c,
        "time_hours": time_hours,
        "atmosphere": atmosphere,
    }


def measurement_provenance_note(measurement) -> str:
    """A deterministic, machine-readable record of *how* a YIELD measurement
    was made, for `ReactionExample.notes` -- NOT used to infer yield basis.

    `uses_internal_standard`/`uses_authentic_standard` describe quantification
    method, not whether the number is an isolated weight or a calibrated
    assay value: a false/unset flag is not evidence of "isolated" (unstandardized
    NMR/LC yields exist too), and a true flag is not evidence of "assay" either.
    ORD's YIELD measurement type simply doesn't carry that distinction, so
    RENKIN maps it to `basis: "unknown"` unconditionally and keeps this
    provenance only as a note for a human (or a future, more specific rule) to
    read -- never as an input to the basis decision itself.
    """
    has = lambda name: measurement.HasField(name)  # noqa: E731
    fields = {
        "ord_measurement_type": _enum_name(measurement, "type"),
        "analysis_key": measurement.analysis_key or None,
        "uses_internal_standard": measurement.uses_internal_standard if has("uses_internal_standard") else None,
        "uses_authentic_standard": measurement.uses_authentic_standard if has("uses_authentic_standard") else None,
        "details": measurement.details or None,
    }
    return json.dumps(fields, sort_keys=True)


def extract_yield(reaction) -> tuple[dict | None, str | None]:
    """Returns (reported_yield_dict_or_None, rejection_reason_or_None).

    Only two bases are ever produced: `conversion` (ORD's own outcome.conversion
    field) and `unknown` (ORD's ProductMeasurement YIELD type does not itself
    distinguish isolated-weight from calibrated-assay measurements -- see
    measurement_provenance_note and docs/guides/reaction-evidence.md -- so
    `isolated`/`assay` are never guessed from uses_internal_standard/
    uses_authentic_standard or anything else).
    Multiple non-duplicate candidates reject the record rather than picking one.
    """
    if len(reaction.outcomes) != 1:
        return None, None
    outcome = reaction.outcomes[0]
    # value/basis is the dedup key; notes travels along keyed by the same pair
    # (first-seen wins if two measurements coincidentally share value+basis).
    candidates: dict[tuple[float, str], str | None] = {}

    if outcome.conversion.HasField("value"):
        value = outcome.conversion.value
        if 0.0 <= value <= 100.0:
            candidates.setdefault((round_value(value), "conversion"), None)

    desired = [p for p in outcome.products if p.is_desired_product]
    if len(desired) == 1:
        for measurement in desired[0].measurements:
            if (
                _enum_name(measurement, "type") == "YIELD"
                and measurement.HasField("percentage")
                and measurement.percentage.HasField("value")
            ):
                value = measurement.percentage.value
                if 0.0 <= value <= 100.0:
                    key = (round_value(value), "unknown")
                    candidates.setdefault(key, measurement_provenance_note(measurement))

    if not candidates:
        return None, None
    if len(candidates) > 1:
        return None, RejectionReason.AMBIGUOUS_YIELD
    (value, basis), note = next(iter(candidates.items()))
    result = {"percentage": value, "basis": basis}
    if note is not None:
        result["notes"] = note
    return result, None


def extract_references(dataset_id: str, reaction_id: str, reaction) -> list[dict]:
    references = [
        {
            "id": f"ord:{dataset_id}:{reaction_id}",
            "kind": "dataset_record",
            "identifier": f"ord:{dataset_id}:{reaction_id}",
        }
    ]
    prov = reaction.provenance
    if prov.doi:
        doi = normalize_doi(prov.doi)
        if doi:
            references.append({"id": f"doi:{doi}", "kind": "doi", "identifier": doi})
    if prov.patent:
        patent = normalize_patent(prov.patent)
        if patent:
            references.append({"id": f"patent:{patent}", "kind": "patent", "identifier": patent})
    if prov.publication_url:
        url = normalize_url(prov.publication_url)
        if url:
            references.append({"id": f"url:{url}", "kind": "url", "identifier": url})
    return references


def count_non_desired_products(reaction) -> int:
    if len(reaction.outcomes) != 1:
        return 0
    return sum(1 for p in reaction.outcomes[0].products if not p.is_desired_product)


# ── Main extraction pass: ORD -> normalized candidate (pre template-match) ──


class Candidate:
    def __init__(self, dataset_id, reaction_id, target_smiles, precursor_smiles,
                 conditions, reported_yield, references, non_desired_product_count):
        self.dataset_id = dataset_id
        self.reaction_id = reaction_id
        self.record_id = f"{dataset_id}:{reaction_id}"
        self.target_smiles = target_smiles
        self.precursor_smiles = precursor_smiles
        self.conditions = conditions
        self.reported_yield = reported_yield
        self.references = references
        self.non_desired_product_count = non_desired_product_count


def extract_candidates(dataset_files: list[Path], report: "AuditReport") -> list[Candidate]:
    seen_records: set[str] = set()
    candidates: list[Candidate] = []

    for path in dataset_files:
        dataset = load_dataset(path)
        dataset_id = dataset.dataset_id
        if not dataset_id:
            report.record_dataset_rejection(str(path), RejectionReason.MISSING_DATASET_ID)
            continue
        report.note_dataset_id(dataset_id)

        for reaction in dataset.reactions:
            report.records_seen += 1
            reaction_id = reaction.reaction_id
            if not reaction_id:
                report.reject(dataset_id, RejectionReason.MISSING_REACTION_ID)
                continue

            record_key = f"{dataset_id}:{reaction_id}"
            if record_key in seen_records:
                report.reject(dataset_id, RejectionReason.DUPLICATE_SOURCE_RECORD)
                continue
            seen_records.add(record_key)

            try:
                target_smiles = extract_desired_product_smiles(reaction)
                precursor_smiles = extract_precursor_smiles(reaction)
            except ExtractionFailure as failure:
                report.reject(dataset_id, failure.reason)
                continue

            conditions = extract_conditions(reaction)
            reported_yield, yield_reject_reason = extract_yield(reaction)
            if yield_reject_reason:
                report.reject(dataset_id, yield_reject_reason)
                continue
            if conditions is None and reported_yield is None:
                report.reject(dataset_id, RejectionReason.NO_YIELD_OR_CONDITION)
                continue

            non_desired = count_non_desired_products(reaction)
            if non_desired:
                report.with_non_desired_products += 1

            references = extract_references(dataset_id, reaction_id, reaction)
            candidates.append(
                Candidate(
                    dataset_id, reaction_id, target_smiles, precursor_smiles,
                    conditions, reported_yield, references, non_desired,
                )
            )
            if conditions is not None:
                report.with_conditions += 1
            if reported_yield is not None:
                report.with_reported_yield += 1
            if any(r["kind"] == "doi" for r in references):
                report.with_doi += 1
            if any(r["kind"] == "patent" for r in references):
                report.with_patent += 1

    return candidates


# ── Batch template matching via the compiled renkin binary ─────────────────


def batch_match(renkin_bin: str, templates_path: str, candidates: list[Candidate]) -> dict[str, dict]:
    """Runs every candidate through `renkin evidence match` in one subprocess
    call and returns {record_id: match_result_row}."""
    if not candidates:
        return {}
    with tempfile.TemporaryDirectory() as tmp:
        input_path = Path(tmp) / "reactions.jsonl"
        output_path = Path(tmp) / "matches.jsonl"
        with open(input_path, "w", encoding="utf-8") as f:
            for c in candidates:
                f.write(
                    json.dumps(
                        {
                            "record_id": c.record_id,
                            "target_smiles": c.target_smiles,
                            "precursor_smiles": c.precursor_smiles,
                        }
                    )
                    + "\n"
                )
        subprocess.run(
            [
                renkin_bin, "evidence", "match",
                "--input", str(input_path),
                "--templates", templates_path,
                "--output", str(output_path),
            ],
            check=True,
        )
        results = {}
        with open(output_path, "r", encoding="utf-8") as f:
            for line in f:
                row = json.loads(line)
                results[row["record_id"]] = row
        return results


# ── Audit report accumulator ────────────────────────────────────────────────


class AuditReport:
    def __init__(self):
        self.records_seen = 0
        self.records_accepted = 0
        self.records_rejected = 0
        self.records_audit_only_excluded = 0
        self.unique_template_matches = 0
        self.ambiguous_template_matches = 0
        self.no_template_matches = 0
        self.with_conditions = 0
        self.with_reported_yield = 0
        self.with_doi = 0
        self.with_patent = 0
        self.with_non_desired_products = 0
        self.by_template_id: dict[str, int] = {}
        self.by_rejection_reason: dict[str, int] = {}
        self.by_dataset_id: dict[str, dict[str, int]] = {}
        self.known_limitations = [
            "ORD has no BASE reaction role distinct from REAGENT; `bases` is "
            "always empty and REAGENT-role components are surfaced under "
            "`reagents` instead. See docs/guides/reaction-evidence.md.",
            "Yield basis is only ever 'conversion' (ORD's own outcome.conversion "
            "field) or 'unknown' (ORD's YIELD measurement type does not itself "
            "distinguish isolated-weight from calibrated-assay measurements) -- "
            "'isolated'/'assay' are never inferred.",
            "no_template_match may conflate a genuinely different reaction with "
            "a canonicalization-tie-breaking artifact of the underlying SMILES "
            "engine (see AGENTS.md); it is not proof the reaction type is absent "
            "from the loaded template set.",
        ]

    def note_dataset_id(self, dataset_id: str) -> None:
        self.by_dataset_id.setdefault(dataset_id, {"accepted": 0, "rejected": 0})

    def record_dataset_rejection(self, path: str, reason: str) -> None:
        self.records_rejected += 0  # no per-reaction record exists yet; file-level only
        self.by_rejection_reason[reason] = self.by_rejection_reason.get(reason, 0) + 1

    def reject(self, dataset_id: str, reason: str) -> None:
        self.records_rejected += 1
        self.by_rejection_reason[reason] = self.by_rejection_reason.get(reason, 0) + 1
        self.by_dataset_id.setdefault(dataset_id, {"accepted": 0, "rejected": 0})
        self.by_dataset_id[dataset_id]["rejected"] += 1

    def accept(self, dataset_id: str, template_id: str) -> None:
        self.records_accepted += 1
        self.by_template_id[template_id] = self.by_template_id.get(template_id, 0) + 1
        self.by_dataset_id.setdefault(dataset_id, {"accepted": 0, "rejected": 0})
        self.by_dataset_id[dataset_id]["accepted"] += 1

    def to_dict(self) -> dict:
        return {
            "records_seen": self.records_seen,
            "records_accepted": self.records_accepted,
            "records_rejected": self.records_rejected,
            "records_audit_only_excluded": self.records_audit_only_excluded,
            "unique_template_matches": self.unique_template_matches,
            "ambiguous_template_matches": self.ambiguous_template_matches,
            "no_template_matches": self.no_template_matches,
            "with_conditions": self.with_conditions,
            "with_reported_yield": self.with_reported_yield,
            "with_doi": self.with_doi,
            "with_patent": self.with_patent,
            "with_non_desired_products": self.with_non_desired_products,
            "by_template_id": self.by_template_id,
            "by_rejection_reason": self.by_rejection_reason,
            "by_dataset_id": self.by_dataset_id,
            "known_limitations": self.known_limitations,
        }


# ── Sidecar assembly ─────────────────────────────────────────────────────────


def build_sidecar(candidates: list[Candidate], match_results: dict[str, dict], report: AuditReport) -> dict:
    templates: dict[str, dict] = {}

    for c in candidates:
        row = match_results[c.record_id]
        status = row["status"]
        if status == "invalid_input":
            report.reject(c.dataset_id, RejectionReason.INVALID_SMILES)
            continue
        if status == "no_match":
            report.no_template_matches += 1
            report.reject(c.dataset_id, RejectionReason.NO_TEMPLATE_MATCH)
            continue
        if status == "ambiguous":
            report.ambiguous_template_matches += 1
            report.reject(c.dataset_id, RejectionReason.AMBIGUOUS_TEMPLATE_MATCH)
            continue

        report.unique_template_matches += 1
        template_id = row["matching_template_ids"][0]

        if template_id in AUDIT_ONLY_TEMPLATE_IDS:
            report.records_audit_only_excluded += 1
            report.by_template_id[template_id] = report.by_template_id.get(template_id, 0) + 1
            continue

        report.accept(c.dataset_id, template_id)

        example = {
            "id": f"ord:{c.dataset_id}:{c.reaction_id}:0",
            "target_smiles": row["canonical_target"],
            "precursor_smiles": row["canonical_precursors"],
            "dataset_record_id": c.record_id,
        }
        reference_ids = sorted({r["id"] for r in c.references})
        if c.conditions is not None:
            example["conditions"] = {
                **c.conditions,
                "source": "dataset_record",
                "scope": "substrate_specific",
                "reference_ids": reference_ids,
            }
        if c.reported_yield is not None:
            example["reported_yield"] = {
                "percentage": c.reported_yield["percentage"],
                "basis": c.reported_yield["basis"],
                "source": "dataset_record",
                "scope": "substrate_specific",
                "reference_ids": reference_ids,
            }
            # Deterministic, machine-readable record of *how* the yield was
            # measured (never used to infer basis -- see
            # measurement_provenance_note). ReactionExample.notes is the only
            # place this fits; ReportedYield itself has no notes field.
            if c.reported_yield.get("notes") is not None:
                example["notes"] = c.reported_yield["notes"]
        example["reference_ids"] = reference_ids

        entry = templates.setdefault(
            template_id, {"references": {}, "condition_candidates": [], "reported_yields": [], "warnings": [], "examples": []}
        )
        entry["examples"].append(example)
        for ref in c.references:
            entry["references"].setdefault(ref["id"], ref)

    output_templates = {}
    for template_id, entry in templates.items():
        output_templates[template_id] = {
            "references": [entry["references"][rid] for rid in sorted(entry["references"])],
            "condition_candidates": [],
            "reported_yields": [],
            "warnings": [],
            "examples": sorted(entry["examples"], key=lambda e: e["id"]),
        }

    return {"schema_version": SIDECAR_SCHEMA_VERSION, "templates": output_templates}


def write_json(path: Path, data: dict) -> None:
    text = json.dumps(data, indent=2, sort_keys=True, separators=(",", ": "), ensure_ascii=False)
    path.write_text(text + "\n", encoding="utf-8")


def read_renkin_version(script_path: Path) -> str:
    cargo_toml = script_path.resolve().parent.parent / "Cargo.toml"
    match = re.search(r'(?m)^version\s*=\s*"([^"]+)"', cargo_toml.read_text(encoding="utf-8"))
    return match.group(1) if match else "unknown"


def read_git_commit(script_path: Path) -> str:
    try:
        result = subprocess.run(
            ["git", "rev-parse", "HEAD"],
            cwd=script_path.resolve().parent.parent,
            check=True,
            capture_output=True,
            text=True,
        )
        return result.stdout.strip()
    except (subprocess.CalledProcessError, FileNotFoundError):
        return "unknown"


def dependency_versions() -> dict:
    from importlib import metadata

    versions = {}
    for package in ("ord-schema", "protobuf", "rdkit"):
        try:
            versions[package] = metadata.version(package)
        except metadata.PackageNotFoundError:
            versions[package] = "not installed"
    return versions


def main(argv=None) -> int:
    if not HAVE_ORD_SCHEMA:
        print(
            "error: ord_schema is not installed. Install it via "
            "scripts/requirements-ord-evidence.txt before running this script.",
            file=sys.stderr,
        )
        return 1

    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ord-data", required=True, type=Path)
    parser.add_argument("--renkin-bin", required=True)
    parser.add_argument("--templates", required=True)
    parser.add_argument("--output-sidecar", required=True, type=Path)
    parser.add_argument("--output-report", required=True, type=Path)
    parser.add_argument("--output-manifest", required=True, type=Path)
    args = parser.parse_args(argv)

    if not args.ord_data.is_dir():
        print(f"error: --ord-data {args.ord_data} is not a directory", file=sys.stderr)
        return 1

    dataset_files = discover_dataset_files(args.ord_data)
    report = AuditReport()
    candidates = extract_candidates(dataset_files, report)
    match_results = batch_match(args.renkin_bin, args.templates, candidates)
    sidecar = build_sidecar(candidates, match_results, report)

    args.output_sidecar.parent.mkdir(parents=True, exist_ok=True)
    args.output_report.parent.mkdir(parents=True, exist_ok=True)
    args.output_manifest.parent.mkdir(parents=True, exist_ok=True)

    write_json(args.output_sidecar, sidecar)

    validation = subprocess.run(
        [args.renkin_bin, "evidence", "validate-sidecar", "--metadata", str(args.output_sidecar)],
        capture_output=True,
        text=True,
    )
    if validation.returncode != 0:
        print("error: generated sidecar failed RENKIN's own validation:", file=sys.stderr)
        print(validation.stdout, file=sys.stderr)
        print(validation.stderr, file=sys.stderr)
        return 1

    write_json(args.output_report, report.to_dict())

    input_sha256 = {str(p): sha256_file(p) for p in dataset_files}
    manifest = {
        "importer_schema_version": IMPORTER_SCHEMA_VERSION,
        "renkin_version": read_renkin_version(Path(__file__)),
        "renkin_git_commit": read_git_commit(Path(__file__)),
        "input_files": [str(p) for p in dataset_files],
        "input_sha256": input_sha256,
        "source_dataset_ids": sorted(report.by_dataset_id.keys()),
        "source_license": SOURCE_DATA_LICENSE,
        "importer_code_license": IMPORTER_CODE_LICENSE,
        "dependency_versions": dependency_versions(),
        "selection_algorithm_version": SELECTION_ALGORITHM_VERSION,
        "records_accepted": report.records_accepted,
        "records_rejected": report.records_rejected,
        "records_audit_only_excluded": report.records_audit_only_excluded,
        "output_sidecar_sha256": sha256_file(args.output_sidecar),
        "output_report_sha256": sha256_file(args.output_report),
        "deterministic_sort_rules": [
            "templates sorted by template_id",
            "examples sorted by example id",
            "references sorted by reference id",
            "reference_ids deduped and sorted",
            "catalysts/reagents/solvents deduped and sorted",
        ],
        "cli_invocation": sys.argv,
        "generated_at": datetime.now(timezone.utc).isoformat(),
        "generated_at_note": "informational only; excluded from reproducibility comparison",
    }
    write_json(args.output_manifest, manifest)

    print(
        f"accepted={report.records_accepted} rejected={report.records_rejected} "
        f"audit_only_excluded={report.records_audit_only_excluded} "
        f"seen={report.records_seen}",
        file=sys.stderr,
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
