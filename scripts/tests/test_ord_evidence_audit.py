"""Tests for scripts/ord_evidence_audit.py.

Run without the ord-schema dependency (`python -m unittest discover
scripts/tests`): only the pure-Python helpers (DOI/patent/URL normalization,
JSON writer determinism) are exercised. Tests that parse ORD protobuf
messages are skipped unless `ord_schema` is installed -- run those via a venv
built from requirements-ord-evidence.txt (see that file's header).
"""

import json
import subprocess
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))

import ord_evidence_audit as m  # noqa: E402

FIXTURE = Path(__file__).resolve().parent / "fixtures" / "ord_evidence_fixture.pbtxt"
RENKIN_BIN = Path(__file__).resolve().parent.parent.parent / "target" / "debug" / "renkin"
TEMPLATES = Path(__file__).resolve().parent.parent.parent / "data" / "templates_extracted_5000.smi"

requires_ord_schema = unittest.skipUnless(
    m.HAVE_ORD_SCHEMA, "requires ord-schema (see scripts/requirements-ord-evidence.txt)"
)
requires_renkin_bin = unittest.skipUnless(
    RENKIN_BIN.exists(), f"requires a built renkin binary at {RENKIN_BIN} (cargo build)"
)


class NormalizationTests(unittest.TestCase):
    """Pure-Python helpers -- no ord-schema needed, always run."""

    def test_normalize_doi_strips_url_prefix_and_lowercases(self):
        self.assertEqual(m.normalize_doi("https://doi.org/10.1000/XYZ"), "10.1000/xyz")
        self.assertEqual(m.normalize_doi("doi:10.1000/XYZ"), "10.1000/xyz")
        self.assertEqual(m.normalize_doi("10.1000/xyz"), "10.1000/xyz")

    def test_normalize_doi_rejects_non_doi(self):
        self.assertIsNone(m.normalize_doi("not a doi"))
        self.assertIsNone(m.normalize_doi(""))
        self.assertIsNone(m.normalize_doi("   "))

    def test_normalize_patent_is_verbatim(self):
        self.assertEqual(m.normalize_patent(" US1234567B2 "), "US1234567B2")
        self.assertIsNone(m.normalize_patent(""))
        self.assertIsNone(m.normalize_patent("   "))

    def test_normalize_url_requires_scheme(self):
        self.assertEqual(m.normalize_url(" https://example.com/paper "), "https://example.com/paper")
        self.assertIsNone(m.normalize_url("not a url"))

    def test_round_value_is_fixed_precision(self):
        self.assertEqual(m.round_value(24.100000381469727), round(24.100000381469727, 4))


class JsonWriterDeterminismTests(unittest.TestCase):
    def test_write_json_sorts_keys_and_has_trailing_newline(self):
        import tempfile

        data = {"b": 1, "a": [3, 2, 1], "c": {"z": 1, "y": 2}}
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "out.json"
            m.write_json(path, data)
            text = path.read_text(encoding="utf-8")
            self.assertTrue(text.endswith("\n"))
            self.assertTrue(text.startswith('{\n  "a"'))  # sorted: a before b before c

    def test_write_json_is_byte_identical_across_calls(self):
        import tempfile

        data = {"templates": {"b": {"x": 1}, "a": {"y": 2}}}
        with tempfile.TemporaryDirectory() as tmp:
            p1, p2 = Path(tmp) / "one.json", Path(tmp) / "two.json"
            m.write_json(p1, data)
            m.write_json(p2, data)
            self.assertEqual(p1.read_bytes(), p2.read_bytes())


