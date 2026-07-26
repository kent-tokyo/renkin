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


@requires_ord_schema
@requires_renkin_bin
class ConverterEndToEndTests(unittest.TestCase):
    """Full pipeline: fixture -> sidecar/report/manifest, via the real renkin binary."""

    def _run(self, ord_data_dir, out_dir):
        return subprocess.run(
            [
                sys.executable,
                str(Path(m.__file__)),
                "--ord-data", str(ord_data_dir),
                "--renkin-bin", str(RENKIN_BIN),
                "--templates", str(TEMPLATES),
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
            for input_path, digest in manifest["input_sha256"].items():
                self.assertEqual(digest, m.sha256_file(Path(input_path)))

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
            for key in ("generated_at", "cli_invocation"):
                manifest1.pop(key, None)
                manifest2.pop(key, None)
            self.assertEqual(manifest1, manifest2)


if __name__ == "__main__":
    unittest.main()
