import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))

import compare_schema as schema  # noqa: E402

COMMERCIAL_NAME_DENYLIST = ["scifinder", "reaxys", "chemdraw", "cas registry"]

# Source files that define/serialize the schema -- the grep-deny-list check
# scans these, not the whole repo (comments/changelogs elsewhere may
# legitimately discuss why commercial tools are excluded).
SCHEMA_SOURCE_FILES = [
    os.path.join(os.path.dirname(__file__), "..", "compare_schema.py"),
    os.path.join(os.path.dirname(__file__), "..", "compare_route_graph.py"),
    os.path.join(os.path.dirname(__file__), "..", "compare_validation.py"),
]


class TestToolEnumClosed(unittest.TestCase):
    def test_exact_set_equality_not_merely_contains(self):
        # A test that only checked "renkin" and "aizynthfinder" are present
        # would still pass if "scifinder" were added alongside them.
        # Equality closes that gap.
        self.assertEqual(schema.VALID_TOOLS, frozenset({"renkin", "aizynthfinder"}))

    def test_deserialization_rejects_commercial_tool_names(self):
        for bad_tool in ("scifinder", "reaxys", "chemdraw"):
            with self.assertRaises(schema.SchemaValidationError):
                schema.PlannerComparisonRow(
                    target_id="t1",
                    target_smiles="CCO",
                    sample_rank=0,
                    tool=bad_tool,
                    tool_version="1.0",
                    configuration_id="cfg1",
                    comparison_mode="native",
                    run_status="completed",
                    route_found=False,
                )

    def test_source_files_never_mention_commercial_tool_names(self):
        for path in SCHEMA_SOURCE_FILES:
            path = os.path.normpath(path)
            with open(path, "r", encoding="utf-8") as f:
                text = f.read().lower()
            for name in COMMERCIAL_NAME_DENYLIST:
                self.assertNotIn(
                    name,
                    text,
                    msg=f"commercial tool name {name!r} must not appear in {path}",
                )

    def test_valid_tools_accepted(self):
        for tool in schema.VALID_TOOLS:
            row = schema.PlannerComparisonRow(
                target_id="t1",
                target_smiles="CCO",
                sample_rank=0,
                tool=tool,
                tool_version="1.0",
                configuration_id="cfg1",
                comparison_mode="native",
                run_status="completed",
                route_found=False,
            )
            self.assertEqual(row.tool, tool)


class TestComparisonModeAndRunStatus(unittest.TestCase):
    def test_invalid_comparison_mode_rejected(self):
        with self.assertRaises(schema.SchemaValidationError):
            schema.PlannerComparisonRow(
                target_id="t1",
                target_smiles="CCO",
                sample_rank=0,
                tool="renkin",
                tool_version="1.0",
                configuration_id="cfg1",
                comparison_mode="unified_templates",  # explicitly disallowed mode
                run_status="completed",
            )

    def test_invalid_run_status_rejected(self):
        with self.assertRaises(schema.SchemaValidationError):
            schema.PlannerComparisonRow(
                target_id="t1",
                target_smiles="CCO",
                sample_rank=0,
                tool="renkin",
                tool_version="1.0",
                configuration_id="cfg1",
                comparison_mode="native",
                run_status="partially_solved",
            )


class TestNullabilityContract(unittest.TestCase):
    def test_setup_error_forbids_timing_fields(self):
        row = schema.PlannerComparisonRow(
            target_id="t1",
            target_smiles="CCO",
            sample_rank=0,
            tool="renkin",
            tool_version="1.0",
            configuration_id="cfg1",
            comparison_mode="native",
            run_status="setup_error",
            total_elapsed_ms=123.0,
        )
        problems = schema.validate_row_nullability(row)
        self.assertTrue(any("total_elapsed_ms" in p for p in problems))

    def test_completed_requires_route_found_set(self):
        row = schema.PlannerComparisonRow(
            target_id="t1",
            target_smiles="CCO",
            sample_rank=0,
            tool="renkin",
            tool_version="1.0",
            configuration_id="cfg1",
            comparison_mode="native",
            run_status="completed",
        )
        problems = schema.validate_row_nullability(row)
        self.assertTrue(any("route_found" in p for p in problems))

    def test_clean_completed_no_route_row_has_no_violations(self):
        row = schema.PlannerComparisonRow(
            target_id="t1",
            target_smiles="CCO",
            sample_rank=0,
            tool="renkin",
            tool_version="1.0",
            configuration_id="cfg1",
            comparison_mode="native",
            run_status="completed",
            route_found=False,
            total_elapsed_ms=42.0,
            peak_rss_bytes=1024,
            raw_output_sha256="abc123",
        )
        self.assertEqual(schema.validate_row_nullability(row), [])

    def test_route_dependent_fields_forbidden_when_no_route(self):
        row = schema.PlannerComparisonRow(
            target_id="t1",
            target_smiles="CCO",
            sample_rank=0,
            tool="renkin",
            tool_version="1.0",
            configuration_id="cfg1",
            comparison_mode="native",
            run_status="completed",
            route_found=False,
            best_route_depth=3,
        )
        problems = schema.validate_row_nullability(row)
        self.assertTrue(any("best_route_depth" in p for p in problems))


class TestToolSpecificNamespacing(unittest.TestCase):
    def test_tool_specific_must_be_namespaced_under_tool_name(self):
        with self.assertRaises(schema.SchemaValidationError):
            schema.PlannerComparisonRow(
                target_id="t1",
                target_smiles="CCO",
                sample_rank=0,
                tool="renkin",
                tool_version="1.0",
                configuration_id="cfg1",
                comparison_mode="native",
                run_status="completed",
                route_found=False,
                tool_specific={"nodes_expanded": 10},  # not namespaced under "renkin"
            )

    def test_correctly_namespaced_tool_specific_accepted(self):
        row = schema.PlannerComparisonRow(
            target_id="t1",
            target_smiles="CCO",
            sample_rank=0,
            tool="renkin",
            tool_version="1.0",
            configuration_id="cfg1",
            comparison_mode="native",
            run_status="completed",
            route_found=False,
            tool_specific={"renkin": {"nodes_expanded": 10}},
        )
        self.assertEqual(row.tool_specific["renkin"]["nodes_expanded"], 10)


class TestRoundTrip(unittest.TestCase):
    def test_to_json_line_round_trips_via_load_rows(self):
        row = schema.PlannerComparisonRow(
            target_id="t1",
            target_smiles="CCO",
            sample_rank=0,
            tool="renkin",
            tool_version="1.0",
            configuration_id="cfg1",
            comparison_mode="native",
            run_status="completed",
            route_found=True,
            tool_reported_route_count=1,
            total_elapsed_ms=10.5,
        )
        import tempfile

        with tempfile.NamedTemporaryFile(mode="w", suffix=".jsonl", delete=False) as f:
            f.write(row.to_json_line() + "\n")
            path = f.name
        self.addCleanup(os.unlink, path)

        loaded = schema.load_rows(path)
        self.assertEqual(len(loaded), 1)
        self.assertEqual(loaded[0].target_id, "t1")
        self.assertEqual(loaded[0].tool_reported_route_count, 1)


if __name__ == "__main__":
    unittest.main()