class BuildSidecarTests(unittest.TestCase):
    """build_sidecar takes plain Candidate objects + a match_results dict --
    no ord-schema/protobuf needed to exercise its rejection-path handling."""

    def _candidate(self, record_id="ds:rxn-1", ref_id="doi:10.1000/shared"):
        dataset_id, reaction_id = record_id.split(":", 1)
        return m.Candidate(
            dataset_id, reaction_id, "CC(=O)OCC", ["CC(=O)O", "CCO"],
            None, {"percentage": 90.0, "basis": "unknown"},
            [{"id": ref_id, "kind": "doi", "identifier": ref_id.split(":", 1)[1]}],
            0,
        )

    def test_invalid_input_status_is_rejected(self):
        c = self._candidate()
        match_results = {c.record_id: {"status": "invalid_input", "matching_template_ids": []}}
        report = m.AuditReport()
        sidecar = m.build_sidecar([c], match_results, report)
        self.assertEqual(sidecar["templates"], {})
        self.assertEqual(report.by_rejection_reason.get(m.RejectionReason.INVALID_SMILES), 1)
        self.assertEqual(report.records_rejected, 1)

    def test_no_match_status_is_rejected(self):
        c = self._candidate()
        match_results = {c.record_id: {"status": "no_match", "matching_template_ids": []}}
        report = m.AuditReport()
        sidecar = m.build_sidecar([c], match_results, report)
        self.assertEqual(sidecar["templates"], {})
        self.assertEqual(report.no_template_matches, 1)
        self.assertEqual(report.by_rejection_reason.get(m.RejectionReason.NO_TEMPLATE_MATCH), 1)

    def test_ambiguous_status_is_rejected(self):
        c = self._candidate()
        match_results = {
            c.record_id: {
                "status": "ambiguous",
                "matching_template_ids": ["rule:ester_cleavage", "rule:amide_cleavage"],
            }
        }
        report = m.AuditReport()
        sidecar = m.build_sidecar([c], match_results, report)
        self.assertEqual(sidecar["templates"], {})
        self.assertEqual(report.ambiguous_template_matches, 1)
        self.assertEqual(report.by_rejection_reason.get(m.RejectionReason.AMBIGUOUS_TEMPLATE_MATCH), 1)

    def test_audit_only_template_is_excluded_from_sidecar_but_counted(self):
        c = self._candidate()
        match_results = {c.record_id: {"status": "unique", "matching_template_ids": ["rule:michael_retro"]}}
        report = m.AuditReport()
        sidecar = m.build_sidecar([c], match_results, report)
        self.assertEqual(sidecar["templates"], {})
        self.assertEqual(report.records_audit_only_excluded, 1)
        self.assertEqual(report.by_template_id.get("rule:michael_retro"), 1)
        self.assertEqual(report.records_accepted, 0)

    def _match(self, c, template_id):
        return {
            c.record_id: {
                "status": "unique",
                "matching_template_ids": [template_id],
                "canonical_target": "CC(=O)OCC",
                "canonical_precursors": ["CC(=O)O", "CCO"],
            }
        }

    def test_each_priority_template_is_accepted(self):
        for template_id in sorted(m.PRIORITY_TEMPLATE_IDS):
            with self.subTest(template_id):
                c = self._candidate()
                report = m.AuditReport()
                sidecar = m.build_sidecar([c], self._match(c, template_id), report)
                self.assertIn(template_id, sidecar["templates"])
                self.assertEqual(report.records_accepted, 1)
                self.assertEqual(report.records_rejected, 0)
                self.assertEqual(report.records_audit_only_excluded, 0)

    def test_each_audit_only_template_is_excluded_not_accepted(self):
        for template_id in sorted(m.AUDIT_ONLY_TEMPLATE_IDS):
            with self.subTest(template_id):
                c = self._candidate()
                report = m.AuditReport()
                sidecar = m.build_sidecar([c], self._match(c, template_id), report)
                self.assertEqual(sidecar["templates"], {})
                self.assertEqual(report.records_audit_only_excluded, 1)
                self.assertEqual(report.records_accepted, 0)
                self.assertEqual(report.records_rejected, 0)

    def test_non_priority_non_audit_only_handcrafted_rule_is_rejected(self):
        # A real hand-crafted rule (Suzuki) that's neither on the priority
        # allowlist nor the audit-only list -- out of scope for Phase 3A even
        # though the match itself was unique.
        c = self._candidate()
        report = m.AuditReport()
        sidecar = m.build_sidecar([c], self._match(c, "rule:suzuki_retro"), report)
        self.assertEqual(sidecar["templates"], {})
        self.assertEqual(report.by_rejection_reason.get(m.RejectionReason.OUT_OF_SCOPE_TEMPLATE), 1)
        self.assertEqual(report.records_accepted, 0)
        self.assertEqual(report.records_audit_only_excluded, 0)

    def test_unique_match_on_extracted_template_is_rejected(self):
        # smirks-sha256:* templates are never on the priority allowlist,
        # regardless of how confidently they matched.
        c = self._candidate()
        report = m.AuditReport()
        template_id = "smirks-sha256:" + "0" * 64
        sidecar = m.build_sidecar([c], self._match(c, template_id), report)
        self.assertEqual(sidecar["templates"], {})
        self.assertEqual(report.by_rejection_reason.get(m.RejectionReason.OUT_OF_SCOPE_TEMPLATE), 1)

    def test_out_of_scope_template_never_appears_in_sidecar(self):
        c1, c2 = self._candidate(record_id="ds:rxn-1"), self._candidate(record_id="ds:rxn-2")
        match_results = {**self._match(c1, "rule:suzuki_retro"), **self._match(c2, "rule:ester_cleavage")}
        report = m.AuditReport()
        sidecar = m.build_sidecar([c1, c2], match_results, report)
        self.assertNotIn("rule:suzuki_retro", sidecar["templates"])
        self.assertIn("rule:ester_cleavage", sidecar["templates"])

    def test_reference_ids_deduped_across_records_sharing_one_template(self):
        c1 = self._candidate(record_id="ds:rxn-1", ref_id="doi:10.1000/shared")
        c2 = self._candidate(record_id="ds:rxn-2", ref_id="doi:10.1000/shared")
        match_results = {
            c.record_id: {
                "status": "unique",
                "matching_template_ids": ["rule:ester_cleavage"],
                "canonical_target": "CC(=O)OCC",
                "canonical_precursors": ["CC(=O)O", "CCO"],
            }
            for c in (c1, c2)
        }
        report = m.AuditReport()
        sidecar = m.build_sidecar([c1, c2], match_results, report)
        refs = sidecar["templates"]["rule:ester_cleavage"]["references"]
        self.assertEqual([r["id"] for r in refs], ["doi:10.1000/shared"])  # one entry, not two
        self.assertEqual(len(sidecar["templates"]["rule:ester_cleavage"]["examples"]), 2)

    def test_accepted_rejected_audit_only_sum_to_seen_overall_and_per_dataset(self):
        c_accept = self._candidate(record_id="ds:rxn-accept")
        c_audit_only = self._candidate(record_id="ds:rxn-audit-only")
        c_reject = self._candidate(record_id="ds:rxn-reject")
        match_results = {
            **self._match(c_accept, "rule:ester_cleavage"),
            **self._match(c_audit_only, "rule:michael_retro"),
            **self._match(c_reject, "rule:suzuki_retro"),
        }
        report = m.AuditReport()
        report.records_seen = 3  # normally incremented during extract_candidates
        m.build_sidecar([c_accept, c_audit_only, c_reject], match_results, report)

        self.assertEqual(
            report.records_accepted + report.records_rejected + report.records_audit_only_excluded,
            report.records_seen,
        )
        bucket = report.by_dataset_id["ds"]
        self.assertEqual(
            bucket["accepted"] + bucket["rejected"] + bucket["audit_only_excluded"],
            report.records_seen,
        )
        self.assertEqual(bucket, {"accepted": 1, "rejected": 1, "audit_only_excluded": 1})


@requires_ord_schema
class ExtractionTests(unittest.TestCase):
    def setUp(self):
        self.report = m.AuditReport()

    def test_minimal_valid_fixture_extracts_expected_candidate(self):
        candidates = m.extract_candidates([FIXTURE], self.report)
        self.assertEqual(len(candidates), 1)
        c = candidates[0]
        self.assertEqual(c.target_smiles, "CC(=O)OCC")
        self.assertEqual(sorted(c.precursor_smiles), ["CC(=O)O", "CCO"])
        self.assertEqual(c.conditions["catalysts"], ["sulfuric acid"])
        self.assertEqual(c.conditions["solvents"], ["toluene"])
        self.assertEqual(c.conditions["bases"], [])
        self.assertEqual(c.conditions["atmosphere"], "nitrogen")
        self.assertEqual(c.conditions["temperature_c"], {"min": 78.0, "max": 78.0})
        self.assertEqual(c.conditions["time_hours"], {"min": 2.0, "max": 2.0})
        self.assertEqual(c.reported_yield["percentage"], 87.5)
        self.assertEqual(c.reported_yield["basis"], "unknown")
        self.assertIn("notes", c.reported_yield)  # measurement provenance, not used for basis
        ref_ids = {r["id"] for r in c.references}
        self.assertIn("doi:10.1000/renkin-fixture-example", ref_ids)

    def _dataset(self, dataset_id="ord_dataset-test"):
        from ord_schema.proto import dataset_pb2

        return dataset_pb2.Dataset(dataset_id=dataset_id)

    def _add_basic_reaction(self, dataset, reaction_id="rxn-1", with_yield=True):
        from ord_schema.proto import reaction_pb2

        r = dataset.reactions.add()
        r.reaction_id = reaction_id
        inp = r.inputs["reactants"]
        for smiles in ("CC(=O)O", "CCO"):
            comp = inp.components.add()
            comp.identifiers.add(type=reaction_pb2.CompoundIdentifier.SMILES, value=smiles)
            comp.reaction_role = reaction_pb2.ReactionRole.REACTANT
        outcome = r.outcomes.add()
        prod = outcome.products.add()
        prod.identifiers.add(type=reaction_pb2.CompoundIdentifier.SMILES, value="CC(=O)OCC")
        prod.is_desired_product = True
        if with_yield:
            meas = prod.measurements.add()
            meas.type = reaction_pb2.ProductMeasurement.YIELD
            meas.percentage.value = 90.0
        return r

    def _write_dataset(self, dataset, tmp_path):
        from google.protobuf import text_format

        path = tmp_path / "d.pbtxt"
        path.write_text(text_format.MessageToString(dataset), encoding="utf-8")
        return path

    def test_missing_desired_product_is_rejected(self):
        import tempfile

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.outcomes[0].products[0].is_desired_product = False
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(
                self.report.by_rejection_reason.get(m.RejectionReason.AMBIGUOUS_DESIRED_PRODUCT), 1
            )

    def test_multiple_desired_products_is_rejected(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        extra = r.outcomes[0].products.add()
        extra.identifiers.add(type=reaction_pb2.CompoundIdentifier.SMILES, value="CCO")
        extra.is_desired_product = True
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(
                self.report.by_rejection_reason.get(m.RejectionReason.AMBIGUOUS_DESIRED_PRODUCT), 1
            )

    def test_missing_precursors_is_rejected(self):
        import tempfile

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.ClearField("inputs")
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(self.report.by_rejection_reason.get(m.RejectionReason.NO_PRECURSORS), 1)

    def test_invalid_smiles_yields_no_candidate_at_extraction_but_flows_to_matcher(self):
        # extract_* only requires a SMILES string to be *present*, not valid --
        # validity is decided by RENKIN's own matcher (see build_sidecar test).
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.outcomes[0].products[0].identifiers[0].value = "not(a smiles"
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 1)
            self.assertEqual(candidates[0].target_smiles, "not(a smiles")

    def test_ambiguous_yield_is_rejected(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        # A second, differently-valued YIELD measurement on the same product --
        # can't be uniquely resolved, so the whole record is rejected.
        extra = r.outcomes[0].products[0].measurements.add()
        extra.type = reaction_pb2.ProductMeasurement.YIELD
        extra.percentage.value = 42.0
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(self.report.by_rejection_reason.get(m.RejectionReason.AMBIGUOUS_YIELD), 1)

    def test_duplicate_yield_measurements_are_deduped_not_rejected(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        # Identical (value, basis) duplicate -- collapses to one candidate.
        dup = r.outcomes[0].products[0].measurements.add()
        dup.type = reaction_pb2.ProductMeasurement.YIELD
        dup.percentage.value = 90.0
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 1)
            self.assertEqual(candidates[0].reported_yield["percentage"], 90.0)
            self.assertEqual(candidates[0].reported_yield["basis"], "unknown")

    def test_yield_basis_is_unknown_regardless_of_standard_flags(self):
        """uses_internal_standard/uses_authentic_standard describe *how* a
        yield was quantified, not whether it's isolated or assay -- neither
        flag, in either state, ever changes the resulting basis. See
        measurement_provenance_note's docstring for why."""
        from ord_schema.proto import reaction_pb2

        cases = [
            ("internal_standard_true", {"uses_internal_standard": True}),
            ("internal_standard_false", {"uses_internal_standard": False}),
            ("internal_standard_unset", {}),
            ("authentic_standard_true", {"uses_authentic_standard": True}),
        ]
        for name, flags in cases:
            with self.subTest(name):
                import tempfile

                dataset = self._dataset(dataset_id=f"ord_dataset-{name}")
                r = self._add_basic_reaction(dataset, with_yield=False)
                meas = r.outcomes[0].products[0].measurements.add()
                meas.type = reaction_pb2.ProductMeasurement.YIELD
                meas.percentage.value = 90.0
                for field, value in flags.items():
                    setattr(meas, field, value)
                with tempfile.TemporaryDirectory() as tmp:
                    path = self._write_dataset(dataset, Path(tmp))
                    report = m.AuditReport()
                    candidates = m.extract_candidates([path], report)
                    self.assertEqual(len(candidates), 1)
                    self.assertEqual(candidates[0].reported_yield["basis"], "unknown")

    def test_explicit_conversion_gives_basis_conversion(self):
        import tempfile

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset, with_yield=False)
        r.outcomes[0].conversion.value = 65.0
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 1)
            self.assertEqual(
                candidates[0].reported_yield, {"percentage": 65.0, "basis": "conversion"}
            )

    def test_duplicate_source_record_is_rejected_on_second_occurrence(self):
        import tempfile

        dataset = self._dataset()
        self._add_basic_reaction(dataset, reaction_id="rxn-dup")
        self._add_basic_reaction(dataset, reaction_id="rxn-dup")
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 1)
            self.assertEqual(
                self.report.by_rejection_reason.get(m.RejectionReason.DUPLICATE_SOURCE_RECORD), 1
            )

    def test_missing_provenance_still_gets_ord_reference(self):
        # No DOI/patent/URL at all -- the structural ord:<dataset>:<reaction>
        # reference (minted from ids already validated present) still counts
        # as provenance, per docs/guides/reaction-evidence.md.
        import tempfile

        dataset = self._dataset()
        self._add_basic_reaction(dataset)
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 1)
            kinds = {r["kind"] for r in candidates[0].references}
            self.assertEqual(kinds, {"dataset_record"})

    def test_missing_dataset_id_rejects_every_reaction_and_stays_accounted(self):
        # A dataset-level skip must not let reactions escape records_seen, and
        # by_rejection_reason's total must still equal records_rejected.
        import tempfile

        dataset = self._dataset(dataset_id="")
        self._add_basic_reaction(dataset, reaction_id="rxn-a")
        self._add_basic_reaction(dataset, reaction_id="rxn-b")
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(self.report.records_seen, 2)
            self.assertEqual(self.report.records_rejected, 2)
            self.assertEqual(
                self.report.by_rejection_reason.get(m.RejectionReason.MISSING_DATASET_ID), 2
            )
            self.assertEqual(sum(self.report.by_rejection_reason.values()), self.report.records_rejected)

    def test_no_yield_or_condition_is_rejected(self):
        import tempfile

        dataset = self._dataset()
        self._add_basic_reaction(dataset, with_yield=False)
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(
                self.report.by_rejection_reason.get(m.RejectionReason.NO_YIELD_OR_CONDITION), 1
            )

    def test_temperature_fahrenheit_converts_to_celsius(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.conditions.temperature.setpoint.value = 212.0
        r.conditions.temperature.setpoint.units = reaction_pb2.Temperature.FAHRENHEIT
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(candidates[0].conditions["temperature_c"], {"min": 100.0, "max": 100.0})

    def test_temperature_kelvin_converts_to_celsius(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.conditions.temperature.setpoint.value = 298.15
        r.conditions.temperature.setpoint.units = reaction_pb2.Temperature.KELVIN
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(candidates[0].conditions["temperature_c"], {"min": 25.0, "max": 25.0})

    def test_temperature_fahrenheit_precision_is_scaled_not_copied(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.conditions.temperature.setpoint.value = 212.0
        r.conditions.temperature.setpoint.units = reaction_pb2.Temperature.FAHRENHEIT
        r.conditions.temperature.setpoint.precision = 9.0  # 9 F° = 5 C°
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(candidates[0].conditions["temperature_c"], {"min": 95.0, "max": 105.0})

    def test_time_minutes_converts_to_hours(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.outcomes[0].reaction_time.value = 30.0
        r.outcomes[0].reaction_time.units = reaction_pb2.Time.MINUTE
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(candidates[0].conditions["time_hours"], {"min": 0.5, "max": 0.5})

    def test_time_days_converts_to_hours(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.outcomes[0].reaction_time.value = 0.5
        r.outcomes[0].reaction_time.units = reaction_pb2.Time.DAY
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(candidates[0].conditions["time_hours"], {"min": 12.0, "max": 12.0})

    def test_temperature_value_with_unsupported_unit_is_rejected(self):
        # value present, units left at the default (UNSPECIFIED) -- must
        # reject, not silently drop the field and accept on yield alone.
        import tempfile

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)  # has a yield
        r.conditions.temperature.setpoint.value = 100.0
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(
                self.report.by_rejection_reason.get(m.RejectionReason.UNSUPPORTED_TEMPERATURE_UNIT), 1
            )

    def test_reaction_time_value_with_unsupported_unit_is_rejected(self):
        import tempfile

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)  # has a yield
        r.outcomes[0].reaction_time.value = 2.0
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(
                self.report.by_rejection_reason.get(m.RejectionReason.UNSUPPORTED_TIME_UNIT), 1
            )

    def test_negative_temperature_precision_is_rejected(self):
        import tempfile
        from ord_schema.proto import reaction_pb2

        dataset = self._dataset()
        r = self._add_basic_reaction(dataset)
        r.conditions.temperature.setpoint.value = 25.0
        r.conditions.temperature.setpoint.units = reaction_pb2.Temperature.CELSIUS
        r.conditions.temperature.setpoint.precision = -1.0
        with tempfile.TemporaryDirectory() as tmp:
            path = self._write_dataset(dataset, Path(tmp))
            candidates = m.extract_candidates([path], self.report)
            self.assertEqual(len(candidates), 0)
            self.assertEqual(
                self.report.by_rejection_reason.get(m.RejectionReason.INVALID_CONDITION_RANGE), 1
            )


@requires_ord_schema
@requires_renkin_bin
class ConverterEndToEndTests(unittest.TestCase):
    """Full pipeline: fixture -> sidecar/report/manifest, via the real renkin binary."""

    def _run(self, ord_data_dir, out_dir, templates=None):
        return subprocess.run(
            [
                sys.executable,
                str(Path(m.__file__)),
                "--ord-data", str(ord_data_dir),
                "--renkin-bin", str(RENKIN_BIN),
                "--templates", str(templates or TEMPLATES),
                "--output-sidecar", str(out_dir / "sidecar.json"),
                "--output-report", str(out_dir / "report.json"),
                "--output-manifest", str(out_dir / "manifest.json"),
            ],
            capture_output=True,
            text=True,
        )

    def test_minimal_fixture_converts_and_validates(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            ord_data = tmp_path / "ord-data"
            ord_data.mkdir()
            (ord_data / "fixture.pbtxt").write_text(FIXTURE.read_text(encoding="utf-8"), encoding="utf-8")
            out = tmp_path / "out"
            out.mkdir()

            result = self._run(ord_data, out)
            self.assertEqual(result.returncode, 0, result.stderr)

            sidecar = json.loads((out / "sidecar.json").read_text())
            self.assertEqual(sidecar["schema_version"], 2)
            self.assertIn("rule:ester_cleavage", sidecar["templates"])
            example = sidecar["templates"]["rule:ester_cleavage"]["examples"][0]
            self.assertNotIn("warnings", example)  # no auto-generated warnings

            report = json.loads((out / "report.json").read_text())
            self.assertEqual(report["records_accepted"], 1)
            self.assertEqual(report["records_rejected"], 0)
            self.assertEqual(
                report["records_accepted"] + report["records_rejected"] + report["records_audit_only_excluded"],
                report["records_seen"],
            )

            manifest = json.loads((out / "manifest.json").read_text())
            self.assertEqual(manifest["output_sidecar_sha256"], m.sha256_file(out / "sidecar.json"))
            self.assertEqual(manifest["output_report_sha256"], m.sha256_file(out / "report.json"))
            self.assertEqual(manifest["renkin_binary_sha256"], m.sha256_file(RENKIN_BIN))
            self.assertEqual(manifest["templates_sha256"], m.sha256_file(TEMPLATES))
            self.assertEqual(manifest["generated_artifact_license"], "CC-BY-SA-4.0")
            # input_sha256 keys are relative to --ord-data, not absolute --
            # so a manifest stays comparable across differently-located checkouts.
            for input_path, digest in manifest["input_sha256"].items():
                self.assertFalse(Path(input_path).is_absolute())
                self.assertEqual(digest, m.sha256_file(ord_data / input_path))

            # renkin's own loader must accept the generated sidecar.
            validate = subprocess.run(
                [str(RENKIN_BIN), "evidence", "validate-sidecar", "--metadata", str(out / "sidecar.json")],
                capture_output=True,
                text=True,
            )
            self.assertEqual(validate.returncode, 0, validate.stderr)

    def test_two_runs_are_byte_identical_except_manifest_timestamp(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            ord_data = tmp_path / "ord-data"
            ord_data.mkdir()
            (ord_data / "fixture.pbtxt").write_text(FIXTURE.read_text(encoding="utf-8"), encoding="utf-8")

            out1, out2 = tmp_path / "out1", tmp_path / "out2"
            out1.mkdir()
            out2.mkdir()
            r1 = self._run(ord_data, out1)
            r2 = self._run(ord_data, out2)
            self.assertEqual(r1.returncode, 0, r1.stderr)
            self.assertEqual(r2.returncode, 0, r2.stderr)

            self.assertEqual((out1 / "sidecar.json").read_bytes(), (out2 / "sidecar.json").read_bytes())
            self.assertEqual((out1 / "report.json").read_bytes(), (out2 / "report.json").read_bytes())

            manifest1 = json.loads((out1 / "manifest.json").read_text())
            manifest2 = json.loads((out2 / "manifest.json").read_text())
            # The manifest documents its own excluded fields -- use that list
            # rather than hardcoding it a second time here.
            self.assertEqual(manifest1["reproducibility_excluded_fields"], ["cli_invocation", "generated_at"])
            for key in manifest1["reproducibility_excluded_fields"]:
                manifest1.pop(key, None)
                manifest2.pop(key, None)
            self.assertEqual(manifest1, manifest2)

    def test_changing_templates_file_changes_templates_sha256(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            ord_data = tmp_path / "ord-data"
            ord_data.mkdir()
            (ord_data / "fixture.pbtxt").write_text(FIXTURE.read_text(encoding="utf-8"), encoding="utf-8")

            alt_templates = tmp_path / "alt_templates.smi"
            alt_templates.write_text("[C:1][OH]>>[C:1]Cl\tdummy_alt_rule\n", encoding="utf-8")

            out1, out2 = tmp_path / "out1", tmp_path / "out2"
            out1.mkdir()
            out2.mkdir()
            r1 = self._run(ord_data, out1, templates=TEMPLATES)
            r2 = self._run(ord_data, out2, templates=alt_templates)
            self.assertEqual(r1.returncode, 0, r1.stderr)
            self.assertEqual(r2.returncode, 0, r2.stderr)

            manifest1 = json.loads((out1 / "manifest.json").read_text())
            manifest2 = json.loads((out2 / "manifest.json").read_text())
            self.assertNotEqual(manifest1["templates_sha256"], manifest2["templates_sha256"])
            self.assertEqual(manifest1["templates_sha256"], m.sha256_file(TEMPLATES))
            self.assertEqual(manifest2["templates_sha256"], m.sha256_file(alt_templates))

    def test_missing_dataset_id_placeholder_never_appears_in_source_dataset_ids(self):
        import tempfile

        with tempfile.TemporaryDirectory() as tmp:
            tmp_path = Path(tmp)
            ord_data = tmp_path / "ord-data"
            ord_data.mkdir()
            # One valid (dataset_id set) + one dataset_id-less file.
            (ord_data / "fixture.pbtxt").write_text(FIXTURE.read_text(encoding="utf-8"), encoding="utf-8")
            no_id_fixture = FIXTURE.read_text(encoding="utf-8").replace(
                'dataset_id: "ord_dataset-0000000000000000000000000000000000000000000000000000fixture"\n',
                "",
            )
            (ord_data / "no_id.pbtxt").write_text(no_id_fixture, encoding="utf-8")
            out = tmp_path / "out"
            out.mkdir()

            result = self._run(ord_data, out)
            self.assertEqual(result.returncode, 0, result.stderr)

            report = json.loads((out / "report.json").read_text())
            self.assertEqual(
                report["by_rejection_reason"].get("missing_dataset_id"), 1
            )
            self.assertEqual(
                report["records_accepted"] + report["records_rejected"] + report["records_audit_only_excluded"],
                report["records_seen"],
            )

            manifest = json.loads((out / "manifest.json").read_text())
            for dataset_id in manifest["source_dataset_ids"]:
                self.assertNotIn("missing", dataset_id.lower())
                self.assertNotIn("<", dataset_id)


if __name__ == "__main__":
    unittest.main()
